//! Private Owner User bridge for product-managed official CLI copies.

use super::*;
use crate::process::{DurableStopConfirmation, NativeBinding, PrepareRequest,
    ProcessLaunch, StopBudgets};
use crate::store::atomic::Parser;
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::time::Duration;

const SCHEMA: &str = "gogoke.37.managed-cli.v1";
fn key(name:&str)->JsonString {JsonString::from_str(name)}
fn field(fields:&BTreeMap<JsonString,Json>,name:&'static str)->Result<String>{
    match fields.get(&key(name)) {
        Some(Json::String(value))=>value.to_well_formed_string()
            .filter(|value|!value.is_empty()&&!value.contains('\0'))
            .ok_or(OrchestrationError::Invalid(name)),
        _=>Err(OrchestrationError::Invalid(name)),
    }
}
fn failure(context:&'static str,error:impl std::fmt::Debug)->OrchestrationError{
    OrchestrationError::V37StoreFailure(format!("managed CLI {context}: {error:?}"))
}
pub(super) fn is_user_managed_cli_frame(frame:&[u8])->bool{
    let Ok(text)=std::str::from_utf8(frame) else{return false};
    let Ok(Json::Object(fields))=Parser::parse(text) else{return false};
    matches!(fields.get(&key("schema")),Some(Json::String(value))
        if value.to_well_formed_string().as_deref()==Some(SCHEMA))
}
fn result(command:&str,fields:impl IntoIterator<Item=(&'static str,Json)>)->Vec<u8>{
    let mut body=BTreeMap::from([
        (key("schema"),Json::String(JsonString::from_str(SCHEMA))),
        (key("command"),Json::String(JsonString::from_str(command))),
    ]);
    for (name,value) in fields {body.insert(key(name),value);}
    Json::Object(body).canonical().into_bytes()
}
fn string(value:&str)->Json{Json::String(JsonString::from_str(value))}
fn optional(value:Option<&str>)->Json{value.map_or(Json::Null,string)}
fn action_hex(bytes:&[u8])->String {
    const DIGITS:&[u8;16]=b"0123456789abcdef";
    let mut out=String::with_capacity(bytes.len()*2);
    for byte in bytes {out.push(DIGITS[(byte>>4) as usize] as char);out.push(DIGITS[(byte&15) as usize] as char);}
    out
}
fn action_bytes(hex:&str)->Result<Vec<u8>> {
    if hex.is_empty() || hex.len()>2_097_152 || hex.len()%2!=0 {
        return Err(OrchestrationError::Invalid("version action receipt bytes"));
    }
    hex.as_bytes().chunks_exact(2).map(|pair| {
        let high=hex_nibble(pair[0]).ok_or(OrchestrationError::Invalid("version action receipt hex"))?;
        let low=hex_nibble(pair[1]).ok_or(OrchestrationError::Invalid("version action receipt hex"))?;
        Ok((high<<4)|low)
    }).collect()
}
fn original_probe_response(action:&instance::ActionRecord)->Result<Vec<u8>> {
    let original=instance::public_probe_receipt(action)
        .map_err(|error|failure("probe journal witness",error))?;
    let expected=bound_step_response("probe",&action.driver,&action.target_version,
        &action.request_id,&action.operation,"PROBED",Some(&action.stage_name));
    if original!=expected {return Err(OrchestrationError::AccessDenied)}
    Ok(original)
}
fn action_failure_hex(driver:&str,state:&str,raw:&str)->String {
    action_hex(&result("failure",[("driverId",string(driver)),("state",string(state)),("raw",string(raw))]))
}
fn bound_step_response(command:&str,driver:&str,target:&str,request:&str,
    operation:&str,state:&str,stage:Option<&str>)->Vec<u8> {
    let mut fields=vec![("driverId",string(driver)),("state",string(state)),
        ("requestId",string(request)),("operation",string(operation)),("targetVersion",string(target))];
    if let Some(stage)=stage {fields.push(("stageName",string(stage)));}
    result(command,fields)
}
fn positive_revision(fields:&BTreeMap<JsonString,Json>)->Result<i64>{
    let text=field(fields,"expectedRevision")?;
    let value=text.parse::<i64>().ok().filter(|value|*value>0)
        .ok_or(OrchestrationError::Invalid("managed CLI revision"))?;
    if value.to_string()!=text{return Err(OrchestrationError::Invalid("managed CLI revision"))}
    Ok(value)
}
fn request_id(fields:&BTreeMap<JsonString,Json>)->Result<String>{
    let value=field(fields,"requestId")?;
    if !instance::valid_managed_switch_request_id(&value){
        return Err(OrchestrationError::Invalid("managed CLI request"));
    }
    Ok(value)
}
fn switch_response(command:&str,driver:&str,receipts:Vec<instance::ManagedSwitchReceipt>,replayed:bool)
    ->Result<Vec<u8>> {
    let count=receipts.len();
    let originals=receipts.into_iter().map(|receipt|
        Parser::parse(&receipt.receipt_json).map_err(OrchestrationError::from))
        .collect::<Result<Vec<_>>>()?;
    Ok(result(command,[("driverId",string(driver)),
        ("status",string(if replayed{"REPLAYED"}else{"APPLIED"})),
        ("migrated",Json::Number(count.to_string())),("receipts",Json::Array(originals))]))
}
fn receipt_set_digest(receipts:&[instance::ManagedSwitchReceipt])->Result<String> {
    let originals=receipts.iter().map(|receipt|Parser::parse(&receipt.receipt_json)
        .map_err(OrchestrationError::from)).collect::<Result<Vec<_>>>()?;
    Ok(crate::store::digest::sha256_hex(Json::Array(originals).canonical().as_bytes()))
}

fn require_first_managed_copy(db:&VerifiedDatabaseConnection<'_>,driver:&str)->Result<()> {
    let row=Statement::prepare(db.as_ptr(),
        "SELECT previous_version IS NULL FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    row.bind_text(1,driver)?;
    if !row.step_row()? || row.column_text(0)?!="1" || row.step_row()? {
        return Err(OrchestrationError::AccessDenied);
    }
    Ok(())
}

impl<'a> ProductDatabase<'a> {
    fn version_action_transaction<T>(&mut self,operation:impl FnOnce(&mut Self)->Result<T>)->Result<T> {
        self.connection.execute("BEGIN IMMEDIATE").map_err(|error|failure("action begin transaction",error))?;
        let outcome=authority::check_owner_in_current_transaction(&self.connection,&self.owner)
            .map_err(|error|failure("action transaction owner",error)).and_then(|()|operation(self));
        match outcome {
            Ok(value)=>{
                self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;
                Ok(value)
            },
            Err(primary)=>{
                if let Err(rollback)=self.connection.execute("ROLLBACK") {
                    return Err(OrchestrationError::V37StoreFailure(format!("{primary:?}; action rollback: {rollback:?}")));
                }
                Err(primary)
            },
        }
    }

    /// In-memory liveness stays strict. Only the F-produced view can match a
    /// particular durable historical row; an instance-wide exception is never
    /// passed into the resource checks below.
    fn managed_version_action_resources(&self,driver:&str,action:Option<&instance::ActionRecord>)
        ->Result<instance::QualifiedVersionAction> {
        if !self.pending_native_launches.is_empty() || !self.pending_credential_preparations.is_empty() {
            return Err(OrchestrationError::AccessDenied);
        }
        let scope=if let Some(action)=action {
            instance::revalidate_version_action(&self.connection,self.root,driver,action)
        } else {
            instance::observe_version_scope(&self.connection,self.root,driver)
        }.map_err(|error|failure("version action qualification",error))?;
        self.check_managed_version_scope(driver,scope)
    }

    fn managed_version_action_resources_at_fingerprints(&self,driver:&str,action:&instance::ActionRecord,
        copy:&str,source:&str)->Result<instance::QualifiedVersionAction> {
        let scope=instance::revalidate_version_action_at_fingerprints(&self.connection,self.root,driver,
            action,copy,source).map_err(|error|failure("version action post-write qualification",error))?;
        self.check_managed_version_scope(driver,scope)
    }

    fn managed_version_action_resources_with_probe(&self,driver:&str,
        action:&instance::ActionRecord,proof:&crate::process::NativeStopProof)
        ->Result<instance::QualifiedVersionAction> {
        let scope=instance::revalidate_version_action_with_probe(&self.connection,self.root,
            driver,action,proof).map_err(|error|failure("version action original probe",error))?;
        self.check_managed_version_scope(driver,scope)
    }

    fn managed_version_action_resources_at_fingerprints_with_probe(&self,driver:&str,
        action:&instance::ActionRecord,copy:&str,source:&str,
        proof:&crate::process::NativeStopProof)->Result<instance::QualifiedVersionAction> {
        let scope=instance::revalidate_version_action_at_fingerprints_with_probe(
            &self.connection,self.root,driver,action,copy,source,proof)
            .map_err(|error|failure("version action original probe post-write",error))?;
        self.check_managed_version_scope(driver,scope)
    }

    fn check_managed_version_scope(&self,driver:&str,scope:instance::QualifiedVersionAction)
        ->Result<instance::QualifiedVersionAction> {
        if !self.pending_native_launches.is_empty() || !self.pending_credential_preparations.is_empty() {
            return Err(OrchestrationError::AccessDenied);
        }
        let rows=Statement::prepare(self.connection.as_ptr(),
            "SELECT instance_id FROM main.gogoke_v37_instances WHERE driver_id=?1 ORDER BY instance_id")?;
        rows.bind_text(1,driver)?;
        let mut ids=Vec::new();
        while rows.step_row()? {ids.push(rows.column_text(0)?);}
        drop(rows);
        for id in ids {
            if self.owner_login.as_ref().is_some_and(|session|v37_login::pending_login_for_instance(session,&id)) ||
                self.native_sessions.values().any(|session|session.evidence.instance_id()==id) {
                return Err(OrchestrationError::AccessDenied);
            }
            let custody=Statement::prepare(self.connection.as_ptr(),
                "SELECT operation_id FROM main.gogoke_coordination_process_custody
                 WHERE domain_id='global' AND profile_id=?1 AND
                   (state IN ('PREPARED','ACTIVE') OR
                    (state='STOPPED' AND (stop_proof_hash IS NULL OR stop_proof_hash='')))")?;
            custody.bind_text(1,&id)?;
            while custody.step_row()? {
                if !scope.allows_custody(&custody.column_text(0)?) {return Err(OrchestrationError::AccessDenied);}
            }
            drop(custody);
            self.qualified_managed_source_resources_in_action(&id,driver,Some(&scope))?;
        }
        scope.recheck_native().map_err(|error|failure("version action final observation",error))?;
        Ok(scope)
    }

    fn bound_version_action(&self,driver:&str,target:&str,request:&str)->Result<instance::ActionRecord> {
        let row=instance::read_version_action(&self.connection,self.root,request)
            .map_err(|error|failure("original version action",error))?
            .ok_or(OrchestrationError::AccessDenied)?;
        if row.driver!=driver || row.target_version!=target || row.phase==instance::ActionPhase::Failed {
            return Err(OrchestrationError::AccessDenied);
        }
        Ok(row)
    }

    fn pending_managed_version_action(&self,driver:&str)->Result<Json> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_id FROM main.gogoke_v37_instance_version_actions WHERE open_driver_id=?1")?;
        q.bind_text(1,driver)?;
        if !q.step_row()? {return Ok(Json::Null);}
        let request=q.column_text(0)?;
        if q.step_row()? {return Err(OrchestrationError::AccessDenied);}
        drop(q);
        let row=instance::read_version_action(&self.connection,self.root,&request)
            .map_err(|error|failure("pending original action readback",error))?.ok_or(OrchestrationError::AccessDenied)?;
        if row.driver!=driver || row.open_driver_id!=driver {return Err(OrchestrationError::AccessDenied);}
        let phase=match row.phase {
            instance::ActionPhase::Begun=>"BEGUN",instance::ActionPhase::Staged=>"STAGED",
            instance::ActionPhase::Probing=>"PROBING",instance::ActionPhase::Probed=>"PROBED",
            _=>return Err(OrchestrationError::AccessDenied),
        };
        // Read-only original identity, never a release or cold authorization.
        Ok(Json::Object(BTreeMap::from([(key("requestId"),string(&row.request_id)),
            (key("operation"),string(&row.operation)),(key("targetVersion"),string(&row.target_version)),
            (key("phase"),string(phase))])))
    }

    fn failed_managed_version_action(&self,driver:&str)->Result<Json> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_id FROM main.gogoke_v37_instance_version_actions WHERE driver_id=?1 AND phase='FAILED' ORDER BY rowid DESC LIMIT 1")?;
        q.bind_text(1,driver)?;
        if !q.step_row()? {return Ok(Json::Null);}
        let request=q.column_text(0)?;drop(q);
        let row=instance::read_version_action(&self.connection,self.root,&request)
            .map_err(|error|failure("original known failure readback",error))?.ok_or(OrchestrationError::AccessDenied)?;
        if row.driver!=driver || row.phase!=instance::ActionPhase::Failed ||
            !row.open_driver_id.is_empty() || row.original_error_hex.is_empty() {
            return Err(OrchestrationError::AccessDenied);
        }
        Ok(Json::Object(BTreeMap::from([(key("requestId"),string(&row.request_id)),
            (key("operation"),string(&row.operation)),(key("targetVersion"),string(&row.target_version)),
            (key("phase"),string("FAILED"))])))
    }

    fn begin_managed_version_action(&mut self,driver:&str,target:&str,request:&str,
        operation:&str,frame:&[u8])->Result<Vec<u8>> {
        let operation_kind=match operation {
            "upgrade"=>instance::ActionOperation::Upgrade,
            "rollback"=>instance::ActionOperation::Rollback,
            _=>return Err(OrchestrationError::Invalid("version action operation")),
        };
        let raw=action_hex(frame);
        self.version_action_transaction(|this| {
            if let Some(row)=instance::read_version_action(&this.connection,this.root,request)
                .map_err(|error|failure("begin original action",error))? {
                if row.driver!=driver || row.target_version!=target || row.operation!=operation || row.begin_hex!=raw ||
                    row.phase==instance::ActionPhase::Failed {
                    return Err(OrchestrationError::AccessDenied);
                }
                this.managed_version_action_resources(driver,Some(&row))?;
                return action_bytes(&row.begin_receipt_hex);
            }
            let before=instance::managed_version_fingerprints(&this.connection,this.root,driver)
                .map_err(|error|failure("begin copy/source fingerprint",error))?;
            let scope=this.managed_version_action_resources(driver,None)?;
            let receipt=bound_step_response("begin",driver,target,request,operation,"UPGRADING",None);
            let receipt_hex=action_hex(&receipt);
            let begin=instance::ActionBegin {
                request_id:request,driver,operation:operation_kind,target_version:target,
                begin_hex:&raw,begin_receipt_hex:&receipt_hex,
                mode:if scope.is_cold(){instance::ActionMode::Cold}else{instance::ActionMode::Strict},
                proof_family:scope.proof_family(),source_hex:scope.source_hex(),source_digest:scope.source_digest(),
                pairs_hex:scope.pairs_hex(),pairs_digest:scope.pairs_digest(),
                copy_fingerprint:&before.0,source_fingerprint:&before.1,
            };
            let row=match instance::begin_version_action_in_current_transaction(&this.connection,this.root,&this.owner,&begin)
                .map_err(|error|failure("begin action journal",error))? {
                instance::BeginOutcome::Created(row)=>row,
                instance::BeginOutcome::Replayed(_)=>return Err(OrchestrationError::AccessDenied),
            };
            instance::record_managed_cli_progress_for_version_action_in_current_transaction(
                &this.connection,this.root,&this.owner,driver,target,"UPGRADING",0,&row)
                .map_err(|error|failure("bound begin progress",error))?;
            let after=instance::managed_version_fingerprints(&this.connection,this.root,driver)
                .map_err(|error|failure("begin post-effect fingerprint",error))?;
            this.managed_version_action_resources_at_fingerprints(driver,&row,&after.0,&after.1)?;
            instance::record_begin_version_action_effect_in_current_transaction(&this.connection,this.root,&this.owner,
                request,row.revision,&before.0,&before.1,&after.0,&after.1)
                .map_err(|error|failure("begin effect chain",error))?;
            Ok(receipt)
        })
    }

    fn stage_managed_version_action(&mut self,driver:&str,target:&str,stage:&str,
        request:&str,frame:&[u8])->Result<Vec<u8>> {
        let raw=action_hex(frame);
        self.version_action_transaction(|this| {
            let row=this.bound_version_action(driver,target,request)?;
            this.managed_version_action_resources(driver,Some(&row))?;
            if !row.stage_receipt_hex.is_empty() {
                if row.stage_hex!=raw || row.stage_name!=stage {return Err(OrchestrationError::AccessDenied);}
                return action_bytes(&row.stage_receipt_hex);
            }
            if row.phase!=instance::ActionPhase::Begun {return Err(OrchestrationError::AccessDenied);}
            instance::record_managed_cli_stage_for_version_action_in_current_transaction(
                &this.connection,this.root,&this.owner,driver,target,stage,&row)
                .map_err(|error|failure("bound version stage",error))?;
            let next=instance::managed_version_fingerprints(&this.connection,this.root,driver)
                .map_err(|error|failure("stage post-effect fingerprint",error))?;
            this.managed_version_action_resources_at_fingerprints(driver,&row,&next.0,&next.1)?;
            let receipt=bound_step_response("stage",driver,target,request,&row.operation,"STAGED",Some(stage));
            let receipt_hex=action_hex(&receipt);
            instance::advance_version_action_in_current_transaction(&this.connection,this.root,&this.owner,
                request,row.revision,&instance::ActionStep {
                    next_phase:instance::ActionPhase::Staged,raw_hex:&raw,receipt_hex:&receipt_hex,stage_name:stage,
                    expected_copy_fingerprint:&row.current_copy_fingerprint,
                    expected_source_fingerprint:&row.current_source_fingerprint,
                    next_copy_fingerprint:&next.0,next_source_fingerprint:&next.1,
                }).map_err(|error|failure("stage action receipt",error))?;
            Ok(receipt)
        })
    }

    fn probe_managed_version_action(&mut self,driver:&str,target:&str,stage:&str,
        request:&str,frame:&[u8])->Result<Vec<u8>> {
        let raw=action_hex(frame);
        let (intent,replayed)=self.version_action_transaction(|this| {
            let row=this.bound_version_action(driver,target,request)?;
            this.managed_version_action_resources(driver,Some(&row))?;
            if row.stage_name!=stage {return Err(OrchestrationError::AccessDenied);}
            if row.phase==instance::ActionPhase::Probed || row.phase==instance::ActionPhase::Consumed {
                if row.probe_hex!=raw {return Err(OrchestrationError::AccessDenied);}
                return Ok((None,original_probe_response(&row)?));
            }
            // An existing PROBING row is an unresolved original effect. Reading
            // that intent never grants permission for a second H launch.
            if row.phase!=instance::ActionPhase::Staged {return Err(OrchestrationError::AccessDenied);}
            let outcome=instance::begin_probe_version_action_in_current_transaction(&this.connection,this.root,&this.owner,
                request,row.revision,&raw,stage,&row.current_copy_fingerprint,&row.current_source_fingerprint)
                .map_err(|error|failure("probe durable intent",error))?;
            match outcome {
                instance::BeginOutcome::Created(intent)=>Ok((Some(intent),Vec::new())),
                instance::BeginOutcome::Replayed(_)=>Err(OrchestrationError::AccessDenied),
            }
        })?;
        // Reached only after a confirmed intent COMMIT. COMMIT unknown returns
        // above; a later caller can only read the existing unresolved intent.
        let Some(intent)=intent else{return Ok(replayed)};
        self.probe_staged_managed_cli_for_version(driver,target,stage,request,true,Some(&intent))?;
        let row=self.bound_version_action(driver,target,request)?;
        self.managed_version_action_resources(driver,Some(&row))?;
        if row.phase!=instance::ActionPhase::Probed || row.probe_hex!=raw {
            return Err(OrchestrationError::AccessDenied);
        }
        original_probe_response(&row)
    }

    fn fail_managed_version_action(&mut self,driver:&str,state:&str,raw:&str,
        action:&instance::ActionRecord)->Result<()> {
        let original_hex=action_failure_hex(driver,state,raw);
        self.version_action_transaction(|this| {
            instance::record_managed_cli_failure_for_version_action_in_current_transaction(
                &this.connection,this.root,&this.owner,driver,state,raw,action)
                .map_err(|error|failure("bound original failure copy",error))?;
            let next=instance::managed_version_fingerprints(&this.connection,this.root,driver)
                .map_err(|error|failure("bound failure post-effect fingerprints",error))?;
            let observed=instance::record_version_action_error_in_current_transaction(&this.connection,this.root,&this.owner,
                &action.request_id,action.revision,&original_hex,
                &action.current_copy_fingerprint,&action.current_source_fingerprint,&next.0,&next.1)
                .map_err(|error|failure("bound original failure journal",error))?;
            if state!="PROBE_UNKNOWN" {
                instance::fail_version_action_in_current_transaction(&this.connection,this.root,&this.owner,
                    &action.request_id,observed.revision,&original_hex)
                    .map_err(|error|failure("bound known failure",error))?;
            }
            Ok(())
        })
    }

    fn require_first_managed_copy(&self,driver:&str)->Result<()> {
        require_first_managed_copy(&self.connection,driver)?;
        instance::require_default_first_copy(&self.connection,driver)
            .map_err(|error|failure("default first copy",error))
    }

    fn strict_managed_version_resources(&mut self,driver:&str)->Result<()> {
        let action=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instance_version_actions WHERE open_driver_id=?1 LIMIT 1")?;
        action.bind_text(1,driver)?;
        if action.step_row()? {return Err(OrchestrationError::AccessDenied);}
        drop(action);
        if !self.pending_native_launches.is_empty() ||
            !self.pending_credential_preparations.is_empty() {
            return Err(OrchestrationError::AccessDenied);
        }
        self.no_live_global_cli_owner(driver)?;
        instance::no_unsettled_managed_version_change(&self.connection,driver)
            .map_err(|error|failure("version quiescence",error))?;
        let rows=Statement::prepare(self.connection.as_ptr(),
            "SELECT i.instance_id FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) ORDER BY i.instance_id")?;
        rows.bind_text(1,driver)?;
        let mut ids=Vec::new();
        while rows.step_row()?{ids.push(rows.column_text(0)?);}
        drop(rows);
        for id in ids {self.qualified_managed_source_resources(&id,driver)?;}
        Ok(())
    }

    fn switch_managed_cli(&mut self,command:&str,driver:&str,target:&str,revision:i64,
        request:&str,frame:&[u8])->Result<Vec<u8>> {
        if instance::read_version_action(&self.connection,self.root,request)
            .map_err(|error|failure("switch bound action lookup",error))?.is_some() {
            return self.switch_managed_version_action(command,driver,target,revision,request,frame);
        }
        // Read the original effect before looking at today's copy/stage/holders.
        if let Some(receipts)=instance::reconcile_managed_switch(&self.connection,self.root,
            request,driver,target,revision,frame).map_err(|error|failure("original switch receipt",error))? {
            return switch_response(command,driver,receipts,true);
        }
        self.connection.execute("BEGIN IMMEDIATE").map_err(|error|failure("switch begin",error))?;
        let applied=(||{
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)
                .map_err(|error|failure("switch owner",error))?;
            self.strict_managed_version_resources(driver)?;
            let copy=instance::read_managed_cli(&self.connection,self.root,driver)
                .map_err(|error|failure("switch copy",error))?.ok_or(OrchestrationError::AccessDenied)?;
            let stage=copy.stage_name.ok_or(OrchestrationError::AccessDenied)?;
            instance::activate_quiescent_managed_version_in_current_transaction(&self.connection,
                self.root,&self.owner,driver,target,&stage,revision,request,frame)
                .map_err(|error|failure("switch source CAS",error))?;
            self.strict_managed_version_resources(driver)?;
            instance::reconcile_managed_switch(&self.connection,self.root,request,driver,target,
                revision,frame).map_err(|error|failure("committed switch receipts",error))?
                .ok_or(OrchestrationError::AccessDenied)
        })();
        match applied {
            Ok(receipts)=>{
                self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;
                switch_response(command,driver,receipts,false)
            },
            Err(primary)=>{
                if let Err(rollback)=self.connection.execute("ROLLBACK") {
                    return Err(OrchestrationError::V37StoreFailure(format!("{primary:?}; switch rollback: {rollback:?}")));
                }
                Err(primary)
            },
        }
    }

    fn switch_managed_version_action(&mut self,command:&str,driver:&str,target:&str,revision:i64,
        request:&str,frame:&[u8])->Result<Vec<u8>> {
        let raw=action_hex(frame);
        self.version_action_transaction(|this| {
            let row=this.bound_version_action(driver,target,request)?;
            if row.operation!=command {return Err(OrchestrationError::AccessDenied);}
            this.managed_version_action_resources(driver,Some(&row))?;
            let originals=instance::reconcile_managed_switch(&this.connection,this.root,request,driver,target,
                revision,frame).map_err(|error|failure("action original switch receipt",error))?;
            if row.phase==instance::ActionPhase::Consumed {
                let receipts=originals.ok_or(OrchestrationError::AccessDenied)?;
                if row.final_hex!=raw || row.final_receipt_set_digest!=receipt_set_digest(&receipts)? {
                    return Err(OrchestrationError::AccessDenied);
                }
                return switch_response(command,driver,receipts,true);
            }
            if row.phase!=instance::ActionPhase::Probed || originals.is_some() {
                return Err(OrchestrationError::AccessDenied);
            }
            let copy=instance::read_managed_cli(&this.connection,this.root,driver)
                .map_err(|error|failure("action final copy",error))?.ok_or(OrchestrationError::AccessDenied)?;
            let stage=copy.stage_name.ok_or(OrchestrationError::AccessDenied)?;
            if copy.revision!=revision || stage!=row.stage_name {return Err(OrchestrationError::AccessDenied);}
            instance::activate_managed_version_action_in_current_transaction(&this.connection,this.root,&this.owner,
                driver,target,&stage,revision,request,frame,&row)
                .map_err(|error|failure("action original F source switch",error))?;
            let next=instance::managed_version_fingerprints(&this.connection,this.root,driver)
                .map_err(|error|failure("action switch post-effect fingerprints",error))?;
            this.managed_version_action_resources_at_fingerprints(driver,&row,&next.0,&next.1)?;
            let receipts=instance::reconcile_managed_switch(&this.connection,this.root,request,driver,target,
                revision,frame).map_err(|error|failure("action effect receipts",error))?
                .ok_or(OrchestrationError::AccessDenied)?;
            let consumed=instance::consume_version_action_in_current_transaction(&this.connection,this.root,&this.owner,
                request,row.revision,&raw,&receipt_set_digest(&receipts)?,
                &row.current_copy_fingerprint,&row.current_source_fingerprint,&next.0,&next.1)
                .map_err(|error|failure("action atomic consume",error))?;
            this.managed_version_action_resources(driver,Some(&consumed))?;
            switch_response(command,driver,receipts,false)
        })
    }
    /// Check current global login/observer custody without turning historical
    /// UNKNOWN rows into a fabricated live process.
    fn no_live_global_cli_owner(&mut self,driver:&str)->Result<()> {
        let rows=Statement::prepare(self.connection.as_ptr(),
            "SELECT instance_id FROM main.gogoke_v37_instances WHERE driver_id=?1")?;
        rows.bind_text(1,driver)?;
        let mut instances=Vec::new();
        while rows.step_row()? {
            instances.push(rows.column_text(0)?);
        }
        drop(rows);
        for id in instances {
            if self.owner_login.as_ref().is_some_and(|session|
                v37_login::pending_login_for_instance(session,&id)) ||
                self.native_sessions.values().any(|session|session.evidence.instance_id()==id){
                return Err(OrchestrationError::AccessDenied);
            }
            let active=Statement::prepare(self.connection.as_ptr(),
                "SELECT 1 FROM main.gogoke_coordination_process_custody
                  WHERE domain_id='global' AND profile_id=?1 AND
                    (state IN ('PREPARED','ACTIVE') OR
                     (state='STOPPED' AND (stop_proof_hash IS NULL OR stop_proof_hash='')))
                  LIMIT 1")?;
            active.bind_text(1,&id)?;
            if active.step_row()?{return Err(OrchestrationError::AccessDenied)}
        }
        // Copy preparation is independent of retained instance resources. In
        // particular it must not run holder recovery or write the old ACL.
        instance::no_unsettled_instance_use(&self.connection,driver)
            .map_err(|error|failure("global login/observer custody",error))?;
        Ok(())
    }

    /// Select only instances whose original source can be switched in this
    /// transaction. A disabled instance is retained, unbound and unable to
    /// launch after READY. Every other old instance must qualify.
    fn qualified_managed_source_instances(&self,driver:&str)->Result<Vec<String>> {
        if !self.pending_native_launches.is_empty() ||
            !self.pending_credential_preparations.is_empty() {
            return Err(OrchestrationError::AccessDenied);
        }
        let rows=Statement::prepare(self.connection.as_ptr(),
            "SELECT i.instance_id,COALESCE(CAST(p.enabled AS TEXT),'') FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) ORDER BY i.instance_id")?;
        rows.bind_text(1,driver)?;
        let mut selected=Vec::new();
        while rows.step_row()? {
            let id=rows.column_text(0)?;
            let enabled=rows.column_text(1)?;
            if enabled=="0" {continue}
            if !matches!(enabled.as_str(),""|"1") ||
                self.owner_login.as_ref().is_some_and(|session|
                    v37_login::pending_login_for_instance(session,&id)) ||
                self.native_sessions.values().any(|session|session.evidence.instance_id()==id) {
                return Err(OrchestrationError::AccessDenied);
            }
            selected.push(id);
        }
        drop(rows);
        for id in &selected {self.qualified_managed_source_resources(id,driver)?;}
        Ok(selected)
    }

    pub(super) fn qualified_managed_source_resources(&self,id:&str,driver:&str)->Result<()> {
        self.qualified_managed_source_resources_in_action(id,driver,None)
    }

    fn qualified_managed_source_resources_in_action(&self,id:&str,driver:&str,
        scope:Option<&instance::QualifiedVersionAction>)->Result<()> {
        let first_claude_copy=driver=="claude" &&
            instance::is_first_same_pin_copy(&self.connection,driver)
                .map_err(|error|failure("first source copy scope",error))?;
        let pending=|sql:&str|->Result<bool>{
            let row=Statement::prepare(self.connection.as_ptr(),sql)?;
            row.bind_text(1,id)?;
            Ok(row.step_row()?)
        };
        if pending("SELECT 1 FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=g.old_process_operation_id WHERE e.instance_id=?1 AND g.stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1")? {
            return Err(OrchestrationError::AccessDenied);
        }
        let ops=Statement::prepare(self.connection.as_ptr(),
            "SELECT o.domain_id,o.session_id,o.request_id FROM main.gogoke_v37_h_operation o JOIN main.gogoke_v37_h_claim c ON c.domain_id=o.domain_id AND c.session_id=o.session_id WHERE c.instance_id=?1 AND o.status!='APPLIED'")?;
        ops.bind_text(1,id)?;
        let mut retained=Vec::new();
        while ops.step_row()? {retained.push((ops.column_text(0)?,ops.column_text(1)?,ops.column_text(2)?));}
        drop(ops);
        for (domain,session,request) in retained {
            if scope.is_some_and(|scope|scope.allows_operation(&domain,&session,&request)) {continue}
            if !first_claude_copy || !self.completed_claude_managed_pending_operation(id,&domain,&session,&request)? {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        let stdin=Statement::prepare(self.connection.as_ptr(),
            "SELECT j.domain_id,j.request_id,j.session_id,j.generation,j.process_operation_id,j.ticket,j.custodian_nonce FROM main.gogoke_v37_h_stdin_journal j JOIN main.gogoke_v37_h_claim c ON c.domain_id=j.domain_id AND c.session_id=j.session_id WHERE c.instance_id=?1 AND j.phase IN ('PREPARED','UNKNOWN')")?;
        stdin.bind_text(1,id)?;
        let mut retained=Vec::new();
        while stdin.step_row()? {
            retained.push((0..7).map(|n|stdin.column_text(n))
                .collect::<std::result::Result<Vec<_>,_>>()?);
        }
        drop(stdin);
        for row in retained {
            if scope.is_some_and(|scope|scope.allows_stdin(id,&row[0],&row[2],&row[3],
                &row[4],&row[1],&row[5],&row[6])) {continue}
            if !first_claude_copy || !self.completed_claude_managed_pending_stdin(id,&row[0],&row[1])? {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        let homes=Statement::prepare(self.connection.as_ptr(),
            "SELECT home_id FROM main.gogoke_v37_instance_homes WHERE instance_id=?1 AND state NOT IN ('ACTIVE','CLEANED','CLOSED')")?;
        homes.bind_text(1,id)?;
        while homes.step_row()? {
            let home=homes.column_text(0)?;
            if !scope.is_some_and(|scope|scope.allows_home(&home)) {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        drop(homes);
        for sql in [
            "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase!='APPLIED' LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_credential_objects WHERE instance_id=?1 AND phase!='ACTIVE' LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_credential_aliases WHERE instance_id=?1 AND state IN ('PREPARING','REMOVE_PENDING','UNKNOWN') LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_credential_profiles WHERE instance_id=?1 AND state IN ('GRANT_PENDING','REVOKE_PENDING','UNKNOWN') LIMIT 1",
        ] {if pending(sql)? {return Err(OrchestrationError::AccessDenied)}}

        let claims=Statement::prepare(self.connection.as_ptr(),
            "SELECT a.domain_id,a.session_id,a.state,COALESCE(a.process_operation_id,''),COALESCE(a.stop_fact_id,''),COALESCE(c.state,''),COALESCE(c.stop_proof_hash,''),a.generation,COALESCE(a.binding_id,'') FROM main.gogoke_v37_h_claim a LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=a.process_operation_id AND c.profile_id=a.instance_id AND c.domain_id=a.domain_id AND c.generation=a.generation WHERE a.instance_id=?1")?;
        claims.bind_text(1,id)?;
        let mut rows=Vec::new();
        while claims.step_row()? {rows.push((0..9).map(|n|claims.column_text(n)).collect::<std::result::Result<Vec<_>,_>>()?);}
        drop(claims);
        for row in rows {
            let (domain,session,state,operation,stop,custody,proof)=
                (&row[0],&row[1],&row[2],&row[3],&row[4],&row[5],&row[6]);
            if scope.is_some_and(|scope|scope.allows_claim(id,domain,session,&row[7],&row[8],operation)) {continue}
            if state=="RELEASED" && operation.is_empty() && stop.is_empty() {
                let q=Statement::prepare(self.connection.as_ptr(),
                    "SELECT 1 FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2 UNION ALL SELECT 1 FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='open' LIMIT 1")?;
                q.bind_text(1,domain)?;q.bind_text(2,session)?;
                if !q.step_row()? {continue}
            }
            if matches!(state.as_str(),"RELEASED"|"STOPPED") && !operation.is_empty() &&
                !stop.is_empty() && custody=="STOPPED" && proof==stop {continue}
            if first_claude_copy && state=="RELEASED" && stop.is_empty() && !operation.is_empty()
                && self.completed_claude_holder_release(id,domain,session,operation)? {continue}
            if state=="RELEASED" && stop.is_empty() && !operation.is_empty() &&
                driver=="codex" && (self.completed_codex_holder_release(id,domain,session,operation)? ||
                    self.completed_codex_failed_resume_resource_release(id,domain,session,operation)?) {
                continue;
            }
            return Err(OrchestrationError::AccessDenied);
        }

        let episodes=Statement::prepare(self.connection.as_ptr(),
            "SELECT e.domain_id,e.session_id,COALESCE(e.process_operation_id,''),e.phase,COALESCE(e.stop_fact_id,''),COALESCE(c.state,''),COALESCE(c.stop_proof_hash,''),e.request_id FROM main.gogoke_v37_h_process_episode e LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE e.instance_id=?1")?;
        episodes.bind_text(1,id)?;
        let mut rows=Vec::new();
        while episodes.step_row()? {rows.push((0..8).map(|n|episodes.column_text(n)).collect::<std::result::Result<Vec<_>,_>>()?);}
        drop(episodes);
        for row in rows {
            let (domain,session,operation,phase,stop,custody,proof)=
                (&row[0],&row[1],&row[2],&row[3],&row[4],&row[5],&row[6]);
            if scope.is_some_and(|scope|scope.allows_episode(domain,&row[7],operation)) {continue}
            if operation.is_empty() && phase=="FAILED" {continue}
            if matches!(phase.as_str(),"STOPPED"|"FAILED") && !stop.is_empty() &&
                custody=="STOPPED" && proof==stop {continue}
            if first_claude_copy && !operation.is_empty()
                && (self.completed_claude_holder_release(id,domain,session,operation)?
                    || self.completed_claude_failed_resume_resource_release(id,domain,session,operation)?) {continue}
            if !operation.is_empty() && driver=="codex" &&
                (self.completed_codex_holder_release(id,domain,session,operation)? ||
                 self.completed_codex_failed_resume_resource_release(id,domain,session,operation)?) {continue}
            return Err(OrchestrationError::AccessDenied);
        }
        Ok(())
    }

    pub(super) fn migrate_managed_cli(&mut self,driver:&str,stage:&str)->Result<usize> {
        // Explicit migration recovery, never part of staging or version probe.
        // Each old ACL effect uses the independent original durable intent.
        self.require_first_managed_copy(driver)?;
        if driver=="claude" && instance::is_first_same_pin_copy(&self.connection,driver)
            .map_err(|error|failure("first source copy recovery scope",error))? {
            self.no_live_global_cli_owner(driver)?;
            let copy=instance::read_managed_cli(&self.connection,self.root,driver)
                .map_err(|error|failure("migration recovery copy",error))?
                .ok_or(OrchestrationError::AccessDenied)?;
            if copy.state!="PROBED" || copy.stage_name.as_deref()!=Some(stage) {
                return Err(OrchestrationError::AccessDenied);
            }
            instance::inspect_staged_official_cli(self.root,driver,stage)
                .map_err(|error|failure("migration recovery exact stage",error))?;
            let instances=Statement::prepare(self.connection.as_ptr(),
                "SELECT i.instance_id FROM main.gogoke_v37_instances i
                 LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id
                 WHERE i.driver_id='claude' AND (p.tombstoned IS NULL OR p.tombstoned=0)
                   AND (p.enabled IS NULL OR p.enabled=1) ORDER BY i.instance_id")?;
            let mut ids=Vec::new();
            while instances.step_row()? {ids.push(instances.column_text(0)?);}
            drop(instances);
            for id in ids {self.recover_claude_failed_resume_for_first_copy(&id)?;}
        }
        self.connection.execute("BEGIN IMMEDIATE")
            .map_err(|error|failure("migration begin",error))?;
        let result=(||{
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)
                .map_err(|error|failure("migration owner",error))?;
            self.require_first_managed_copy(driver)?;
            self.no_live_global_cli_owner(driver)?;
            let qualified=self.qualified_managed_source_instances(driver)?;
            instance::migrate_quiescent_legacy_instances(&self.connection,self.root,&self.owner,
                driver,stage,&qualified).map_err(|error|failure("legacy migration",error))
        })();
        match result {
            Ok(count)=>{self.connection.execute("COMMIT")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(count)},
            Err(primary)=>{
                if let Err(rollback)=self.connection.execute("ROLLBACK") {
                    return Err(OrchestrationError::V37StoreFailure(format!(
                        "managed CLI migration: {primary:?}; rollback: {rollback:?}")));
                }
                Err(primary)
            },
        }
    }

    pub(super) fn dispatch_user_managed_cli(&mut self,frame:&[u8])->Result<Vec<u8>>{
        let text=std::str::from_utf8(frame).map_err(|error|failure("frame UTF-8",error))?;
        let Json::Object(fields)=Parser::parse(text)? else{
            return Err(OrchestrationError::Invalid("managed CLI frame"));
        };
        if field(&fields,"schema")?!=SCHEMA{return Err(OrchestrationError::AccessDenied)}
        let command=field(&fields,"command")?;
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        match command.as_str(){
            "migrate-legacy-codex-source" if fields.len()==11=>{
                let driver=field(&fields,"driverId")?;
                if driver!="codex" {return Err(OrchestrationError::AccessDenied)}
                let request=field(&fields,"requestId")?;
                let target=field(&fields,"targetId")?;
                let original=field(&fields,"originalRequestId")?;
                let fingerprint=field(&fields,"originalRequestFingerprint")?;
                let receipt=field(&fields,"originalNativeReceiptHex")?;
                let home=field(&fields,"originalHomeIdentity")?;
                let expected_root=field(&fields,"expectedRootIdentity")?;
                let expected_database=field(&fields,"expectedDatabaseIdentity")?;
                // This bridge owns every in-process execution source. Native
                // F also rechecks the persisted H/C family in both immediate
                // transactions before making its one SOURCE effect.
                if self.owner_login.is_some() || !self.native_sessions.is_empty() ||
                    !self.pending_native_launches.is_empty() ||
                    !self.pending_credential_preparations.is_empty() {
                    return Err(OrchestrationError::AccessDenied);
                }
                let applied=result("migrate-legacy-codex-source",[
                    ("driverId",string("codex")),("requestId",string(&request)),
                    ("targetId",string(&target)),("status",string("APPLIED"))]);
                let outcome=instance::migrate_legacy_codex_source(&mut self.connection,self.root,
                    &self.owner,&instance::LegacySourceRequest {
                        raw:frame,request_id:&request,target_id:&target,
                        original_request_id:&original,original_fingerprint:&fingerprint,
                        original_receipt_hex:&receipt,original_home_identity:&home,
                        expected_root_identity:&expected_root,
                        expected_database_identity:&expected_database,
                    },&applied).map_err(|error|failure("strict legacy SOURCE",error))?;
                Ok(outcome.receipt)
            },
            "status" if fields.len()==3=>{
                let driver=field(&fields,"driverId")?;
                let pending=self.pending_managed_version_action(&driver)?;
                let failed=self.failed_managed_version_action(&driver)?;
                let copy=instance::read_managed_cli(&self.connection,self.root,&driver)
                    .map_err(|error|failure("status",error))?;
                let prepared=instance::read_prepared_managed_replacement(&self.connection,self.root,&driver)
                    .map_err(|error|failure("status source shape",error))?;
                Ok(match copy {
                    None=>result("status",[("driverId",string(&driver)),("state",string("NOT_INSTALLED")),
                        ("revision",Json::Null),("version",Json::Null),
                        ("stageName",Json::Null),("preparedReplacement",Json::Null),
                        ("progressBytes",Json::Null),("pendingAction",pending),("failedAction",failed)]),
                    Some(copy)=>result("status",[("driverId",string(&driver)),("state",string(&copy.state)),
                        ("revision",string(&copy.revision.to_string())),("version",optional(copy.version.as_deref())),
                        ("stageName",optional(copy.stage_name.as_deref())),
                        ("preparedReplacement",prepared.map_or(Json::Null,Json::Bool)),
                        ("progressBytes",Json::Number(copy.progress_bytes.to_string())),
                        ("previousVersion",optional(copy.previous_version.as_deref())),
                        ("verifiedOtherVersions",Json::Array(copy.verified_other_versions.iter().map(|version|string(version)).collect())),
                        ("officialVersion",optional(copy.official_notice.as_deref())),
                        ("checkedAt",optional(copy.checked_at.as_deref())),("raw",optional(copy.raw_error.as_deref())),
                        ("pendingAction",pending),("failedAction",failed)]),
                })
            },
            "upgrade"|"rollback" if fields.len()==6=>{
                let driver=field(&fields,"driverId")?;
                let target=field(&fields,"targetVersion")?;
                let request=request_id(&fields)?;
                self.switch_managed_cli(&command,&driver,&target,positive_revision(&fields)?,&request,frame)
            },
            "uninstall" if fields.len()==4=>{
                let driver=field(&fields,"driverId")?;
                let revision=positive_revision(&fields)?;
                self.strict_managed_version_resources(&driver)?;
                instance::uninstall_managed_cli(&mut self.connection,self.root,&self.owner,&driver,revision)
                    .map_err(|error|failure("uninstall",error))?;
                Ok(result("uninstall",[("driverId",string(&driver)),("state",string("NOT_INSTALLED"))]))
            },
            "check-updates" if fields.len()==4=>{
                let driver=field(&fields,"driverId")?;
                let notice=match fields.get(&key("officialVersion")){
                    Some(Json::Null)=>None,
                    Some(Json::String(value))=>Some(value.to_well_formed_string()
                        .ok_or(OrchestrationError::Invalid("official version notice"))?),
                    _=>return Err(OrchestrationError::Invalid("official version notice")),
                };
                let checked=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error|failure("notice clock",error))?.as_millis().to_string();
                instance::record_official_cli_notice(&mut self.connection,&self.owner,&driver,
                    notice.as_deref(),&checked)
                    .map_err(|error|failure("unverified official notice",error))?;
                Ok(result("check-updates",[("driverId",string(&driver)),("officialVersion",optional(notice.as_deref())),
                    ("checkedAt",string(&checked)),("verifiedForActivation",Json::Bool(false))]))
            },
            "resume" if fields.len()==4=>{
                let driver=field(&fields,"driverId")?;
                let request=field(&fields,"requestId")?;
                if request.len()>64 || !request.bytes().all(|byte|
                    byte.is_ascii_alphanumeric()||matches!(byte,b'-'|b'_')){
                    return Err(OrchestrationError::Invalid("managed CLI resume request"));
                }
                self.no_live_global_cli_owner(&driver)?;
                let copy=instance::read_managed_cli(&self.connection,self.root,&driver)
                    .map_err(|error|failure("resume read",error))?
                    .ok_or(OrchestrationError::AccessDenied)?;
                let stage=copy.stage_name.ok_or(OrchestrationError::AccessDenied)?;
                self.require_first_managed_copy(&driver)?;
                match copy.state.as_str() {
                    "STAGED"=>self.probe_staged_managed_cli(&driver,&stage,&request)?,
                    "PROBED"=>(),
                    _=>return Err(OrchestrationError::AccessDenied),
                }
                let migrated=self.migrate_managed_cli(&driver,&stage)?;
                Ok(result("resume",[("driverId",string(&driver)),
                    ("state",string("READY")),("migrated",Json::Number(migrated.to_string()))]))
            },
            "root" if fields.len()==2=>{
                let path=instance::managed_cli_root(self.root)
                    .map_err(|error|failure("private staging root",error))?;
                Ok(result("root",[("root",string(&path.to_string_lossy()))]))
            },
            "begin" if fields.len()==3=>{
                let driver=field(&fields,"driverId")?;
                instance::require_default_first_copy(&self.connection,&driver)
                    .map_err(|error|failure("default first copy",error))?;
                self.no_live_global_cli_owner(&driver)?;
                instance::record_managed_cli_progress(&mut self.connection,&self.owner,
                    &driver,"DOWNLOADING",0).map_err(|error|failure("begin download",error))?;
                Ok(result("begin",[("driverId",string(&driver)),
                    ("state",string("DOWNLOADING"))]))
            },
            "begin" if fields.len()==4=>{
                let driver=field(&fields,"driverId")?;
                let version=field(&fields,"targetVersion")?;
                self.strict_managed_version_resources(&driver)?;
                instance::record_managed_cli_progress_for_version(&mut self.connection,&self.owner,
                    &driver,&version,"UPGRADING",0).map_err(|error|failure("begin verified version",error))?;
                Ok(result("begin",[("driverId",string(&driver)),("state",string("UPGRADING"))]))
            },
            "begin" if fields.len()==6=>{
                let driver=field(&fields,"driverId")?;
                let target=field(&fields,"targetVersion")?;
                let request=request_id(&fields)?;
                self.begin_managed_version_action(&driver,&target,&request,&field(&fields,"operation")?,frame)
            },
            "stage" if fields.len()==4=>{
                let driver=field(&fields,"driverId")?;
                let stage=field(&fields,"stageName")?;
                instance::require_default_first_copy(&self.connection,&driver)
                    .map_err(|error|failure("default first copy",error))?;
                self.no_live_global_cli_owner(&driver)?;
                instance::record_managed_cli_stage(&mut self.connection,self.root,
                    &self.owner,&driver,&stage).map_err(|error|failure("record stage",error))?;
                Ok(result("stage",[("driverId",string(&driver)),("state",string("STAGED"))]))
            },
            "stage" if fields.len()==5=>{
                let driver=field(&fields,"driverId")?;
                let stage=field(&fields,"stageName")?;
                let version=field(&fields,"targetVersion")?;
                self.strict_managed_version_resources(&driver)?;
                instance::record_managed_cli_stage_for_version(&mut self.connection,self.root,&self.owner,
                    &driver,&version,&stage).map_err(|error|failure("record verified version stage",error))?;
                Ok(result("stage",[("driverId",string(&driver)),("state",string("STAGED"))]))
            },
            "stage" if fields.len()==6=>{
                let driver=field(&fields,"driverId")?;
                let target=field(&fields,"targetVersion")?;
                let stage=field(&fields,"stageName")?;
                let request=request_id(&fields)?;
                self.stage_managed_version_action(&driver,&target,&stage,&request,frame)
            },
            "failure" if fields.len()==6=>{
                let driver=field(&fields,"driverId")?;
                let state=field(&fields,"state")?;
                let raw=field(&fields,"raw")?;
                let request=request_id(&fields)?;
                let row=instance::read_version_action(&self.connection,self.root,&request)
                    .map_err(|error|failure("bound failure action",error))?.ok_or(OrchestrationError::AccessDenied)?;
                // This ingress reports downloader failure before stage/probe.
                // A caller's error is never evidence that a committed probe
                // intent did not launch. Only native probe observations may
                // settle later phases; exact recorded FAILED replay remains.
                if row.driver!=driver || !matches!(row.phase,instance::ActionPhase::Begun|instance::ActionPhase::Failed) {
                    return Err(OrchestrationError::AccessDenied);
                }
                if row.phase==instance::ActionPhase::Failed {
                    if row.original_error_hex!=action_failure_hex(&driver,&state,&raw) {return Err(OrchestrationError::AccessDenied);}
                } else {
                    self.fail_managed_version_action(&driver,&state,&raw,&row)?;
                }
                Ok(result("failure",[("driverId",string(&driver)),("state",string(&state))]))
            },
            "failure" if fields.len()==5=>{
                let driver=field(&fields,"driverId")?;
                let state=field(&fields,"state")?;
                let raw=field(&fields,"raw")?;
                let copy=instance::read_managed_cli(&self.connection,self.root,&driver)
                    .map_err(|error|failure("original failure state",error))?;
                if copy.is_some_and(|copy|matches!(copy.state.as_str(),"PROBE_UNKNOWN"|"UNINSTALLING")){
                    return Err(OrchestrationError::AccessDenied);
                }
                instance::record_managed_cli_failure(&mut self.connection,&driver,&state,&raw)
                    .map_err(|error|failure("failure record",error))?;
                Ok(result("failure",[("driverId",string(&driver)),("state",string(&state))]))
            },
            "probe" if fields.len()==5=>{
                let driver=field(&fields,"driverId")?;
                let stage=field(&fields,"stageName")?;
                let request=field(&fields,"requestId")?;
                if request.len()>64 || !request.bytes().all(|byte|
                    byte.is_ascii_alphanumeric()||matches!(byte,b'-'|b'_')){
                    return Err(OrchestrationError::Invalid("managed CLI probe request"));
                }
                self.no_live_global_cli_owner(&driver)?;
                self.probe_staged_managed_cli(&driver,&stage,&request)?;
                Ok(result("probe",[("driverId",string(&driver)),("state",string("PROBED"))]))
            },
            "probe" if fields.len()==6=>{
                let driver=field(&fields,"driverId")?;
                let stage=field(&fields,"stageName")?;
                let version=field(&fields,"targetVersion")?;
                let request=request_id(&fields)?;
                if instance::read_version_action(&self.connection,self.root,&request)
                    .map_err(|error|failure("probe bound action lookup",error))?.is_some() {
                    return self.probe_managed_version_action(&driver,&version,&stage,&request,frame);
                }
                self.strict_managed_version_resources(&driver)?;
                self.probe_staged_managed_cli_version(&driver,&version,&stage,&request)?;
                Ok(result("probe",[("driverId",string(&driver)),("state",string("PROBED"))]))
            },
            "migrate" if fields.len()==4=>{
                let driver=field(&fields,"driverId")?;
                let stage=field(&fields,"stageName")?;
                self.no_live_global_cli_owner(&driver)?;
                let migrated=self.migrate_managed_cli(&driver,&stage)?;
                Ok(result("migrate",[("driverId",string(&driver)),
                    ("state",string("READY")),("migrated",Json::Number(migrated.to_string()))]))
            },
            _=>Err(OrchestrationError::Invalid("managed CLI command")),
        }
    }
}

#[cfg(all(test,windows))]
mod tests {
    use super::*;
    use crate::store::same_open::{create_new,route_b_test_guard};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex,OnceLock};
    use std::time::{SystemTime,UNIX_EPOCH};

    static PINNED_VERSION_ARCHIVES:OnceLock<Mutex<HashMap<(String,String),PathBuf>>>=OnceLock::new();

    fn confirm_durable_test_stop(product:&mut ProductDatabase<'_>,
        prepared:&crate::process::PreparedCustody,proof:&crate::process::NativeStopProof,
        revision:u64) {
        assert!(product.process_custodian.active(&prepared.ticket).is_some(),
            "native custody stays held until exact durable confirmation");
        let mut wrong_proof=proof.proof_hash();
        let replacement=if wrong_proof.as_bytes()[7]==b'0' {"1"} else {"0"};
        wrong_proof.replace_range(7..8,replacement);
        assert!(product.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:wrong_proof,
            durable_revision:revision,
        }).is_err(),"changed proof cannot release retained native custody");
        assert!(product.process_custodian.active(&prepared.ticket).is_some());
        assert!(product.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:proof.proof_hash(),
            durable_revision:0,
        }).is_err(),"zero is not a durable authority revision");
        assert!(product.process_custodian.active(&prepared.ticket).is_some());
        let confirmed=product.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:proof.proof_hash(),
            durable_revision:revision,
        }).unwrap();
        assert_eq!(confirmed.proof_hash(),proof.proof_hash());
        assert!(product.process_custodian.active(&prepared.ticket).is_none(),
            "durably confirmed stop releases the exact active ticket");
    }

    #[test]
    fn strict_legacy_source_history_route_fixtures() {
        instance::LegacySourceResult::run_focused_history_fixtures();
    }

    #[test]
    fn strict_legacy_source_released_route_fixtures() {
        instance::LegacySourceResult::run_released_source_fixtures();
    }

    #[test]
    fn strict_legacy_codex_unregistered_physical_acl_applies_and_replays() {
        strict_legacy_codex_physical_acl_applies_and_replays(false,false,false,None);
    }

    #[test]
    fn strict_legacy_codex_registered_physical_acl_applies_and_replays() {
        strict_legacy_codex_physical_acl_applies_and_replays(true,true,true,None);
    }

    #[test]
    fn strict_legacy_codex_complete_fence_family_without_target_row_applies_and_replays() {
        strict_legacy_codex_physical_acl_applies_and_replays(true,false,true,None);
    }

    #[test]
    fn strict_legacy_source_first_commit_error_after_real_commit_blocks_effect() {
        strict_legacy_codex_physical_acl_applies_and_replays(false,false,false,Some(true));
    }

    #[test]
    fn strict_legacy_source_first_commit_error_after_real_rollback_allows_fresh_admission() {
        strict_legacy_codex_physical_acl_applies_and_replays(false,false,false,Some(false));
    }

    fn strict_legacy_codex_physical_acl_applies_and_replays(
        registered: bool,fenced: bool,legacy_family_present: bool,
        first_commit_error_after: Option<bool>,
    ) {
        assert!(!fenced || legacy_family_present);
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-legacy-source-arm-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let (stage,image)=super::super::managed_cli_test_setup::prepared_image(&root,"codex");
        let pin=instance::read_fixed_official_cli("codex").unwrap();
        let old_path=path.join("original-codex-image");
        fs::write(&old_path,b"original synthetic Codex image").unwrap();
        let original_program=instance::ProgramObservation::observe(&old_path,"0.149.0").unwrap();
        let original_raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-INSTANCE","operation":"register","requestId":"registerA","targetId":"legacyA","domainId":"global","expectedRevision":"0","payload":{"driverId":"codex"}}"#;
        instance::register_instance(&mut product.connection,&root,&instance::Registration {
            request_id:"registerA",request_bytes:original_raw,instance_id:"legacyA",
            driver_id:"codex",program:&original_program,
        }).unwrap();
        let observed=instance::ProgramObservation::observe(&image,pin.version).unwrap();
        let observed_digest=format!("sha256:{}",pin.image_sha256);
        assert!(observed.matches_pin(&observed_digest,pin.version));
        let repin_raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-INSTANCE","operation":"repin-after-manual-upgrade","requestId":"repinA","targetId":"legacyA","domainId":"global","expectedRevision":"1","payload":{}}"#;
        instance::repin_program(&mut product.connection,&root,&product.owner,
            &instance::ProgramRepin {request_id:"repinA",request_bytes:repin_raw,
                instance_id:"legacyA",expected_revision:1},&observed).unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_instance_profiles(instance_id,display_name,enabled,tombstoned,revision) VALUES('legacyA','legacy fixture',0,0,1)").unwrap();
        let copy=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,stage_name,revision) VALUES('codex','READY',?1,?2,?3,?4,1)").unwrap();
        for (index,value) in [pin.version,pin.archive_sha256,pin.image_sha256,stage.as_str()]
            .iter().enumerate(){copy.bind_text((index+1) as i32,value).unwrap();}
        copy.step_done().unwrap();drop(copy);
        let original=Statement::prepare(product.connection.as_ptr(),
            "SELECT request_hex,native_receipt_id,phase FROM main.gogoke_v37_instance_operations WHERE request_id='registerA'").unwrap();
        assert!(original.step_row().unwrap());
        let fingerprint=original.column_text(0).unwrap();
        let home=original.column_text(1).unwrap();
        assert_eq!(original.column_text(2).unwrap(),"APPLIED");
        assert!(!original.step_row().unwrap());drop(original);
        let instance_before=table_rows(&product,
            "SELECT instance_id,program_digest,version,home_identity,revision FROM main.gogoke_v37_instances WHERE instance_id='legacyA'",5);
        let expected_root=product.connection.root_identity().opaque();
        let expected_database=product.connection.identity().opaque();
        let frame=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"migrate-legacy-codex-source\",\"driverId\":\"codex\",\"requestId\":\"sourceMoveA\",\"targetId\":\"legacyA\",\"originalRequestId\":\"registerA\",\"originalRequestFingerprint\":\"{fingerprint}\",\"originalNativeReceiptHex\":\"{}\",\"originalHomeIdentity\":\"{home}\",\"expectedRootIdentity\":\"{expected_root}\",\"expectedDatabaseIdentity\":\"{expected_database}\"}}",action_hex(home.as_bytes()));
        let home_path=root.canonical_root().canonical_path.join("v37-instances").join("legacyA");
        let home_identity=crate::root::inspect_root(&home_path).unwrap().identity;
        let auth_path=home_path.join("auth.json");
        fs::write(&auth_path,b"synthetic fixture source; never a real credential").unwrap();
        // The real native ACL producer must reject an otherwise complete
        // migration until the historical SID is physically on the HOME tree.
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err());
        let owner_name={
            let digest=crate::store::digest::sha256_hex(
                format!("{}\nlegacyA",home_identity.opaque()).as_bytes());
            format!("Gogoke37.OwnerLogin.{}",&digest[..40])
        };
        let (source_id,source_links)=crate::process::CredentialBinding::observe_source_metadata(
            &root,&auth_path,&home_identity).unwrap();
        assert_eq!(source_links,1);
        let original_binding=crate::process::CredentialBinding::open_registered(&root,
            &auth_path,&home_identity,&source_id,&[]).unwrap();
        assert!(crate::process::LegacyAclInventory::capture(&root,&home_path,
            &home_identity,&original_binding,&[owner_name.clone()]).is_err(),
            "the actual physical HOME lacks its historical broad SID ACE");
        drop(original_binding);
        let historical=crate::process::AppContainerProfile::derive_for_revocation(&owner_name).unwrap();
        historical.grant_bound_tree(&home_path,&home_identity,true).unwrap();
        historical.verify_bound_tree_grant(&home_path,&home_identity,true).unwrap();
        let unregistered_alias=path.join("unregistered-auth-alias");
        fs::hard_link(&auth_path,&unregistered_alias).unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "a second physical link without an F alias must refuse");
        fs::remove_file(&unregistered_alias).unwrap();
        assert!(product.dispatch_user_managed_cli(frame.replace(&home,"wrong-home").as_bytes()).is_err(),
            "caller HOME cannot replace the original registered HOME FileID");
        let original_binding=crate::process::CredentialBinding::open_registered(&root,
            &auth_path,&home_identity,&source_id,&[]).unwrap();
        let physical=crate::process::LegacyAclInventory::capture(&root,&home_path,
            &home_identity,&original_binding,&[owner_name.clone()]).unwrap();
        assert!(!physical.original_digest().is_empty());
        assert!(crate::process::LegacyAclInventory::capture(&root,&home_path,
            &root.canonical_root().identity,&original_binding,&[owner_name.clone()]).is_err(),
            "the native producer rejects a wrong physical HOME FileID");
        drop(original_binding);
        if registered {
        let (file_identity,links)=crate::process::CredentialBinding::observe_source_metadata(
            &root,&auth_path,&home_identity).unwrap();
        let operation_digest=crate::store::digest::sha256_hex(b"legacyA\nbackendA\n0");
        let backend_operation=format!("owner-login-{}",&operation_digest[..40]);
        let mut backend_launch=crate::process::ProcessLaunch::new(image.clone());
        backend_launch.arguments=vec!["--version".into()];
        backend_launch.current_directory=Some(home_path.clone());
        backend_launch.protocol_stdio=true;
        let backend_custody=product.process_custodian.prepare(&crate::process::PrepareRequest {
            launch:backend_launch,binding:crate::process::NativeBinding {
                binary_digest_sha256:observed_digest.clone(),profile_id:"legacyA".into(),
                domain_id:"global".into(),generation:"1".into(),
            }}).unwrap();
        crate::store::authority::record_prepared_process(&mut product.connection,
            &backend_operation,&backend_custody).unwrap();
        product.process_custodian.activate(&backend_custody).unwrap();
        crate::store::authority::mark_process_active(&mut product.connection,
            &backend_operation,&backend_custody).unwrap();
        let backend_stop=product.process_custodian.stop(&backend_custody.ticket,
            crate::process::StopBudgets::production(),||Ok(())).unwrap();
        let backend_stop_revision=crate::store::authority::mark_process_stopped(
            &mut product.connection,&backend_operation,&backend_stop).unwrap();
        confirm_durable_test_stop(&mut product,&backend_custody,&backend_stop,
            backend_stop_revision);
        // The backend journal is bound to the actual current program tuple
        // and original stopped C source, not just a copied FILE enum.
        instance::record_credential_backend(&mut product.connection,&instance::BackendSource {
            request_id:"backendA".into(),instance_id:"legacyA".into(),
            home_identity:home_identity.clone(),program_digest:observed_digest.clone(),
            version:pin.version.into(),backend:instance::CredentialBackend::File,
            startup_selector:instance::CredentialStartupSelector::FileBound,
            operation_id:backend_operation,ticket:backend_custody.ticket.opaque().into(),
            nonce:backend_custody.custodian_nonce.clone(),generation:"1".into(),
        }).unwrap();
        let root_identity=product.connection.root_identity().clone();
        instance::bind_credential_object(&mut product.connection,&instance::CredentialObjectInput {
            request_id:"objectA".into(),instance_id:"legacyA".into(),
            root_identity,
            home_identity:home_identity.clone(),file_identity,source_parent_identity:home_identity.clone(),
            observed_nlink:u64::from(links),expected_revision:0,rebind_from:None,
        }).unwrap();
        }
        assert_eq!(instance::read_credential_object(&product.connection,"legacyA")
            .unwrap().is_some(),registered,
            "the two positive cases differ in F credential registration");
        product.connection.execute(&format!(
            "INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('owner-login-1111111111111111111111111111111111111111','globalTicketA','globalNonceA','4294967295','2','synthetic.exe','{observed_digest}','legacyA','global','2','UNKNOWN')"
        )).unwrap();
        assert!(legacy_fence_schema_rows(&product).is_empty(),
            "ProductDatabase::open must leave the lazy legacy ACL family wholly absent");
        if legacy_family_present {
            instance::initialize_legacy_fence_schema(&mut product.connection).unwrap();
            assert_eq!(legacy_fence_schema_rows(&product).iter()
                .filter(|row| row[0]=="main" && row[1]=="table").count(),3,
                "fixture setup must create the exact complete legacy ACL family");
            assert!(instance::read_legacy_fence(&product.connection,"legacyA").unwrap().is_none(),
                "complete family setup must not fabricate a target fence");
        }
        if fenced {
            let custody=table_rows(&product,
                "SELECT operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody WHERE profile_id='legacyA' ORDER BY operation_id",12)
                .into_iter().map(|c|instance::LegacyCustodyRow {
                    operation_id:c[0].clone(),ticket:c[1].clone(),custodian_nonce:c[2].clone(),
                    pid:c[3].clone(),creation_time_100ns:c[4].clone(),image_path:c[5].clone(),
                    binary_digest_sha256:c[6].clone(),profile_id:c[7].clone(),
                    domain_id:c[8].clone(),generation:c[9].clone(),state:c[10].clone(),
                    stop_proof_hash:if c[11].is_empty(){None}else{Some(c[11].clone())},
                }).collect();
            let capture=instance::LegacyFenceCapture {
                instance_id:"legacyA".into(),
                original_boot:format!("boot-start:{:016x}",crate::process::native_boot_start().unwrap()),
                database_identity:product.connection.identity().clone(),
                root_identity:product.connection.root_identity().clone(),
                home_identity:home_identity.clone(),source_identity:source_id.clone(),
                source_parent_identity:home_identity.clone(),
                source_revision:instance::read_credential_object(&product.connection,"legacyA")
                    .unwrap().map(|row|row.revision),
                source_link_count:1,registered_alias_count:0,custody,
                home_acl_digest:physical.home_original_digest(),
                source_acl_digest:physical.source_original_digest(),
                acl_provenance_digest:crate::store::digest::sha256_hex(owner_name.as_bytes()),
                native_snapshot:physical.encode_snapshot(),
                native_snapshot_digest:physical.original_digest(),
            };
            instance::capture_legacy_fence(&mut product.connection,&capture).unwrap();
            let drift=home_path.join("post-fence-extra-runtime");
            fs::write(&drift,b"not in the original HOME inventory").unwrap();
            assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
                "fenced restore must reject physical HOME tree drift");
            fs::remove_file(drift).unwrap();
        }
        product.connection.execute("INSERT INTO main.gogoke_v37_h_owner_binding(binding_id,instance_id,domain_id,kind,owner_id,generation,state) VALUES('unstartedBindingA','legacyA','unstartedProjectA','SESSION','unstartedSessionA','1','ACTIVE')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES('unstartedHomeA','legacyA','unstartedProjectA','SESSION','unstartedSessionA','1','ACTIVE',1)").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('unstartedProjectA','unstartedSessionA','legacyA','unstartedHomeA','unstartedBindingA','1','COMMITTED',2)").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "a process-less COMMITTED reservation cannot be discharged by empty native pairs");
        product.connection.execute("UPDATE main.gogoke_v37_h_claim SET state='RELEASED',revision=3 WHERE domain_id='unstartedProjectA' AND session_id='unstartedSessionA'").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "RELEASED alone cannot substitute for original H raw and F HOME creation");
        product.connection.execute("DELETE FROM main.gogoke_v37_h_claim WHERE domain_id='unstartedProjectA' AND session_id='unstartedSessionA'").unwrap();
        product.connection.execute("DELETE FROM main.gogoke_v37_instance_homes WHERE home_id='unstartedHomeA'").unwrap();
        product.connection.execute("DELETE FROM main.gogoke_v37_h_owner_binding WHERE binding_id='unstartedBindingA'").unwrap();
        let origin=crate::store::seat::NativeOrigin::user(&product.owner);
        crate::store::seat::store_template(&mut product.connection,origin,
            crate::store::seat::StoreTemplate {domain_id:"unstartedProjectA",template_id:"unstartedTemplateA",
                settings_json:br#"{"instruction":"default"}"#}).unwrap();
        let created=crate::store::seat::create(&mut product.connection,
            crate::store::seat::NativeOrigin::user(&product.owner),
            crate::store::seat::CreateSeat {domain_id:"unstartedProjectA",seat_id:"unstartedSeatA",
                template_id:"unstartedTemplateA",instance_id:Some("legacyA"),
                kind:crate::store::seat::Kind::Long,request_id:"unstartedSeatCreateA",
                request_bytes:b"create historical test seat"}).unwrap().seat;
        let busy=crate::store::seat::set_dispatch_state(&mut product.connection,&created,true).unwrap();
        let generation=busy.generation.to_string();
        let released_suffix=crate::store::digest::sha256_hex(format!(
            "{}\nunstartedProjectA\nreleasedSessionA\n{}\n{generation}",
            root.canonical_root().identity.opaque(),busy.incarnation).as_bytes());
        let released_suffix=&released_suffix[..40];
        let released_home=format!("home-{released_suffix}");
        let released_binding=format!("binding-{released_suffix}");
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        crate::store::session_transport::bind_owner_in_transaction(&mut product.connection,
            &crate::store::session_transport::OwnerBinding {binding_id:&released_binding,
                instance_id:"legacyA",domain_id:"unstartedProjectA",kind:"SESSION",
                owner_id:"releasedSessionA",generation:&generation}).unwrap();
        let selection=crate::store::session_transport::session_binding::SessionBinding {
            domain_id:"unstartedProjectA".into(),session_id:"releasedSessionA".into(),
            seat_id:busy.seat_id.clone(),seat_incarnation:busy.incarnation.clone(),
            seat_authorization_generation:busy.generation,selected_instance_id:"legacyA".into(),
            provenance:crate::store::session_transport::session_binding::Provenance::NativeV2,
        };
        crate::store::session_transport::session_binding::select_native_in_transaction(
            &product.connection,&selection).unwrap();
        product.connection.execute("COMMIT").unwrap();
        let reserve_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-reserve\",\"requestId\":\"releasedReserveA\",\"targetId\":\"releasedSessionA\",\"domainId\":\"unstartedProjectA\",\"expectedRevision\":\"0\",\"payload\":{{\"seatId\":\"{}\",\"generation\":\"{generation}\"}}}}",busy.seat_id);
        let preparation=crate::store::digest::sha256_hex(b"unstartedProjectA\nreleasedReserveA");
        let home_request=format!("homeprep-{}",&preparation[..40]);
        let session_profile=crate::process::AppContainerProfile::derive_for_revocation(
            &format!("Gogoke37.Session.{released_suffix}")).unwrap();
        instance::create_temporary_home(&mut product.connection,&root,&session_profile,
            &instance::CreateTemporaryHome {request_id:&home_request,request_bytes:reserve_raw.as_bytes(),
                home_id:&released_home,instance_id:"legacyA",domain_id:"unstartedProjectA",
                kind:instance::TemporaryKind::Session,owner_id:"releasedSessionA",generation:&generation}).unwrap();
        for (operation,request,expected) in [
            ("admission-reserve","releasedReserveA",0_i64),
            ("admission-commit","releasedCommitA",1_i64),
        ] {
            let raw=if expected==0 {reserve_raw.clone()} else {format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"{operation}\",\"requestId\":\"{request}\",\"targetId\":\"releasedSessionA\",\"domainId\":\"unstartedProjectA\",\"expectedRevision\":\"{expected}\",\"payload\":{{\"seatId\":\"{}\",\"generation\":\"{generation}\"}}}}",busy.seat_id)};
            let input=crate::store::session_transport::AdmissionRequest {domain_id:"unstartedProjectA",
                session_id:"releasedSessionA",request_id:request,raw_bytes:raw.as_bytes(),
                instance_id:"legacyA",home_id:&released_home,generation:&generation,
                expected_revision:expected};
            let result=if expected==0 {
                crate::store::session_transport::reserve_admission(&mut product.connection,&input,
                    |_|Ok(crate::store::session_transport::TrustedLimits {
                        project_parallel:4,instance_concurrency:4})).unwrap()
            } else {
                crate::store::session_transport::commit_admission(&mut product.connection,&input,
                    |_|Ok(())).unwrap()
            };
            assert!(matches!(result,crate::store::session_transport::AdmissionResult::Applied(_)));
        }
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "original process-less COMMITTED reservation remains unqualified");
        let release_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-release\",\"requestId\":\"releasedReleaseA\",\"targetId\":\"releasedSessionA\",\"domainId\":\"unstartedProjectA\",\"expectedRevision\":\"2\",\"payload\":{{\"seatId\":\"{}\",\"generation\":\"{generation}\"}}}}",busy.seat_id);
        let release=crate::store::session_transport::AdmissionRequest {domain_id:"unstartedProjectA",
            session_id:"releasedSessionA",request_id:"releasedReleaseA",raw_bytes:release_raw.as_bytes(),
            instance_id:"legacyA",home_id:&released_home,generation:&generation,expected_revision:2};
        assert!(matches!(crate::store::session_transport::runtime::release_native(
            &mut product.connection,&crate::store::seat::NativeOrigin::user(&product.owner),&release).unwrap(),
            crate::store::session_transport::AdmissionResult::Applied(3)));
        // The production release state machine also permits RESERVED to move
        // directly to RELEASED. Keep this second original history beside the
        // three-step case so migration qualification cannot invent a commit.
        crate::store::seat::store_template(&mut product.connection,
            crate::store::seat::NativeOrigin::user(&product.owner),
            crate::store::seat::StoreTemplate {domain_id:"directProjectA",
                template_id:"directTemplateA",settings_json:br#"{"instruction":"default"}"#}).unwrap();
        let direct_created=crate::store::seat::create(&mut product.connection,
            crate::store::seat::NativeOrigin::user(&product.owner),
            crate::store::seat::CreateSeat {domain_id:"directProjectA",seat_id:"directSeatA",
                template_id:"directTemplateA",instance_id:Some("legacyA"),
                kind:crate::store::seat::Kind::Long,request_id:"directSeatCreateA",
                request_bytes:b"create direct-release historical seat"}).unwrap().seat;
        let direct_busy=crate::store::seat::set_dispatch_state(
            &mut product.connection,&direct_created,true).unwrap();
        let direct_generation=direct_busy.generation.to_string();
        let direct_suffix=crate::store::digest::sha256_hex(format!(
            "{}\ndirectProjectA\ndirectSessionA\n{}\n{direct_generation}",
            root.canonical_root().identity.opaque(),direct_busy.incarnation).as_bytes());
        let direct_suffix=&direct_suffix[..40];
        let direct_home=format!("home-{direct_suffix}");
        let direct_binding=format!("binding-{direct_suffix}");
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        crate::store::session_transport::bind_owner_in_transaction(&mut product.connection,
            &crate::store::session_transport::OwnerBinding {binding_id:&direct_binding,
                instance_id:"legacyA",domain_id:"directProjectA",kind:"SESSION",
                owner_id:"directSessionA",generation:&direct_generation}).unwrap();
        let direct_selection=crate::store::session_transport::session_binding::SessionBinding {
            domain_id:"directProjectA".into(),session_id:"directSessionA".into(),
            seat_id:direct_busy.seat_id.clone(),seat_incarnation:direct_busy.incarnation.clone(),
            seat_authorization_generation:direct_busy.generation,
            selected_instance_id:"legacyA".into(),
            provenance:crate::store::session_transport::session_binding::Provenance::NativeV2,
        };
        crate::store::session_transport::session_binding::select_native_in_transaction(
            &product.connection,&direct_selection).unwrap();
        product.connection.execute("COMMIT").unwrap();
        let direct_reserve_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-reserve\",\"requestId\":\"directReserveA\",\"targetId\":\"directSessionA\",\"domainId\":\"directProjectA\",\"expectedRevision\":\"0\",\"payload\":{{\"seatId\":\"{}\",\"generation\":\"{direct_generation}\"}}}}",direct_busy.seat_id);
        let direct_preparation=crate::store::digest::sha256_hex(b"directProjectA\ndirectReserveA");
        let direct_home_request=format!("homeprep-{}",&direct_preparation[..40]);
        let direct_profile=crate::process::AppContainerProfile::derive_for_revocation(
            &format!("Gogoke37.Session.{direct_suffix}")).unwrap();
        instance::create_temporary_home(&mut product.connection,&root,&direct_profile,
            &instance::CreateTemporaryHome {request_id:&direct_home_request,
                request_bytes:direct_reserve_raw.as_bytes(),home_id:&direct_home,
                instance_id:"legacyA",domain_id:"directProjectA",
                kind:instance::TemporaryKind::Session,owner_id:"directSessionA",
                generation:&direct_generation}).unwrap();
        let direct_reserve=crate::store::session_transport::AdmissionRequest {
            domain_id:"directProjectA",session_id:"directSessionA",request_id:"directReserveA",
            raw_bytes:direct_reserve_raw.as_bytes(),instance_id:"legacyA",home_id:&direct_home,
            generation:&direct_generation,expected_revision:0};
        assert!(matches!(crate::store::session_transport::reserve_admission(
            &mut product.connection,&direct_reserve,|_|Ok(
                crate::store::session_transport::TrustedLimits {
                    project_parallel:4,instance_concurrency:4})).unwrap(),
            crate::store::session_transport::AdmissionResult::Applied(1)));
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "a still-RESERVED original is not settled");
        let direct_release_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-release\",\"requestId\":\"directReleaseA\",\"targetId\":\"directSessionA\",\"domainId\":\"directProjectA\",\"expectedRevision\":\"1\",\"payload\":{{\"seatId\":\"{}\",\"generation\":\"{direct_generation}\"}}}}",direct_busy.seat_id);
        let direct_release=crate::store::session_transport::AdmissionRequest {
            domain_id:"directProjectA",session_id:"directSessionA",request_id:"directReleaseA",
            raw_bytes:direct_release_raw.as_bytes(),instance_id:"legacyA",home_id:&direct_home,
            generation:&direct_generation,expected_revision:1};
        assert!(matches!(crate::store::session_transport::runtime::release_native(
            &mut product.connection,&crate::store::seat::NativeOrigin::user(&product.owner),
            &direct_release).unwrap(),crate::store::session_transport::AdmissionResult::Applied(2)));
        product.connection.execute("DELETE FROM main.gogoke_v37_native_selection WHERE domain_id='directProjectA' AND session_id='directSessionA'").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "a released claim cannot invent its missing original native selection");
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        crate::store::session_transport::session_binding::select_native_in_transaction(
            &product.connection,&direct_selection).unwrap();
        product.connection.execute("COMMIT").unwrap();
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        crate::store::session_transport::session_binding::insert_native_in_transaction(
            &product.connection,&direct_selection).unwrap();
        product.connection.execute("COMMIT").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "a post-OPEN v2 relationship contradicts never-started classification");
        product.connection.execute("DELETE FROM main.gogoke_v37_session_binding_v2 WHERE domain_id='directProjectA' AND session_id='directSessionA'").unwrap();
        let open=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('unstartedProjectA','unstartedOpenIntentA',?1,'open','releasedSessionA','APPLIED',3,4)").unwrap();
        open.bind_text(1,&action_hex(b"synthetic open intent only")).unwrap();open.step_done().unwrap();drop(open);
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "a later original H open intent forbids never-started classification");
        product.connection.execute("DELETE FROM main.gogoke_v37_h_operation WHERE domain_id='unstartedProjectA' AND request_id='unstartedOpenIntentA'").unwrap();
        let original_before=table_rows(&product,
            "SELECT request_id,request_hex,target_id,phase,COALESCE(native_receipt_id,'') FROM main.gogoke_v37_instance_operations ORDER BY request_id",5);
        let legacy_schema_before=legacy_fence_schema_rows(&product);
        let legacy_rows_before=legacy_fence_family_rows(&product,legacy_family_present);
        let original_receipt_hex=action_hex(home.as_bytes());
        let qualification_input=instance::LegacySourceRequest {
            raw:frame.as_bytes(),request_id:"sourceMoveA",target_id:"legacyA",
            original_request_id:"registerA",original_fingerprint:&fingerprint,
            original_receipt_hex:&original_receipt_hex,original_home_identity:&home,
            expected_root_identity:&expected_root,expected_database_identity:&expected_database,
        };
        let qualified=instance::LegacySourceResult::qualification_digest_for_test(
            &product.connection,&root,&qualification_input).unwrap();
        // Back up exact SQL values/types without enclosing the USER dispatcher
        // in a SAVEPOINT: the dispatcher owns its own authority transactions.
        for (table,predicate,value) in [
            ("gogoke_v37_instance_homes","home_id=?1",released_home.as_str()),
            ("gogoke_v37_h_operation","domain_id='unstartedProjectA' AND request_id=?1","releasedReserveA"),
            ("gogoke_v37_instance_operations","request_id=?1",home_request.as_str()),
        ] {
            let backup=Statement::prepare(product.connection.as_ptr(),&format!(
                "CREATE TEMP TABLE source_history_restore AS SELECT * FROM main.{table} WHERE {predicate}")).unwrap();
            backup.bind_text(1,value).unwrap();backup.step_done().unwrap();drop(backup);
            assert_eq!(table_rows(&product,
                "SELECT CAST(COUNT(*) AS TEXT) FROM temp.source_history_restore",1),
                vec![vec!["1".to_owned()]]);
            assert_eq!(instance::LegacySourceResult::qualification_digest_for_test(
                &product.connection,&root,&qualification_input).unwrap(),qualified);
            let remove=Statement::prepare(product.connection.as_ptr(),&format!(
                "DELETE FROM main.{table} WHERE {predicate}")).unwrap();
            remove.bind_text(1,value).unwrap();remove.step_done().unwrap();drop(remove);
            assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
                "a RELEASED claim still requires its original {table} source");
            assert!(table_rows(&product,
                "SELECT request_id FROM main.gogoke_v37_instance_legacy_source_migrations",1).is_empty());
            assert!(table_rows(&product,
                "SELECT instance_id FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",1).is_empty());
            product.connection.execute(&format!(
                "INSERT INTO main.{table} SELECT * FROM temp.source_history_restore; DROP TABLE temp.source_history_restore")).unwrap();
            assert_eq!(instance::LegacySourceResult::qualification_digest_for_test(
                &product.connection,&root,&qualification_input).unwrap(),qualified,
                "restoring the exact original source must restore the complete positive qualification");
        }
        let create=Statement::prepare(product.connection.as_ptr(),
            "SELECT request_hex FROM main.gogoke_v37_instance_operations WHERE request_id=?1").unwrap();
        create.bind_text(1,&home_request).unwrap();assert!(create.step_row().unwrap());
        let original_create=create.column_text(0).unwrap();
        assert!(!create.step_row().unwrap());drop(create);
        let original_sid=session_profile.sid_identity().unwrap();
        let mut changed_sid=original_sid.clone();
        let last=changed_sid.pop().unwrap();changed_sid.push(if last=='0' {'1'} else {'0'});
        assert!(changed_sid.starts_with("S-1-15-2-"));assert_ne!(changed_sid,original_sid);
        let sid_hex=action_hex(original_sid.as_bytes());
        let prefix=original_create.strip_suffix(&sid_hex).expect("original F create ends in its exact native SID");
        let changed_create=format!("{prefix}{}",action_hex(changed_sid.as_bytes()));
        assert_eq!(changed_create.len(),original_create.len());
        let change=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_operations SET request_hex=?1 WHERE request_id=?2").unwrap();
        change.bind_text(1,&changed_create).unwrap();change.bind_text(2,&home_request).unwrap();
        change.step_done().unwrap();drop(change);
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "another well-formed AppContainer SID cannot replace the original native HOME SID");
        let restore=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_operations SET request_hex=?1 WHERE request_id=?2").unwrap();
        restore.bind_text(1,&original_create).unwrap();restore.bind_text(2,&home_request).unwrap();
        restore.step_done().unwrap();drop(restore);
        assert_eq!(instance::LegacySourceResult::qualification_digest_for_test(
            &product.connection,&root,&qualification_input).unwrap(),qualified);
        assert_eq!(table_rows(&product,
            "SELECT request_id,request_hex,target_id,phase,COALESCE(native_receipt_id,'') FROM main.gogoke_v37_instance_operations ORDER BY request_id",5),original_before);
        assert!(product.dispatch_user_managed_cli(frame.replace(&fingerprint,"00").as_bytes()).is_err());
        assert!(product.dispatch_user_managed_cli(frame.replacen("}",",\"extra\":\"x\"}",1).as_bytes()).is_err());
        let missing_stage=format!("codex-{}-missing",pin.version);
        let changed_stage=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_cli_copies SET stage_name=?1 WHERE driver_id='codex'").unwrap();
        changed_stage.bind_text(1,&missing_stage).unwrap();changed_stage.step_done().unwrap();drop(changed_stage);
        let stage_error=instance::locate_ready_managed_program_from_db(&product.connection,
            "codex",&observed_digest,pin.version).unwrap_err();
        let original_code=match &stage_error {
            instance::ManagedCliError::Io(error) => error.raw_os_error().unwrap(),
            other => panic!("missing stage must yield its original IO error: {other:?}"),
        };
        let delivered=product.dispatch_user_managed_cli(frame.as_bytes()).unwrap_err();
        let delivered_text=format!("{delivered:?}");
        assert!(delivered_text.contains("official stage") &&
            delivered_text.contains("NotFound") &&
            delivered_text.contains(&format!("code: {original_code}")),
            "migration must preserve the original read-only stage error: {delivered:?}");
        assert!(!path.join("v37-managed-cli").join("staging").join(&missing_stage).exists());
        assert!(table_rows(&product,
            "SELECT request_id FROM main.gogoke_v37_instance_legacy_source_migrations",1).is_empty());
        assert!(table_rows(&product,
            "SELECT instance_id FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",1).is_empty());
        let restored=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_cli_copies SET stage_name=?1 WHERE driver_id='codex'").unwrap();
        restored.bind_text(1,&stage).unwrap();restored.step_done().unwrap();drop(restored);
        product.connection.execute("UPDATE main.gogoke_v37_instance_cli_copies SET state='BLOCKED',raw_error='original managed copy failure' WHERE driver_id='codex'").unwrap();
        let delivered=product.dispatch_user_managed_cli(frame.as_bytes()).unwrap_err();
        assert!(format!("{delivered:?}").contains("original managed copy failure"),
            "persisted managed copy error must survive migration refusal");
        product.connection.execute("UPDATE main.gogoke_v37_instance_cli_copies SET state='READY',raw_error=NULL WHERE driver_id='codex'").unwrap();
        let managed=path.join("v37-managed-cli");
        let staging=managed.join("staging");
        for (original,held) in [
            (managed.clone(),path.join("v37-managed-cli-held")),
            (staging.clone(),managed.join("staging-held")),
        ] {
            fs::rename(&original,&held).unwrap();
            assert!(!original.exists());
            assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
                "missing managed/staging directory must refuse");
            assert!(!original.exists(),
                "SOURCE prequalification must not recreate a missing managed/staging directory");
            fs::rename(&held,&original).unwrap();
        }
        assert!(table_rows(&product,
            "SELECT request_id FROM main.gogoke_v37_instance_legacy_source_migrations",1).is_empty());
        assert!(table_rows(&product,
            "SELECT instance_id FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",1).is_empty());
        product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('unsettledA','ticketA','nonceA','4294967295','1','synthetic.exe','sha256:0000000000000000000000000000000000000000000000000000000000000000','legacyA','global','1','UNKNOWN')").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err());
        product.connection.execute("DELETE FROM main.gogoke_coordination_process_custody WHERE operation_id='unsettledA'").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES('temporaryA','legacyA','projectA','SESSION','sessionA','1','CLOSED',2)").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES('pendingHomeA','pendingHomeA','temporaryA','PREPARING')").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err());
        product.connection.execute("DELETE FROM main.gogoke_v37_instance_operations WHERE request_id='pendingHomeA'").unwrap();
        product.connection.execute("DELETE FROM main.gogoke_v37_instance_homes WHERE home_id='temporaryA'").unwrap();
        let credential_target=format!("credential-instance-{}",crate::store::digest::sha256_hex(b"legacyA"));
        let pending=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES('pendingCredentialA','pendingCredentialA',?1,'PREPARING')").unwrap();
        pending.bind_text(1,&credential_target).unwrap();pending.step_done().unwrap();drop(pending);
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err());
        product.connection.execute("DELETE FROM main.gogoke_v37_instance_operations WHERE request_id='pendingCredentialA'").unwrap();
        assert!(table_rows(&product,
            "SELECT request_id FROM main.gogoke_v37_instance_legacy_source_migrations",1).is_empty());
        // The otherwise complete positive fixture must refuse ANY previous
        // migration, including one for a different target. These controlled
        // journal rows test exclusion; they are not another migration proof.
        let journal_rows="SELECT request_id,target_id,raw_hex,raw_sha256,original_request_id,original_fingerprint,original_receipt_hex,original_home_identity,root_identity,database_identity,qualification_digest,phase,COALESCE(source_fingerprint,''),COALESCE(receipt_hex,'') FROM main.gogoke_v37_instance_legacy_source_migrations ORDER BY request_id";
        for phase in ["PREPARING","APPLIED"] {
            let prior=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_legacy_source_migrations(request_id,target_id,raw_hex,raw_sha256,original_request_id,original_fingerprint,original_receipt_hex,original_home_identity,root_identity,database_identity,qualification_digest,phase,source_fingerprint,receipt_hex) VALUES('priorMigration','priorTarget',?1,?2,'priorRegistration',?3,?4,?5,?6,?7,'controlled-prior-qualification',?8,CASE WHEN ?8='APPLIED' THEN 'controlled-prior-source' ELSE NULL END,CASE WHEN ?8='APPLIED' THEN '7b7d' ELSE NULL END)").unwrap();
            for (index,value) in [action_hex(frame.as_bytes()),
                crate::store::digest::sha256_hex(frame.as_bytes()),fingerprint.clone(),
                action_hex(b"controlled prior registration receipt"),home.clone(),
                product.connection.root_identity().opaque(),product.connection.identity().opaque(),
                phase.to_owned()].iter().enumerate() {
                prior.bind_text((index+1) as i32,value).unwrap();
            }
            prior.step_done().unwrap();drop(prior);
            let prior_rows=table_rows(&product,journal_rows,14);
            assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
                "a different target's {phase} migration blocks a new migration");
            assert_eq!(table_rows(&product,journal_rows,14),prior_rows,
                "refusal must leave the previous journal and request set unchanged");
            assert!(table_rows(&product,
                "SELECT instance_id FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",1).is_empty());
            product.connection.execute("DELETE FROM main.gogoke_v37_instance_legacy_source_migrations WHERE request_id='priorMigration'").unwrap();
        }
        let root_field=format!("\"expectedRootIdentity\":\"{expected_root}\"");
        let database_field=format!("\"expectedDatabaseIdentity\":\"{expected_database}\"");
        for (field,replacement) in [
            (&root_field,"\"expectedRootIdentity\":\"\""),
            (&root_field,"\"expectedRootIdentity\":\"wrong-root\""),
            (&database_field,"\"expectedDatabaseIdentity\":\"\""),
            (&database_field,"\"expectedDatabaseIdentity\":\"wrong-database\""),
        ] {
            let changed=frame.replace(field,replacement);
            assert_ne!(changed,frame);
            assert!(product.dispatch_user_managed_cli(changed.as_bytes()).is_err(),
                "an empty or wrong physical authority expectation must refuse");
        }
        assert!(table_rows(&product,
            "SELECT request_id FROM main.gogoke_v37_instance_legacy_source_migrations",1).is_empty());
        assert!(table_rows(&product,
            "SELECT instance_id FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",1).is_empty());
        if let Some(commit_first) = first_commit_error_after {
            instance::test_first_admission_commit_error_after(commit_first);
            let error=product.dispatch_user_managed_cli(frame.as_bytes()).unwrap_err();
            assert!(format!("{error:?}").contains("ADMISSION_COMMIT_UNKNOWN"),
                "the original first-COMMIT error must retain its admission-unknown label: {error:?}");
            assert!(format!("{error:?}").contains("controlled first COMMIT return error"),
                "the admission-unknown label cannot erase the original error: {error:?}");
            assert!(table_rows(&product,
                "SELECT instance_id FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",1).is_empty(),
                "a first-COMMIT error cannot enter the SOURCE transaction");
            let journal=table_rows(&product,
                "SELECT request_id,phase FROM main.gogoke_v37_instance_legacy_source_migrations",2);
            if commit_first {
                assert_eq!(journal,vec![vec!["sourceMoveA".to_owned(),"PREPARING".to_owned()]],
                    "actual committed PREPARING retains the original ID");
                assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
                    "a durable PREPARING row cannot resume the effect");
                assert!(product.dispatch_user_managed_cli(frame.replace("sourceMoveA","sourceMoveB").as_bytes()).is_err(),
                    "a fresh ID cannot pass a durable PREPARING row");
            } else {
                assert!(journal.is_empty(), "actual rollback left no durable admission");
                let journal_schema=table_rows(&product,
                    "SELECT sql FROM main.sqlite_schema WHERE type='table' AND name='gogoke_v37_instance_legacy_source_migrations'",1);
                assert_eq!(journal_schema.len(),1);
                product.connection.execute("DROP TABLE main.gogoke_v37_instance_legacy_source_migrations").unwrap();
                assert!(product.dispatch_user_managed_cli(
                    frame.replace("sourceMoveA","sourceMoveB").as_bytes()).is_err(),
                    "missing dedicated journal schema cannot be treated as empty");
                assert!(table_rows(&product,
                    "SELECT instance_id FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",1).is_empty());
                product.connection.execute(&journal_schema[0][0]).unwrap();
                let fresh=product.dispatch_user_managed_cli(
                    frame.replace("sourceMoveA","sourceMoveB").as_bytes()).unwrap();
                assert!(String::from_utf8_lossy(&fresh).contains("\"status\":\"APPLIED\""));
                assert_eq!(table_rows(&product,
                    "SELECT request_id,phase FROM main.gogoke_v37_instance_legacy_source_migrations",2),
                    vec![vec!["sourceMoveB".to_owned(),"APPLIED".to_owned()]]);
            }
            assert_eq!(table_rows(&product,
                "SELECT request_id,request_hex,target_id,phase,COALESCE(native_receipt_id,'') FROM main.gogoke_v37_instance_operations ORDER BY request_id",5),original_before);
            product.close_checked().unwrap();drop(root);
            fs::remove_file(database).unwrap();
            fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
            fs::remove_dir_all(path).unwrap();
            return;
        }
        if !registered && !legacy_family_present {
            // Clone a complete, otherwise eligible physical database under the
            // same RootLock. The old DB FileID must refuse; the clone's own
            // retained FileID must pass the same native qualifications.
            let copied_database=path.join("state-copy.sqlite");
            // Route-B forbids ATTACH, including VACUUM INTO's internal ATTACH.
            // Copy only a normally closed database while retaining RootLock.
            product.close_checked().unwrap();
            fs::copy(&database,&copied_database).unwrap();
            product=ProductDatabase::open(&root,&database).unwrap();
            let mut copied=ProductDatabase::open(&root,&copied_database).unwrap();
            let copied_identity=copied.connection.identity().opaque();
            assert_eq!(copied.connection.root_identity().opaque(),expected_root);
            assert_ne!(copied_identity,expected_database);
            assert!(copied.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
                "the old expected DB FileID must refuse a complete copied DB under the same root");
            assert!(table_rows(&copied,
                "SELECT request_id FROM main.gogoke_v37_instance_legacy_source_migrations",1).is_empty());
            let copied_frame=frame.replace(&database_field,
                &format!("\"expectedDatabaseIdentity\":\"{copied_identity}\""));
            assert_ne!(copied_frame,frame);
            assert!(String::from_utf8_lossy(&copied.dispatch_user_managed_cli(
                copied_frame.as_bytes()).unwrap()).contains("\"status\":\"APPLIED\""),
                "the copied DB must be otherwise eligible, not a false-negative fixture");
            copied.close_checked().unwrap();
            fs::remove_file(copied_database).unwrap();
            fs::remove_file(path.join(".gogoke-state-copy.sqlite.custody-v1")).unwrap();
        }
        let applied=product.dispatch_user_managed_cli(frame.as_bytes()).unwrap();
        assert!(String::from_utf8_lossy(&applied).contains("\"status\":\"APPLIED\""));
        assert_eq!(table_rows(&product,
            "SELECT request_id,request_hex,target_id,phase,COALESCE(native_receipt_id,'') FROM main.gogoke_v37_instance_operations ORDER BY request_id",5),original_before);
        assert_eq!(table_rows(&product,
            "SELECT instance_id,program_digest,version,home_identity,revision FROM main.gogoke_v37_instances WHERE instance_id='legacyA'",5),instance_before);
        assert_eq!(table_rows(&product,
            "SELECT source,stage_name,program_digest,version,home_identity,registration_request_id,revision FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",7),
            vec![vec!["MANAGED".into(),stage.clone(),observed_digest,
                pin.version.into(),home.clone(),"registerA".into(),"1".into()]]);
        assert_eq!(table_rows(&product,
            "SELECT request_id,phase FROM main.gogoke_v37_instance_legacy_source_migrations",2),
            vec![vec![String::from("sourceMoveA"),String::from("APPLIED")]]);
        assert_eq!(legacy_fence_schema_rows(&product),legacy_schema_before,
            "legacy SOURCE migration must not initialize or alter the optional family schema");
        assert_eq!(legacy_fence_family_rows(&product,legacy_family_present),legacy_rows_before,
            "legacy SOURCE migration must not alter any optional family row");
        assert_eq!(product.dispatch_user_managed_cli(frame.as_bytes()).unwrap(),applied);
        let applied_source=table_rows(&product,
            "SELECT source,stage_name,program_digest,version,home_identity,registration_request_id,revision FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",7);
        let applied_journal=table_rows(&product,journal_rows,14);
        product.connection.execute("UPDATE main.gogoke_v37_instance_program_sources SET version='3' WHERE instance_id='legacyA'").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "APPLIED replay cannot use changed SOURCE contents");
        let restore=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_program_sources SET version=?1 WHERE instance_id='legacyA'").unwrap();
        restore.bind_text(1,pin.version).unwrap();restore.step_done().unwrap();drop(restore);
        assert_eq!(table_rows(&product,journal_rows,14),applied_journal,
            "SOURCE mismatch refusal leaves the migration journal unchanged");
        assert_eq!(table_rows(&product,
            "SELECT source,stage_name,program_digest,version,home_identity,registration_request_id,revision FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",7),applied_source);
        assert_eq!(product.dispatch_user_managed_cli(frame.as_bytes()).unwrap(),applied,
            "restored SOURCE yields only the original migration receipt");
        assert_eq!(table_rows(&product,
            "SELECT typeof(receipt_json) FROM main.gogoke_v37_instance_operations WHERE request_id='registerA'",1),
            vec![vec!["null".to_owned()]],
            "the original register producer's nullable receipt field must be known before restore");
        product.connection.execute("DELETE FROM main.gogoke_v37_instance_operations WHERE request_id='registerA'").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "APPLIED replay cannot use a missing original registration");
        let restore=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase,native_receipt_id) VALUES('registerA',?1,'legacyA','APPLIED',?2)").unwrap();
        restore.bind_text(1,&fingerprint).unwrap();restore.bind_text(2,&home).unwrap();
        restore.step_done().unwrap();drop(restore);
        product.connection.execute("UPDATE main.gogoke_v37_instance_operations SET request_hex='00' WHERE request_id='registerA'").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "APPLIED replay cannot use changed original raw");
        let restore=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_operations SET request_hex=?1 WHERE request_id='registerA'").unwrap();
        restore.bind_text(1,&fingerprint).unwrap();restore.step_done().unwrap();drop(restore);
        product.connection.execute("UPDATE main.gogoke_v37_instance_operations SET phase='UNKNOWN' WHERE request_id='registerA'").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "APPLIED replay cannot use changed original phase");
        product.connection.execute("UPDATE main.gogoke_v37_instance_operations SET phase='APPLIED' WHERE request_id='registerA'").unwrap();
        product.connection.execute("UPDATE main.gogoke_v37_instance_operations SET native_receipt_id='wrong-home' WHERE request_id='registerA'").unwrap();
        assert!(product.dispatch_user_managed_cli(frame.as_bytes()).is_err(),
            "APPLIED replay cannot use changed original native receipt");
        let restore=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_operations SET native_receipt_id=?1 WHERE request_id='registerA'").unwrap();
        restore.bind_text(1,&home).unwrap();restore.step_done().unwrap();drop(restore);
        assert_eq!(table_rows(&product,journal_rows,14),applied_journal,
            "all APPLIED readback refusals leave the migration journal unchanged");
        assert_eq!(table_rows(&product,
            "SELECT source,stage_name,program_digest,version,home_identity,registration_request_id,revision FROM main.gogoke_v37_instance_program_sources WHERE instance_id='legacyA'",7),applied_source,
            "all APPLIED readback refusals leave SOURCE unchanged");
        assert_eq!(table_rows(&product,
            "SELECT request_id,request_hex,target_id,phase,COALESCE(native_receipt_id,'') FROM main.gogoke_v37_instance_operations ORDER BY request_id",5),original_before);
        assert_eq!(product.dispatch_user_managed_cli(frame.as_bytes()).unwrap(),applied,
            "restored original registration yields only the original migration receipt");
        assert_eq!(legacy_fence_schema_rows(&product),legacy_schema_before,
            "exact replay must not initialize or alter the optional family schema");
        assert_eq!(legacy_fence_family_rows(&product,legacy_family_present),legacy_rows_before,
            "exact replay must not alter any optional family row");
        assert!(product.dispatch_user_managed_cli(format!(" {frame}").as_bytes()).is_err());
        assert!(product.dispatch_user_managed_cli(frame.replace("sourceMoveA","sourceMoveB").as_bytes()).is_err());
        for (field,replacement) in [
            (&root_field,"\"expectedRootIdentity\":\"wrong-root\""),
            (&database_field,"\"expectedDatabaseIdentity\":\"wrong-database\""),
        ] {
            let changed_anchor=frame.replace(field,replacement);
            assert_ne!(changed_anchor.as_bytes(),frame.as_bytes(),
                "same-ID anchor changes must change the exact original raw bytes");
            assert!(product.dispatch_user_managed_cli(changed_anchor.as_bytes()).is_err());
        }
        assert!(product.dispatch_user_managed_cli(frame.replacen("}",",\"extra\":\"x\"}",1).as_bytes()).is_err());
        assert_eq!(table_rows(&product,
            "SELECT request_id,phase FROM main.gogoke_v37_instance_legacy_source_migrations",2).len(),1);
        let second_name=format!("Gogoke37.Unrecorded.{}",nonce);
        let unrecorded=crate::process::AppContainerProfile::derive_for_revocation(&second_name).unwrap();
        unrecorded.grant_bound_tree(&home_path,&home_identity,true).unwrap();
        let bound=crate::process::CredentialBinding::open_registered(&root,&auth_path,
            &home_identity,&crate::process::CredentialBinding::observe_source_metadata(
                &root,&auth_path,&home_identity).unwrap().0,&[]).unwrap();
        assert!(crate::process::LegacyAclInventory::capture(&root,&home_path,
            &home_identity,&bound,&[owner_name]).is_err(),
            "an unrecorded physical package SID cannot be omitted from the ACL proof");
        drop(bound);
        product.close_checked().unwrap();drop(root);
        fs::remove_file(database).unwrap();
        fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    fn real_opencode_31_stage(root:&RootLock)->(String,PathBuf) {
        real_opencode_version_stage(root,"1.18.31")
    }

    fn real_opencode_version_stage(root:&RootLock,version:&str)->(String,PathBuf) {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"),
            "two-version official CLI fixture is cloud-only");
        let pin=instance::read_official_cli_version("opencode",version).unwrap();
        let key=(pin.driver.to_owned(),pin.version.to_owned());
        let mut cache=PINNED_VERSION_ARCHIVES.get_or_init(||Mutex::new(HashMap::new())).lock().unwrap();
        let archive=cache.entry(key).or_insert_with(||{
            let folder=std::env::temp_dir().join(format!(
                "gogoke-opencode-two-pin-{}",std::process::id()));
            fs::create_dir_all(&folder).unwrap();
            let path=folder.join(format!("opencode-{version}.download"));
            let status=Command::new("curl.exe").args([
                "--fail","--location","--silent","--show-error","--output",
            ]).arg(&path).arg(format!("https://registry.npmjs.org/opencode-windows-x64/-/opencode-windows-x64-{version}.tgz"))
                .status().expect("fixed OpenCode version archive download");
            assert!(status.success(),"fixed OpenCode {version} archive download: {status}");
            path
        }).clone();
        drop(cache);
        assert_eq!(crate::store::digest::sha256_hex(&fs::read(&archive).unwrap()),
            pin.archive_sha256,"fixed OpenCode version archive pin");
        let staging=instance::managed_cli_root(root).unwrap();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let stage=format!("opencode-{version}-fixture-{nonce}");
        let folder=staging.join(&stage);
        let content=folder.join("content");
        fs::create_dir(&folder).unwrap();
        fs::create_dir(&content).unwrap();
        let source=folder.join("source.download");
        fs::copy(&archive,&source).unwrap();
        let status=Command::new("tar.exe").arg("-xzf").arg(&source).arg("-C").arg(&content)
            .status().expect("fixed OpenCode version archive extraction");
        assert!(status.success(),"fixed OpenCode {version} archive extraction: {status}");
        let image=instance::inspect_staged_official_cli_version(root,pin.driver,pin.version,&stage)
            .expect("native verified fixed OpenCode version archive and image");
        (stage,image)
    }

    fn ready_two_opencode_sources(product:&mut ProductDatabase<'_>,root:&RootLock)
        ->(String,PathBuf) {
        let (stage,image)=super::super::managed_cli_test_setup::prepared_image(root,"opencode");
        default_begin(product,root);
        super::super::managed_cli_test_setup::staged_probe(product,"opencode",&stage);
        let pin=instance::read_fixed_official_cli("opencode").unwrap();
        let observed=instance::ProgramObservation::observe(&image,pin.version).unwrap();
        for id in ["instanceA","instanceB"] {
            let request=format!("real-first-copy-{id}");
            instance::register_instance(&mut product.connection,root,&instance::Registration{
                request_id:&request,request_bytes:request.as_bytes(),instance_id:id,
                driver_id:"opencode",program:&observed,
            }).unwrap();
            let profile=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_profiles(instance_id,display_name,enabled,revision) VALUES(?1,?2,1,1)").unwrap();
            profile.bind_text(1,id).unwrap();profile.bind_text(2,id).unwrap();
            profile.step_done().unwrap();
        }
        let sentinel=root.canonical_root().canonical_path.join("v37-instances")
            .join("instanceA").join("f2-home-sentinel");
        fs::write(&sentinel,b"original registered HOME sentinel").unwrap();
        let migrate=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"migrate\",\"driverId\":\"opencode\",\"stageName\":\"{stage}\"}}");
        product.dispatch_user_managed_cli(migrate.as_bytes()).unwrap();
        product.connection.execute("UPDATE main.gogoke_v37_instance_profiles SET enabled=0 WHERE instance_id='instanceB'").unwrap();
        let copy=instance::read_managed_cli(&product.connection,root,"opencode").unwrap().unwrap();
        assert_eq!(copy.state,"READY");
        assert_eq!(copy.version.as_deref(),Some("1.18.32"));
        (stage,sentinel)
    }

    fn managed_user(product:&mut ProductDatabase<'_>,raw:&str)->Vec<u8> {
        product.dispatch_user_managed_cli(raw.as_bytes())
            .unwrap_or_else(|error|panic!("original managed USER frame {raw}: {error:?}"))
    }

    fn default_begin(product:&mut ProductDatabase<'_>,root:&RootLock) {
        assert!(instance::read_managed_cli(&product.connection,root,"opencode").unwrap().is_none(),
            "the first default begin must start without a copy row");
        managed_user(product,
            r#"{"schema":"gogoke.37.managed-cli.v1","command":"begin","driverId":"opencode"}"#);
        let copy=instance::read_managed_cli(&product.connection,root,"opencode").unwrap().unwrap();
        assert_eq!(copy.state,"DOWNLOADING");
        assert!(copy.version.is_none());
        assert!(copy.previous_version.is_none());
    }

    fn version_frame(command:&str,version:&str,stage:Option<&str>,request:Option<&str>)->String {
        let mut frame=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"{command}\",\"driverId\":\"opencode\",\"targetVersion\":\"{version}\"");
        if let Some(stage)=stage {frame.push_str(&format!(",\"stageName\":\"{stage}\""));}
        if let Some(request)=request {frame.push_str(&format!(",\"requestId\":\"{request}\""));}
        frame.push('}');
        frame
    }

    fn receipt_set(bytes:&[u8])->(String,Vec<String>) {
        let Json::Object(mut response)=Parser::parse(std::str::from_utf8(bytes).unwrap()).unwrap()
            else {panic!("switch response object")};
        let status=match response.remove(&key("status")).unwrap() {
            Json::String(value)=>value.to_well_formed_string().unwrap(),
            _=>panic!("switch status string"),
        };
        let Json::Array(receipts)=response.remove(&key("receipts")).unwrap()
            else {panic!("original per-instance receipts")};
        (status,receipts.iter().map(Json::canonical).collect())
    }

    #[derive(Debug,PartialEq,Eq)]
    struct ManagedSnapshot {
        copy:Vec<Vec<String>>,instances:Vec<Vec<String>>,profiles:Vec<Vec<String>>,
        sources:Vec<Vec<String>>,
        operations:Vec<Vec<String>>,custody:Vec<Vec<String>>,
    }
    fn table_rows(product:&ProductDatabase<'_>,sql:&str,columns:i32)->Vec<Vec<String>> {
        let rows=Statement::prepare(product.connection.as_ptr(),sql).unwrap();
        let mut result=Vec::new();
        while rows.step_row().unwrap() {
            result.push((0..columns).map(|index|rows.column_text(index).unwrap()).collect());
        }
        result
    }
    fn legacy_fence_schema_rows(product:&ProductDatabase<'_>)->Vec<Vec<String>> {
        table_rows(product,
            "SELECT 'main',type,name,tbl_name,COALESCE(sql,''),sql IS NULL FROM main.sqlite_schema WHERE lower(substr(name,1,22))='gogoke_v37_legacy_acl_' OR lower(substr(tbl_name,1,22))='gogoke_v37_legacy_acl_' UNION ALL SELECT 'temp',type,name,tbl_name,COALESCE(sql,''),sql IS NULL FROM temp.sqlite_schema WHERE lower(substr(name,1,22))='gogoke_v37_legacy_acl_' OR lower(substr(tbl_name,1,22))='gogoke_v37_legacy_acl_' ORDER BY 1,3",6)
    }
    fn legacy_fence_family_rows(product:&ProductDatabase<'_>,present:bool)
        ->Vec<Vec<Vec<String>>> {
        if !present {return Vec::new()}
        vec![
            table_rows(product,
                "SELECT instance_id,boot_identity,database_identity,root_identity,home_identity,source_identity,source_parent_identity,COALESCE(CAST(source_revision AS TEXT),''),custody_digest,home_acl_digest,source_acl_digest,acl_provenance_digest,native_snapshot,native_snapshot_digest,revision FROM main.gogoke_v37_legacy_acl_fences ORDER BY instance_id",15),
            table_rows(product,
                "SELECT instance_id,operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,COALESCE(stop_proof_hash,''),stop_proof_hash IS NULL FROM main.gogoke_v37_legacy_acl_custody ORDER BY instance_id,operation_id",14),
            table_rows(product,
                "SELECT instance_id,step,phase,request_id,boot_identity,before_home_acl_digest,before_source_acl_digest,target_home_acl_digest,target_source_acl_digest,intent_revision,COALESCE(CAST(applied_revision AS TEXT),'') FROM main.gogoke_v37_legacy_acl_steps ORDER BY instance_id,step",11),
        ]
    }
    fn managed_snapshot(product:&ProductDatabase<'_>)->ManagedSnapshot {
        ManagedSnapshot{
            copy:table_rows(product,"SELECT driver_id,state,COALESCE(version,''),COALESCE(archive_sha256,''),COALESCE(image_sha256,''),COALESCE(stage_name,''),COALESCE(previous_version,''),COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,''),revision,COALESCE(raw_error,'') FROM main.gogoke_v37_instance_cli_copies ORDER BY driver_id",11),
            instances:table_rows(product,"SELECT instance_id,version,program_digest,home_identity,install_state,login_state,revision FROM main.gogoke_v37_instances ORDER BY instance_id",7),
            profiles:table_rows(product,"SELECT instance_id,display_name,enabled,COALESCE(connected_model_source,''),tombstoned,revision FROM main.gogoke_v37_instance_profiles ORDER BY instance_id",6),
            sources:table_rows(product,"SELECT instance_id,stage_name,version,program_digest,home_identity,registration_request_id,revision FROM main.gogoke_v37_instance_program_sources ORDER BY instance_id",7),
            operations:table_rows(product,"SELECT request_id,request_hex,target_id,phase,COALESCE(receipt_json,''),COALESCE(native_receipt_id,'') FROM main.gogoke_v37_instance_operations ORDER BY request_id",6),
            custody:table_rows(product,"SELECT operation_id,state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody ORDER BY operation_id",3),
        }
    }

    #[test]
    fn default_begin_rejects_previous_nondefault_and_existing_source_without_effects() {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-default-begin-boundary-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let begin=br#"{"schema":"gogoke.37.managed-cli.v1","command":"begin","driverId":"opencode"}"#;
        let default=instance::read_fixed_official_cli("opencode").unwrap();
        let old=instance::read_official_cli_version("opencode","1.18.31").unwrap();

        let previous=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,previous_version,previous_image_sha256,previous_stage_name,revision) VALUES('opencode','NOT_INSTALLED',?1,?2,'opencode-1.18.31-historical',1)").unwrap();
        previous.bind_text(1,old.version).unwrap();
        previous.bind_text(2,old.image_sha256).unwrap();
        previous.step_done().unwrap();drop(previous);
        let before=managed_snapshot(&product);
        assert!(product.dispatch_user_managed_cli(begin).is_err());
        assert_eq!(managed_snapshot(&product),before);
        product.connection.execute("DELETE FROM main.gogoke_v37_instance_cli_copies WHERE driver_id='opencode'").unwrap();

        let insert=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','opencode','homeA','identityA',?1,?2,'UNKNOWN','UNKNOWN',1)").unwrap();
        insert.bind_text(1,&format!("sha256:{}",old.image_sha256)).unwrap();
        insert.bind_text(2,old.version).unwrap();
        insert.step_done().unwrap();drop(insert);
        let before=managed_snapshot(&product);
        assert!(product.dispatch_user_managed_cli(begin).is_err());
        assert_eq!(managed_snapshot(&product),before);

        let update=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instances SET version=?1,program_digest=?2 WHERE instance_id='instanceA'").unwrap();
        update.bind_text(1,default.version).unwrap();
        update.bind_text(2,&format!("sha256:{}",default.image_sha256)).unwrap();
        update.step_done().unwrap();drop(update);
        let source=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_program_sources(instance_id,source,stage_name,program_digest,version,home_identity,registration_request_id,revision) VALUES('instanceA','MANAGED','opencode-1.18.32-existing',?1,?2,'identityA','original-creation',1)").unwrap();
        source.bind_text(1,&format!("sha256:{}",default.image_sha256)).unwrap();
        source.bind_text(2,default.version).unwrap();
        source.step_done().unwrap();drop(source);
        let before=managed_snapshot(&product);
        assert!(product.dispatch_user_managed_cli(begin).is_err());
        assert_eq!(managed_snapshot(&product),before);
        product.close_checked().unwrap();
        drop(root);
        fs::remove_file(database).unwrap();
        fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn real_current_default_upgrades_to_fixed33_and_rolls_back_to_original32() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"),
            "fixed higher-version USER lifecycle test is cloud-only");
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-fixed33-user-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let (stage32,sentinel)=ready_two_opencode_sources(&mut product,&root);
        let original_home=fs::read(&sentinel).unwrap();
        let baseline=managed_snapshot(&product);
        let copy=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert!(copy.previous_version.is_none());
        assert!(copy.verified_other_versions.iter().any(|version|version=="1.18.33"));
        let (stage33,_image33)=real_opencode_version_stage(&root,"1.18.33");
        managed_user(&mut product,&version_frame("begin","1.18.33",None,None));
        managed_user(&mut product,&version_frame("stage","1.18.33",Some(&stage33),None));
        managed_user(&mut product,&version_frame("probe","1.18.33",Some(&stage33),Some("probe.33/actual:1")));
        let probed=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(probed.state,"PROBED");
        let upgrade_raw=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"upgrade\",\"requestId\":\"upgrade.33/actual:1\",\"driverId\":\"opencode\",\"targetVersion\":\"1.18.33\",\"expectedRevision\":\"{}\"}}",probed.revision);
        let (status,original_upgrade)=receipt_set(&managed_user(&mut product,&upgrade_raw));
        assert_eq!(status,"APPLIED");
        assert_eq!(original_upgrade.len(),2);
        let upgraded=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(upgraded.version.as_deref(),Some("1.18.33"));
        assert_eq!(upgraded.previous_version.as_deref(),Some("1.18.32"));
        let upgrade_snapshot=managed_snapshot(&product);
        assert!(upgrade_snapshot.instances.iter().all(|row|row[1]=="1.18.33"));
        assert!(upgrade_snapshot.sources.iter().all(|row|row[2]=="1.18.33"));
        assert_eq!(upgrade_snapshot.profiles,baseline.profiles);
        assert_eq!(fs::read(&sentinel).unwrap(),original_home);
        managed_user(&mut product,&version_frame("begin","1.18.32",None,None));
        managed_user(&mut product,&version_frame("stage","1.18.32",Some(&stage32),None));
        managed_user(&mut product,&version_frame("probe","1.18.32",Some(&stage32),Some("probe.32/rollback:1")));
        let probed=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        let rollback_raw=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"rollback\",\"requestId\":\"rollback.32/actual:1\",\"driverId\":\"opencode\",\"targetVersion\":\"1.18.32\",\"expectedRevision\":\"{}\"}}",probed.revision);
        let (status,original_rollback)=receipt_set(&managed_user(&mut product,&rollback_raw));
        assert_eq!(status,"APPLIED");
        assert_eq!(original_rollback.len(),2);
        let rolled_back=managed_snapshot(&product);
        assert!(rolled_back.instances.iter().all(|row|row[1]=="1.18.32"));
        assert!(rolled_back.sources.iter().all(|row|row[2]=="1.18.32"));
        assert_eq!(rolled_back.profiles,baseline.profiles);
        for (before,after) in baseline.instances.iter().zip(&rolled_back.instances) {
            assert_eq!((&before[0],&before[3]),(&after[0],&after[3]));
        }
        assert_eq!(fs::read(&sentinel).unwrap(),original_home);
        let (status,replayed)=receipt_set(&managed_user(&mut product,&upgrade_raw));
        assert_eq!(status,"REPLAYED");
        assert_eq!(replayed,original_upgrade);
        assert_eq!(managed_snapshot(&product),rolled_back);
        let (status,replayed)=receipt_set(&managed_user(&mut product,&rollback_raw));
        assert_eq!(status,"REPLAYED");
        assert_eq!(replayed,original_rollback);
        assert_eq!(managed_snapshot(&product),rolled_back);
        product.close_checked().unwrap();
        drop(root);
        fs::remove_file(database).unwrap();
        fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn real_bound_version_action_replays_without_relaunch_and_rollback_uses_new_action() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"),"official CLI lifecycle is cloud-only");
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-bound-version-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let (stage32,sentinel)=ready_two_opencode_sources(&mut product,&root);
        let original_home=fs::read(&sentinel).unwrap();
        let original_profiles=managed_snapshot(&product).profiles;
        let (stage33,image33)=real_opencode_version_stage(&root,"1.18.33");
        let begin=|id:&str,version:&str,operation:&str|format!(
            "{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"begin\",\"driverId\":\"opencode\",\"targetVersion\":\"{version}\",\"requestId\":\"{id}\",\"operation\":\"{operation}\"}}");
        let external_failure=|id:&str,state:&str,raw:&str|format!(
            "{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"failure\",\"driverId\":\"opencode\",\"requestId\":\"{id}\",\"state\":\"{state}\",\"raw\":\"{raw}\"}}");
        managed_user(&mut product,&begin("download-failure","1.18.33","upgrade"));
        let download_failure=external_failure("download-failure","UPGRADE_FAILED","original downloader error before stage");
        let original_failure_reply=managed_user(&mut product,&download_failure);
        let failed_download=instance::read_version_action(&product.connection,&root,"download-failure").unwrap().unwrap();
        assert_eq!(failed_download.phase,instance::ActionPhase::Failed);
        assert!(failed_download.open_driver_id.is_empty());
        let after_download_failure=managed_snapshot(&product);
        assert_eq!(managed_user(&mut product,&download_failure),original_failure_reply);
        assert_eq!(managed_snapshot(&product),after_download_failure);
        assert!(product.dispatch_user_managed_cli(external_failure("download-failure","UPGRADE_FAILED","different error").as_bytes()).is_err());
        assert_eq!(instance::read_version_action(&product.connection,&root,"download-failure").unwrap().unwrap(),failed_download);
        assert_eq!(managed_snapshot(&product),after_download_failure);
        managed_user(&mut product,&begin("failed-before-launch","1.18.33","upgrade"));
        managed_user(&mut product,&version_frame("stage","1.18.33",Some(&stage33),Some("failed-before-launch")));
        let probe_home=image33.ancestors().find(|path|path.file_name().and_then(|name|name.to_str())==Some("content"))
            .unwrap().parent().unwrap().join("probe-home");
        fs::write(&probe_home,b"known non-directory fixture before any H prepare").unwrap();
        let before_failure_c=table_rows(&product,"SELECT operation_id,state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody ORDER BY operation_id",3);
        let failed=version_frame("probe","1.18.33",Some(&stage33),Some("failed-before-launch"));
        assert!(product.dispatch_user_managed_cli(failed.as_bytes()).is_err());
        let failed_action=instance::read_version_action(&product.connection,&root,"failed-before-launch").unwrap().unwrap();
        assert_eq!(failed_action.phase,instance::ActionPhase::Failed);
        assert!(failed_action.open_driver_id.is_empty());assert!(!failed_action.original_error_hex.is_empty());
        let blocked=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(blocked.state,"BLOCKED");
        assert!(blocked.raw_error.as_deref().unwrap().contains("private probe home identity"));
        assert_eq!(table_rows(&product,"SELECT operation_id,state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody ORDER BY operation_id",3),before_failure_c);
        fs::remove_file(probe_home).unwrap();
        let up_begin=begin("bound-upgrade","1.18.33","upgrade");
        let begun=managed_user(&mut product,&up_begin);
        let after_begin=managed_snapshot(&product);
        assert_eq!(managed_user(&mut product,&up_begin),begun);
        assert_eq!(managed_snapshot(&product),after_begin);
        for foreign in [begin("other-id","1.18.33","upgrade"),begin("bound-upgrade","1.18.33","rollback"),
            version_frame("begin","1.18.33",None,None),
            version_frame("stage","1.18.33",Some(&stage33),Some("other-id")),
            version_frame("probe","1.18.33",Some(&stage33),Some("other-id"))] {
            assert!(product.dispatch_user_managed_cli(foreign.as_bytes()).is_err());
            assert_eq!(managed_snapshot(&product),after_begin);
        }
        let stage=version_frame("stage","1.18.33",Some(&stage33),Some("bound-upgrade"));
        let staged=managed_user(&mut product,&stage);
        let after_stage=managed_snapshot(&product);
        assert_eq!(managed_user(&mut product,&stage),staged);
        assert_eq!(managed_snapshot(&product),after_stage);
        let staged_action=instance::read_version_action(&product.connection,&root,"bound-upgrade").unwrap().unwrap();
        assert!(product.dispatch_user_managed_cli(external_failure("bound-upgrade","UPGRADE_FAILED","late downloader error").as_bytes()).is_err());
        assert_eq!(managed_snapshot(&product),after_stage);
        assert_eq!(instance::read_version_action(&product.connection,&root,"bound-upgrade").unwrap().unwrap(),staged_action);
        let probe=version_frame("probe","1.18.33",Some(&stage33),Some("bound-upgrade"));
        let probed_reply=managed_user(&mut product,&probe);
        let custody=table_rows(&product,"SELECT operation_id,state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody ORDER BY operation_id",3);
        let after_probe=managed_snapshot(&product);
        assert_eq!(managed_user(&mut product,&probe),probed_reply);
        assert_eq!(managed_snapshot(&product),after_probe);
        let probed_action=instance::read_version_action(&product.connection,&root,"bound-upgrade").unwrap().unwrap();
        // Begin with the actual successful native probe above. Preserve exact
        // SQL values and NULLs, then change one durable C field at a time.
        let probe_prefix=format!("managed-probe-opencode-{}-*",
            &crate::store::digest::sha256_hex(b"bound-upgrade")[..40]);
        let backup=Statement::prepare(product.connection.as_ptr(),
            "CREATE TEMP TABLE bound_probe_restore AS SELECT * FROM main.gogoke_coordination_process_custody WHERE operation_id GLOB ?1").unwrap();
        backup.bind_text(1,&probe_prefix).unwrap();backup.step_done().unwrap();drop(backup);
        assert_eq!(table_rows(&product,"SELECT CAST(COUNT(*) AS TEXT) FROM temp.bound_probe_restore",1),
            vec![vec!["1".to_owned()]]);
        assert_eq!(managed_user(&mut product,&probe),probed_reply,
            "the exact SQL backup must not make the positive replay fail");
        for alteration in [
            None,
            Some("ticket='changed-original-ticket'"),
            Some("custodian_nonce='changed-original-nonce'"),
            Some("pid=CAST(CAST(pid AS INTEGER)+1 AS TEXT)"),
            Some("creation_time_100ns=CAST(CAST(creation_time_100ns AS INTEGER)+1 AS TEXT)"),
            Some("generation=CAST(CAST(generation AS INTEGER)+1 AS TEXT)"),
            Some("stop_proof_hash='sha256:0000000000000000000000000000000000000000000000000000000000000000'"),
        ] {
            let sql=alteration.map_or_else(||
                "DELETE FROM main.gogoke_coordination_process_custody WHERE operation_id GLOB ?1".to_owned(),
                |set|format!("UPDATE main.gogoke_coordination_process_custody SET {set} WHERE operation_id GLOB ?1"));
            let change=Statement::prepare(product.connection.as_ptr(),&sql).unwrap();
            change.bind_text(1,&probe_prefix).unwrap();change.step_done().unwrap();drop(change);
            assert!(product.dispatch_user_managed_cli(probe.as_bytes()).is_err(),
                "a PROBED receipt cannot replace its missing or changed original C: {alteration:?}");
            assert_eq!(instance::read_version_action(&product.connection,&root,"bound-upgrade").unwrap().unwrap(),probed_action);
            let remove=Statement::prepare(product.connection.as_ptr(),
                "DELETE FROM main.gogoke_coordination_process_custody WHERE operation_id GLOB ?1").unwrap();
            remove.bind_text(1,&probe_prefix).unwrap();remove.step_done().unwrap();drop(remove);
            product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody SELECT * FROM temp.bound_probe_restore").unwrap();
            assert_eq!(managed_user(&mut product,&probe),probed_reply,
                "restoring the exact C must restore the positive native-probe replay");
        }
        assert!(product.dispatch_user_managed_cli(external_failure("bound-upgrade","BLOCKED","late caller error").as_bytes()).is_err());
        assert_eq!(managed_snapshot(&product),after_probe);
        assert_eq!(instance::read_version_action(&product.connection,&root,"bound-upgrade").unwrap().unwrap(),probed_action);
        assert_eq!(table_rows(&product,"SELECT operation_id,state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody ORDER BY operation_id",3),custody);
        let copy=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        let final_frame=|operation:&str,id:&str,target:&str,revision:i64|format!(
            "{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"{operation}\",\"driverId\":\"opencode\",\"targetVersion\":\"{target}\",\"requestId\":\"{id}\",\"expectedRevision\":\"{revision}\"}}");
        assert!(product.dispatch_user_managed_cli(final_frame("upgrade","other-id","1.18.33",copy.revision).as_bytes()).is_err());
        let up=final_frame("upgrade","bound-upgrade","1.18.33",copy.revision);
        let (status,receipts)=receipt_set(&managed_user(&mut product,&up));
        assert_eq!(status,"APPLIED");assert_eq!(receipts.len(),2);
        let upgraded=managed_snapshot(&product);
        let remove=Statement::prepare(product.connection.as_ptr(),
            "DELETE FROM main.gogoke_coordination_process_custody WHERE operation_id GLOB ?1").unwrap();
        remove.bind_text(1,&probe_prefix).unwrap();remove.step_done().unwrap();drop(remove);
        let missing_probe_snapshot=managed_snapshot(&product);
        assert!(product.dispatch_user_managed_cli(up.as_bytes()).is_err(),
            "a CONSUMED receipt must still bind the original successful probe C");
        assert_eq!(managed_snapshot(&product),missing_probe_snapshot);
        product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody SELECT * FROM temp.bound_probe_restore").unwrap();
        product.connection.execute("DROP TABLE temp.bound_probe_restore").unwrap();
        assert_eq!(managed_snapshot(&product),upgraded);
        let (status,replayed)=receipt_set(&managed_user(&mut product,&up));
        assert_eq!(status,"REPLAYED");assert_eq!(replayed,receipts);
        assert_eq!(managed_snapshot(&product),upgraded);
        assert_eq!(instance::read_version_action(&product.connection,&root,"bound-upgrade").unwrap().unwrap().phase,
            instance::ActionPhase::Consumed);
        product.close_checked().unwrap();
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        managed_user(&mut product,&begin("bound-rollback","1.18.32","rollback"));
        managed_user(&mut product,&version_frame("stage","1.18.32",Some(&stage32),Some("bound-rollback")));
        managed_user(&mut product,&version_frame("probe","1.18.32",Some(&stage32),Some("bound-rollback")));
        let copy=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        let rollback=final_frame("rollback","bound-rollback","1.18.32",copy.revision);
        assert_eq!(receipt_set(&managed_user(&mut product,&rollback)).0,"APPLIED");
        assert_eq!(receipt_set(&managed_user(&mut product,&rollback)).0,"REPLAYED");
        assert_eq!(fs::read(&sentinel).unwrap(),original_home);
        let after=managed_snapshot(&product);
        assert_eq!(after.profiles,original_profiles);
        assert!(after.instances.iter().all(|row|row[1]=="1.18.32"));
        assert!(after.sources.iter().all(|row|row[2]=="1.18.32"));
        // An older action cannot silently adopt the later action's copy/source
        // revision chain, even for a readback of its original final bytes.
        assert!(product.dispatch_user_managed_cli(up.as_bytes()).is_err());
        assert_eq!(managed_snapshot(&product),after);
        // Model a crash immediately after the real production intent COMMIT,
        // before any H launch. A later public error cannot assert that fact or
        // release this unresolved action; replay must not launch a new probe.
        managed_user(&mut product,&begin("held-probe-intent","1.18.33","upgrade"));
        managed_user(&mut product,&version_frame("stage","1.18.33",Some(&stage33),Some("held-probe-intent")));
        let held_probe=version_frame("probe","1.18.33",Some(&stage33),Some("held-probe-intent"));
        let held_hex=action_hex(held_probe.as_bytes());
        let held=product.version_action_transaction(|this| {
            let row=this.bound_version_action("opencode","1.18.33","held-probe-intent")?;
            this.managed_version_action_resources("opencode",Some(&row))?;
            instance::begin_probe_version_action_in_current_transaction(&this.connection,this.root,&this.owner,
                &row.request_id,row.revision,&held_hex,&stage33,&row.current_copy_fingerprint,&row.current_source_fingerprint)
                .map_err(|error|failure("test original prelaunch intent",error))
        }).unwrap();
        let instance::BeginOutcome::Created(held)=held else {panic!("new original probe intent")};
        assert_eq!(held.phase,instance::ActionPhase::Probing);
        let unresolved=managed_snapshot(&product);
        for state in ["UPGRADE_FAILED","BLOCKED","PROBE_UNKNOWN"] {
            assert!(product.dispatch_user_managed_cli(external_failure("held-probe-intent",state,"caller has no native outcome").as_bytes()).is_err());
            assert_eq!(managed_snapshot(&product),unresolved);
            assert_eq!(instance::read_version_action(&product.connection,&root,"held-probe-intent").unwrap().unwrap(),held);
        }
        assert!(product.dispatch_user_managed_cli(begin("replacement-for-held","1.18.33","upgrade").as_bytes()).is_err());
        assert!(product.dispatch_user_managed_cli(held_probe.as_bytes()).is_err());
        assert_eq!(managed_snapshot(&product),unresolved);
        assert_eq!(instance::read_version_action(&product.connection,&root,"held-probe-intent").unwrap().unwrap(),held);
        product.close_checked().unwrap();drop(root);fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn real_two_pin_user_switch_replays_original_receipts_without_new_effects() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"),
            "real two-pin USER lifecycle test is cloud-only");
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-two-pin-user-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let (stage32,sentinel)=ready_two_opencode_sources(&mut product,&root);
        let home_before=fs::read(&sentinel).unwrap();
        let baseline=managed_snapshot(&product);
        assert_eq!(baseline.profiles.len(),2);
        assert_eq!(baseline.profiles[1][2],"0");
        let (stage31,_image31)=real_opencode_31_stage(&root);
        managed_user(&mut product,&version_frame("begin","1.18.31",None,None));
        managed_user(&mut product,&version_frame("stage","1.18.31",Some(&stage31),None));
        managed_user(&mut product,&version_frame("probe","1.18.31",Some(&stage31),Some("probe.31/actual:1")));
        let probed=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(probed.state,"PROBED");
        assert_eq!(probed.previous_version.as_deref(),Some("1.18.32"));
        let rollback_id="rollback.31/actual:1";
        let rollback_raw=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"rollback\",\"requestId\":\"{rollback_id}\",\"driverId\":\"opencode\",\"targetVersion\":\"1.18.31\",\"expectedRevision\":\"{}\"}}",probed.revision);
        let (first_status,originals)=receipt_set(&managed_user(&mut product,&rollback_raw));
        assert_eq!(first_status,"APPLIED");
        assert_eq!(originals.len(),2);
        assert!(originals.iter().any(|value|value.contains("instanceA")));
        assert!(originals.iter().any(|value|value.contains("instanceB")));
        let ready31=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!((ready31.state.as_str(),ready31.version.as_deref()),("READY",Some("1.18.31")));
        let applied=managed_snapshot(&product);
        assert_eq!(applied.instances.len(),2);
        assert_eq!(applied.sources.len(),2);
        assert!(applied.instances.iter().all(|row|row[1]=="1.18.31"));
        assert!(applied.sources.iter().all(|row|row[2]=="1.18.31"));
        for (before,after) in baseline.instances.iter().zip(&applied.instances) {
            assert_eq!((&before[0],&before[3]),(&after[0],&after[3]));
            assert_eq!(after[5],"UNKNOWN");
        }
        assert_eq!(fs::read(&sentinel).unwrap(),home_before);
        assert_eq!(applied.profiles,baseline.profiles);
        assert!(baseline.operations.iter().all(|row|applied.operations.contains(row)));
        let (replay_status,replayed)=receipt_set(&managed_user(&mut product,&rollback_raw));
        assert_eq!(replay_status,"REPLAYED");
        assert_eq!(replayed,originals);
        assert_eq!(managed_snapshot(&product),applied);
        let changed=format!(" {rollback_raw}");
        assert!(product.dispatch_user_managed_cli(changed.as_bytes()).is_err());
        assert_eq!(managed_snapshot(&product),applied);
        product.close_checked().unwrap();
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let (reopen_status,reopened_receipts)=receipt_set(&managed_user(&mut product,&rollback_raw));
        assert_eq!(reopen_status,"REPLAYED");
        assert_eq!(reopened_receipts,originals);
        assert_eq!(managed_snapshot(&product),applied);
        assert_eq!(fs::read(&sentinel).unwrap(),home_before);

        managed_user(&mut product,&version_frame("begin","1.18.32",None,None));
        managed_user(&mut product,&version_frame("stage","1.18.32",Some(&stage32),None));
        managed_user(&mut product,&version_frame("probe","1.18.32",Some(&stage32),Some("probe.32/actual:2")));
        let probed=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(probed.state,"PROBED");
        let upgrade_raw=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"upgrade\",\"requestId\":\"upgrade.32/actual:2\",\"driverId\":\"opencode\",\"targetVersion\":\"1.18.32\",\"expectedRevision\":\"{}\"}}",probed.revision);
        let (upgrade_status,upgrade_receipts)=receipt_set(&managed_user(&mut product,&upgrade_raw));
        assert_eq!(upgrade_status,"APPLIED");
        assert_eq!(upgrade_receipts.len(),2);
        let after_upgrade=managed_snapshot(&product);
        assert!(after_upgrade.instances.iter().all(|row|row[1]=="1.18.32"));
        assert!(after_upgrade.sources.iter().all(|row|row[2]=="1.18.32"));
        for (before,after) in baseline.instances.iter().zip(&after_upgrade.instances) {
            assert_eq!((&before[0],&before[3]),(&after[0],&after[3]));
            assert_eq!(after[5],"UNKNOWN");
        }
        assert_eq!(after_upgrade.profiles,baseline.profiles);
        assert_eq!(fs::read(&sentinel).unwrap(),home_before);
        let (old_status,old_receipts)=receipt_set(&managed_user(&mut product,&rollback_raw));
        assert_eq!(old_status,"REPLAYED");
        assert_eq!(old_receipts,originals);
        assert_eq!(managed_snapshot(&product),after_upgrade);

        use sha2::Digest;
        let witness_key=format!("managed-switch-{:x}",sha2::Sha256::digest(
            format!("{rollback_id}\0instanceA").as_bytes()));
        let corrupt=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_operations SET receipt_json='{}' WHERE request_id=?1").unwrap();
        corrupt.bind_text(1,&witness_key).unwrap();corrupt.step_done().unwrap();drop(corrupt);
        let damaged=managed_snapshot(&product);
        assert!(product.dispatch_user_managed_cli(rollback_raw.as_bytes()).is_err());
        assert_eq!(managed_snapshot(&product),damaged);
        product.close_checked().unwrap();
        drop(root);
        fs::remove_file(database).unwrap();
        fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn real_probe_custody_then_settled_failure_reuses_stage_and_unknown_stays_closed() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"),
            "real two-pin probe test is cloud-only");
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-two-pin-retry-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let (stage32,sentinel)=ready_two_opencode_sources(&mut product,&root);
        let home_before=fs::read(&sentinel).unwrap();
        let (stage31,_image31)=real_opencode_31_stage(&root);
        let begin31=version_frame("begin","1.18.31",None,None);
        let stage31_frame=version_frame("stage","1.18.31",Some(&stage31),None);
        let probe31=version_frame("probe","1.18.31",Some(&stage31),Some("probe.31/retry:1"));
        managed_user(&mut product,&begin31);
        managed_user(&mut product,&stage31_frame);
        managed_user(&mut product,&probe31); // Original H process and durable StopFact.
        let first_probed=managed_snapshot(&product);
        let failure=br#"{"schema":"gogoke.37.managed-cli.v1","command":"failure","driverId":"opencode","state":"BLOCKED","raw":"controlled settled failure after actual H stop"}"#;
        managed_user(&mut product,std::str::from_utf8(failure).unwrap());
        let blocked=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(blocked.state,"BLOCKED");
        assert_eq!(blocked.previous_version.as_deref(),Some("1.18.32"));
        assert_eq!(blocked.stage_name.as_deref(),Some(stage31.as_str()));
        let failed=managed_snapshot(&product);
        assert_eq!(failed.custody,first_probed.custody);
        let default_begin=br#"{"schema":"gogoke.37.managed-cli.v1","command":"begin","driverId":"opencode"}"#;
        assert!(product.dispatch_user_managed_cli(default_begin).is_err());
        let default_stage=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"stage\",\"driverId\":\"opencode\",\"stageName\":\"{stage32}\"}}");
        assert!(product.dispatch_user_managed_cli(default_stage.as_bytes()).is_err());
        assert_eq!(managed_snapshot(&product),failed);
        managed_user(&mut product,&begin31);
        let retry_begin=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(retry_begin.raw_error.as_deref(),Some("controlled settled failure after actual H stop"));
        managed_user(&mut product,&stage31_frame); // Reuses original verified target stage.
        let restaged=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(restaged.state,"STAGED");
        assert_eq!(restaged.previous_version.as_deref(),Some("1.18.32"));
        managed_user(&mut product,&probe31); // A second actual H/StopFact, not a fake probe.
        let reprobed=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(reprobed.state,"PROBED");
        assert_eq!(managed_snapshot(&product).custody.len(),failed.custody.len()+1);
        let unknown=br#"{"schema":"gogoke.37.managed-cli.v1","command":"failure","driverId":"opencode","state":"PROBE_UNKNOWN","raw":"controlled unresolved outcome"}"#;
        managed_user(&mut product,std::str::from_utf8(unknown).unwrap());
        let unknown_snapshot=managed_snapshot(&product);
        assert!(product.dispatch_user_managed_cli(begin31.as_bytes()).is_err());
        assert!(product.dispatch_user_managed_cli(stage31_frame.as_bytes()).is_err());
        assert!(product.dispatch_user_managed_cli(probe31.as_bytes()).is_err());
        assert_eq!(managed_snapshot(&product),unknown_snapshot);
        assert_eq!(fs::read(&sentinel).unwrap(),home_before);
        product.close_checked().unwrap();
        drop(root);
        fs::remove_file(database).unwrap();
        fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn switch_request_id_uses_the_original_f_witness_grammar() {
        for id in ["switch.1","switch:1","switch/1"] {
            let raw=format!("{{\"requestId\":\"{id}\"}}");
            let Json::Object(fields)=Parser::parse(&raw).unwrap() else {panic!("object")};
            assert_eq!(request_id(&fields).unwrap(),id);
            assert!(instance::valid_managed_switch_request_id(id));
        }
        for id in ["", "bad space", "bad%id"] {
            let raw=format!("{{\"requestId\":\"{id}\"}}");
            if let Ok(Json::Object(fields))=Parser::parse(&raw) {
                assert!(request_id(&fields).is_err());
            }
        }
    }

    #[test]
    fn original_switch_ids_reach_the_user_bridge_before_copy_refusal() {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-switch-id-bridge-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        for id in ["switch.1","switch:1","switch/1"] {
            let raw=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"rollback\",\"requestId\":\"{id}\",\"driverId\":\"opencode\",\"targetVersion\":\"1.18.31\",\"expectedRevision\":\"7\"}}");
            assert!(matches!(product.dispatch_user_managed_cli(raw.as_bytes()),
                Err(OrchestrationError::AccessDenied)));
        }
        let invalid=br#"{"schema":"gogoke.37.managed-cli.v1","command":"rollback","requestId":"bad%id","driverId":"opencode","targetVersion":"1.18.31","expectedRevision":"7"}"#;
        assert!(matches!(product.dispatch_user_managed_cli(invalid),
            Err(OrchestrationError::Invalid("managed CLI request"))));
        let status=br#"{"schema":"gogoke.37.managed-cli.v1","command":"status","driverId":"opencode"}"#;
        let Json::Object(empty)=Parser::parse(&String::from_utf8(
            product.dispatch_user_managed_cli(status).unwrap()).unwrap()).unwrap()
            else {panic!("status object")};
        assert!(matches!(empty.get(&key("preparedReplacement")),Some(Json::Null)));
        assert!(matches!(empty.get(&key("progressBytes")),Some(Json::Null)));
        let pin=instance::read_fixed_official_cli("opencode").unwrap();
        let copy=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,stage_name,progress_bytes,revision) VALUES('opencode','STAGED',?1,?2,?3,'opencode-1.18.32-first',123,1)").unwrap();
        copy.bind_text(1,pin.version).unwrap();
        copy.bind_text(2,pin.archive_sha256).unwrap();
        copy.bind_text(3,pin.image_sha256).unwrap();copy.step_done().unwrap();drop(copy);
        let Json::Object(staged)=Parser::parse(&String::from_utf8(
            product.dispatch_user_managed_cli(status).unwrap()).unwrap()).unwrap()
            else {panic!("status object")};
        assert!(matches!(staged.get(&key("preparedReplacement")),Some(Json::Null)));
        assert!(matches!(staged.get(&key("progressBytes")),Some(Json::Number(value)) if value=="123"));
        assert!(matches!(staged.get(&key("stageName")),Some(Json::String(value))
            if value.to_well_formed_string().as_deref()==Some("opencode-1.18.32-first")));
        product.close_checked().unwrap();
        drop(root);
        fs::remove_file(database).unwrap();
        fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn real_default_migration_keeps_disabled_legacy_unbound_and_status_readable() {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-first-copy-mixed-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let (stage,image)=super::super::managed_cli_test_setup::prepared_image(&root,"opencode");
        let pin=instance::read_fixed_official_cli("opencode").unwrap();
        let observed=instance::ProgramObservation::observe(&image,pin.version).unwrap();
        for (id,request,enabled) in [
            ("enabledA","first-copy-enabled",1),("disabledB","first-copy-disabled",0),
        ] {
            instance::register_instance(&mut product.connection,&root,&instance::Registration{
                request_id:request,request_bytes:request.as_bytes(),instance_id:id,
                driver_id:"opencode",program:&observed,
            }).unwrap();
            let profile=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_profiles(instance_id,display_name,enabled,revision) VALUES(?1,?2,?3,1)").unwrap();
            profile.bind_text(1,id).unwrap();profile.bind_text(2,id).unwrap();
            profile.bind_i64(3,enabled).unwrap();profile.step_done().unwrap();
        }
        assert!(instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().is_none());
        super::super::managed_cli_test_setup::staged_probe(&mut product,"opencode",&stage);
        let migrate=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"migrate\",\"driverId\":\"opencode\",\"stageName\":\"{stage}\"}}");
        product.dispatch_user_managed_cli(migrate.as_bytes()).unwrap();
        let bound=Statement::prepare(product.connection.as_ptr(),
            "SELECT instance_id FROM main.gogoke_v37_instance_program_sources ORDER BY instance_id").unwrap();
        assert!(bound.step_row().unwrap());
        assert_eq!(bound.column_text(0).unwrap(),"enabledA");
        assert!(!bound.step_row().unwrap());drop(bound);
        let status=br#"{"schema":"gogoke.37.managed-cli.v1","command":"status","driverId":"opencode"}"#;
        let Json::Object(fields)=Parser::parse(&String::from_utf8(
            product.dispatch_user_managed_cli(status).unwrap()).unwrap()).unwrap()
            else {panic!("status object")};
        assert!(matches!(fields.get(&key("state")),Some(Json::String(value))
            if value.to_well_formed_string().as_deref()==Some("READY")));
        assert!(matches!(fields.get(&key("preparedReplacement")),Some(Json::Null)));
        assert!(matches!(fields.get(&key("progressBytes")),Some(Json::Number(value))
            if value.parse::<i64>().is_ok_and(|bytes|bytes>0)));
        product.connection.execute("UPDATE main.gogoke_v37_instance_profiles SET enabled=1 WHERE instance_id='disabledB'").unwrap();
        assert!(product.dispatch_user_managed_cli(status).is_err());
        product.connection.execute("UPDATE main.gogoke_v37_instance_profiles SET enabled=0 WHERE instance_id='disabledB'").unwrap();
        product.connection.execute("UPDATE main.gogoke_v37_instances SET version='corrupt' WHERE instance_id='disabledB'").unwrap();
        assert!(product.dispatch_user_managed_cli(status).is_err());
        let restore=Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instances SET version=?1 WHERE instance_id='disabledB'").unwrap();
        restore.bind_text(1,pin.version).unwrap();restore.step_done().unwrap();drop(restore);
        let before=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap().revision;
        let begin=br#"{"schema":"gogoke.37.managed-cli.v1","command":"begin","driverId":"opencode","targetVersion":"1.18.31"}"#;
        assert!(product.dispatch_user_managed_cli(begin).is_err());
        assert_eq!(instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap().revision,before);
        product.close_checked().unwrap();
        drop(root);
        fs::remove_file(database).unwrap();
        fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn first_migration_rejects_raw_previous_version_even_when_status_masks_it() {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-first-copy-bridge-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut db=create_new(&root,&database).unwrap();
        instance::initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,revision) VALUES('opencode','STAGED',1)").unwrap();
        require_first_managed_copy(&db,"opencode").unwrap();
        db.execute("UPDATE main.gogoke_v37_instance_cli_copies SET previous_version='damaged' WHERE driver_id='opencode'").unwrap();
        assert!(instance::read_managed_cli(&db,&root,"opencode").unwrap()
            .unwrap().previous_version.is_none());
        assert!(matches!(require_first_managed_copy(&db,"opencode"),
            Err(OrchestrationError::AccessDenied)));
        db.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    // These controls seed only the old H wire-operation metadata and the
    // already-probed first copy. F HOME, H admission/episode, C identity and
    // native StopProof still come from their production writers. In particular,
    // a seeded H STOP row is not evidence that ProductDatabase ran a USER stop.
    fn family_insert(product:&ProductDatabase<'_>,sql:&str,values:&[&str]) {
        let row=Statement::prepare(product.connection.as_ptr(),sql).unwrap();
        for (index,value) in values.iter().enumerate() {
            row.bind_text(index as i32+1,value).unwrap();
        }
        row.step_done().unwrap();
    }

    fn stopped_official_family_member(product:&mut ProductDatabase<'_>,root:&RootLock,
        image:&std::path::Path,tag:&str,pending_stdin:bool)->(String,String,String) {
        use crate::store::session_transport as h;
        let session=format!("session{tag}");
        let seat="seatA".to_owned();
        let incarnation="incA".to_owned();
        let reserve_id=format!("reserve{tag}");
        let commit_id=format!("commit{tag}");
        let open_id=format!("open{tag}");
        let stop_id=format!("stop{tag}");
        let suffix=crate::store::digest::sha256_hex(format!("{}\nglobal\n{session}\n{incarnation}\n1",
            root.canonical_root().identity.opaque()).as_bytes());
        let suffix=&suffix[..40];
        let home=format!("home-{suffix}");
        let binding=format!("binding-{suffix}");
        let profile=crate::process::AppContainerProfile::derive_for_revocation(
            &format!("Gogoke37.Session.{suffix}")).unwrap();
        if tag=="A" {
            family_insert(product,"INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('global',?1,?2,'USER','LONG','instanceA','BUSY',1,1)",
                &[&seat,&incarnation]);
            family_insert(product,"INSERT INTO main.gogoke_v37_seat_secretary VALUES(1,'global',?1,?2,'family-designation','family-history-fixture')",
                &[&seat,&incarnation]);
        }
        let reserve_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-reserve\",\"requestId\":\"{reserve_id}\",\"targetId\":\"{session}\",\"domainId\":\"global\",\"expectedRevision\":\"0\",\"payload\":{{}}}}");
        let home_request=format!("homeprep-{}",&crate::store::digest::sha256_hex(
            format!("global\n{reserve_id}").as_bytes())[..40]);
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        h::bind_owner_in_transaction(&mut product.connection,&h::OwnerBinding {
            binding_id:&binding,instance_id:"instanceA",domain_id:"global",
            kind:"SESSION",owner_id:&session,generation:"1"}).unwrap();
        product.connection.execute("COMMIT").unwrap();
        instance::create_temporary_home(&mut product.connection,root,&profile,
            &instance::CreateTemporaryHome {request_id:&home_request,
                request_bytes:reserve_raw.as_bytes(),home_id:&home,instance_id:"instanceA",
                domain_id:"global",kind:instance::TemporaryKind::Session,
                owner_id:&session,generation:"1"}).unwrap();
        let selection=h::session_binding::SessionBinding {
            domain_id:"global".into(),session_id:session.clone(),seat_id:seat.clone(),
            seat_incarnation:incarnation.clone(),seat_authorization_generation:
                crate::store::seat::get(&product.connection,"global",&seat)
                    .unwrap().unwrap().generation,
            selected_instance_id:"instanceA".into(),
            provenance:h::session_binding::Provenance::NativeV2,
        };
        let reserve=h::AdmissionRequest {domain_id:"global",session_id:&session,
            request_id:&reserve_id,raw_bytes:reserve_raw.as_bytes(),instance_id:"instanceA",
            home_id:&home,generation:"1",expected_revision:0};
        assert!(matches!(h::reserve_admission(&mut product.connection,&reserve,|db|{
            h::session_binding::select_native_in_transaction(db,&selection)
                .map_err(h::AdmissionError::Relationship)?;
            Ok(h::TrustedLimits {project_parallel:4,instance_concurrency:4})
        }).unwrap(),h::AdmissionResult::Applied(1)));
        let commit_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-commit\",\"requestId\":\"{commit_id}\",\"targetId\":\"{session}\",\"domainId\":\"global\",\"expectedRevision\":\"1\",\"payload\":{{}}}}");
        let commit=h::AdmissionRequest {domain_id:"global",session_id:&session,
            request_id:&commit_id,raw_bytes:commit_raw.as_bytes(),instance_id:"instanceA",
            home_id:&home,generation:"1",expected_revision:1};
        assert!(matches!(h::commit_admission(&mut product.connection,&commit,|_|Ok(())).unwrap(),
            h::AdmissionResult::Applied(2)));
        let open_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"open\",\"requestId\":\"{open_id}\",\"targetId\":\"{session}\",\"domainId\":\"global\",\"expectedRevision\":\"2\",\"payload\":{{}}}}");
        let operation=format!("h-open-{}",&crate::store::digest::sha256_hex(open_raw.as_bytes())[..40]);
        let open_hex=action_hex(open_raw.as_bytes());
        family_insert(product,"INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('global',?1,?2,'open',?3,'UNKNOWN',2,2)",
            &[&open_id,&open_hex,&session]);
        let pin=instance::read_fixed_official_cli("opencode").unwrap();
        let mut launch=ProcessLaunch::new(image.to_path_buf());
        launch.arguments=vec!["--version".into()];
        launch.current_directory=Some(root.canonical_root().canonical_path.clone());
        let prepared=product.process_custodian.prepare(&PrepareRequest {launch,binding:NativeBinding {
            binary_digest_sha256:format!("sha256:{}",pin.image_sha256),
            profile_id:"instanceA".into(),domain_id:"global".into(),generation:"1".into(),
        }}).unwrap();
        authority::record_prepared_process(&mut product.connection,&operation,&prepared).unwrap();
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        h::bind_process_operation_in_transaction(&mut product.connection,"global",&session,&operation).unwrap();
        h::record_initial(&product.connection,"global",&session,&open_id,&operation).unwrap();
        h::session_binding::insert_native_in_transaction(&product.connection,&selection).unwrap();
        product.connection.execute("COMMIT").unwrap();
        product.process_custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut product.connection,&operation,&prepared).unwrap();
        family_insert(product,"UPDATE main.gogoke_v37_h_operation SET status='APPLIED',revision=3 WHERE domain_id='global' AND request_id=?1 AND status='UNKNOWN' AND previous_revision=2 AND revision=2",
            &[&open_id]);
        h::mark_active(&product.connection,&operation).unwrap();
        // Seed the completed OPEN's H claim revision before the original stdin
        // prepare; the later native STOP remains a distinct transition to 4.
        family_insert(product,"UPDATE main.gogoke_v37_h_claim SET revision=3 WHERE domain_id='global' AND session_id=?1 AND revision=2",
            &[&session]);
        if pending_stdin {
            let send_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"send\",\"requestId\":\"send{tag}\",\"targetId\":\"{session}\",\"domainId\":\"global\",\"expectedRevision\":\"3\",\"payload\":{{\"generation\":\"1\",\"body\":\"retained intent only\"}}}}\n");
            h::prepare_stdin_request(&mut product.connection,&h::StdinRequest {
                domain_id:"global",session_id:&session,ticket:prepared.ticket.opaque(),
                generation:"1",request_bytes:send_raw.as_bytes(),
            }).unwrap();
        }
        let stop_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"stop\",\"requestId\":\"{stop_id}\",\"targetId\":\"{session}\",\"domainId\":\"global\",\"expectedRevision\":\"3\",\"payload\":{{\"seatId\":\"{seat}\",\"generation\":\"1\"}}}}");
        let stop_hex=action_hex(stop_raw.as_bytes());
        family_insert(product,"INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('global',?1,?2,'stop',?3,'APPLIED',3,4)",
            &[&stop_id,&stop_hex,&session]);
        family_insert(product,"UPDATE main.gogoke_v37_h_process_episode SET stop_request_id=?1 WHERE process_operation_id=?2",
            &[&stop_id,&operation]);
        let proof=product.process_custodian.stop(&prepared.ticket,StopBudgets::production(),||Ok(())).unwrap();
        let revision=authority::mark_process_stopped(&mut product.connection,&operation,&proof).unwrap();
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        let stored=h::record_session_stop_in_transaction(&mut product.connection,"global",&session,&operation).unwrap();
        product.connection.execute("COMMIT").unwrap();
        assert_eq!(stored,proof.proof_hash());
        confirm_durable_test_stop(product,&prepared,&proof,revision);
        (home,operation,session)
    }

    #[test]
    fn original_stopped_family_with_pending_stdin_reaches_bound_user_action() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"),
            "official family and native stop controls are cloud-only");
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-family-stop-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut product=ProductDatabase::open(&root,&database).unwrap();
        let (source_stage,image)=super::super::managed_cli_test_setup::prepared_image(&root,"opencode");
        // This first-copy PROBED state is an explicit fixture seed. F's actual
        // stage/confirm/switch writers and official bytes are used, but the
        // first version string is supplied from the fixed pin, not H stdout.
        default_begin(&mut product,&root);
        instance::record_managed_cli_stage(&mut product.connection,&root,&product.owner,
            "opencode",&source_stage).unwrap();
        let pin=instance::read_fixed_official_cli("opencode").unwrap();
        instance::confirm_managed_cli_launch(&mut product.connection,&root,"opencode",
            &source_stage,&format!("sha256:{}",pin.image_sha256),pin.version).unwrap();
        let observed=instance::ProgramObservation::observe(&image,pin.version).unwrap();
        instance::register_instance(&mut product.connection,&root,&instance::Registration {
            request_id:"register-family",request_bytes:b"register-family",instance_id:"instanceA",
            driver_id:"opencode",program:&observed,
        }).unwrap();
        family_insert(&product,"INSERT INTO main.gogoke_v37_instance_profiles(instance_id,display_name,enabled,revision) VALUES('instanceA','instanceA',1,1)",&[]);
        let migrate=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"migrate\",\"driverId\":\"opencode\",\"stageName\":\"{source_stage}\"}}");
        managed_user(&mut product,&migrate);
        let (home_a,operation_a,_)=stopped_official_family_member(&mut product,&root,&image,"A",true);
        // A genuine Secretary cannot acquire a sibling until the earlier H
        // claim is RELEASED. Keep its original HOME, pending stdin and STOP.
        let release_raw=b"{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-release\",\"requestId\":\"releaseA\",\"targetId\":\"sessionA\",\"domainId\":\"global\",\"expectedRevision\":\"4\",\"payload\":{}}";
        assert!(matches!(crate::store::session_transport::release_admission(
            &mut product.connection,&crate::store::session_transport::AdmissionRequest {
                domain_id:"global",session_id:"sessionA",request_id:"releaseA",
                raw_bytes:release_raw,instance_id:"instanceA",home_id:&home_a,
                generation:"1",expected_revision:4},|_|Ok(())).unwrap(),
            crate::store::session_transport::AdmissionResult::Applied(5)));
        let seat=crate::store::seat::get(&product.connection,"global","seatA")
            .unwrap().unwrap();
        crate::store::seat::set_dispatch_state(&mut product.connection,&seat,true).unwrap();
        let (_home_b,_operation_b,_)=stopped_official_family_member(&mut product,&root,&image,"B",false);
        let qualified=product.managed_version_action_resources("opencode",None)
            .expect("both distinct sessions and the retained pending stdin must qualify together");
        assert!(qualified.is_cold(),"a complete stopped family must produce COLD, not strict empty scope");
        assert_eq!(table_rows(&product,"SELECT session_id,generation FROM main.gogoke_v37_h_process_episode ORDER BY session_id",2),
            vec![vec!["sessionA".into(),"1".into()],vec!["sessionB".into(),"1".into()]]);
        let original_h=table_rows(&product,"SELECT session_id,phase,stop_request_id,stop_fact_id,raw_hex FROM main.gogoke_v37_h_process_episode ORDER BY session_id",5);
        let original_h_operations=table_rows(&product,"SELECT session_id,request_id,raw_hex,status,revision FROM main.gogoke_v37_h_operation ORDER BY session_id,request_id",5);
        let original_stdin=table_rows(&product,"SELECT request_id,request_hex,phase,ticket,custodian_nonce FROM main.gogoke_v37_h_stdin_journal ORDER BY request_id",5);
        let original_c=table_rows(&product,"SELECT operation_id,state,stop_proof_hash FROM main.gogoke_coordination_process_custody WHERE profile_id='instanceA' ORDER BY operation_id",3);
        for (name,sql,values) in [
            ("F create receipt","UPDATE main.gogoke_v37_instance_operations SET native_receipt_id='changed' WHERE target_id=?1",vec![home_a.as_str()]),
            ("C stop proof","UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash=NULL WHERE operation_id=?1",vec![operation_a.as_str()]),
            ("stdin raw","UPDATE main.gogoke_v37_h_stdin_journal SET request_hex='00' WHERE request_id='sendA'",vec![]),
            ("stdin nonce","UPDATE main.gogoke_v37_h_stdin_journal SET custodian_nonce='changed' WHERE request_id='sendA'",vec![]),
        ] {
            product.connection.execute("SAVEPOINT family_damage").unwrap();
            family_insert(&product,sql,&values);
            assert!(product.managed_version_action_resources("opencode",None).is_err(),
                "{name} must independently refuse from the same positive family");
            product.connection.execute("ROLLBACK TO family_damage").unwrap();
            product.connection.execute("RELEASE family_damage").unwrap();
            assert!(product.managed_version_action_resources("opencode",None).unwrap().is_cold(),
                "restoring {name} must restore the same original family");
        }
        let (stage33,_)=real_opencode_version_stage(&root,"1.18.33");
        let begin=r#"{"schema":"gogoke.37.managed-cli.v1","command":"begin","driverId":"opencode","targetVersion":"1.18.33","requestId":"family-upgrade","operation":"upgrade"}"#;
        managed_user(&mut product,begin);
        managed_user(&mut product,&version_frame("stage","1.18.33",Some(&stage33),Some("family-upgrade")));
        managed_user(&mut product,&version_frame("probe","1.18.33",Some(&stage33),Some("family-upgrade")));
        let copy=instance::read_managed_cli(&product.connection,&root,"opencode").unwrap().unwrap();
        assert_eq!(copy.state,"PROBED","the target --version must be observed by the native probe");
        let switch=format!("{{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"upgrade\",\"driverId\":\"opencode\",\"targetVersion\":\"1.18.33\",\"requestId\":\"family-switch\",\"expectedRevision\":\"{}\"}}",copy.revision);
        let (status,receipts)=receipt_set(&managed_user(&mut product,&switch));
        assert_eq!(status,"APPLIED");assert_eq!(receipts.len(),1);
        assert_eq!(table_rows(&product,"SELECT session_id,phase,stop_request_id,stop_fact_id,raw_hex FROM main.gogoke_v37_h_process_episode ORDER BY session_id",5),original_h);
        assert_eq!(table_rows(&product,"SELECT session_id,request_id,raw_hex,status,revision FROM main.gogoke_v37_h_operation ORDER BY session_id,request_id",5),original_h_operations);
        assert_eq!(table_rows(&product,"SELECT request_id,request_hex,phase,ticket,custodian_nonce FROM main.gogoke_v37_h_stdin_journal ORDER BY request_id",5),original_stdin);
        assert_eq!(table_rows(&product,"SELECT operation_id,state,stop_proof_hash FROM main.gogoke_coordination_process_custody WHERE profile_id='instanceA' ORDER BY operation_id",3),original_c,
            "the action must preserve the original two STOP facts and pending stdin");
        product.close_checked().unwrap();drop(root);
        fs::remove_file(database).unwrap();
        fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        fs::remove_dir_all(path).unwrap();
    }
}
impl<'a> ProductDatabase<'a> {
    fn managed_cli_failure(&mut self, driver: &str, state: &str, raw: String) -> OrchestrationError {
        match instance::record_managed_cli_failure(&mut self.connection, driver, state, &raw) {
            Ok(()) => OrchestrationError::V37StoreFailure(raw),
            Err(error) => OrchestrationError::V37StoreFailure(format!(
                "{raw}; managed CLI failure record: {error:?}"
            )),
        }
    }
    fn probe_staged_managed_cli(&mut self,driver:&str,stage:&str,request:&str)->Result<()> {
        self.require_first_managed_copy(driver)?;
        let pin=instance::read_fixed_official_cli(driver)
            .ok_or(OrchestrationError::Invalid("fixed CLI driver"))?;
        self.probe_staged_managed_cli_for_version(driver,pin.version,stage,request,false,None)
    }

    fn probe_staged_managed_cli_version(&mut self,driver:&str,version:&str,
        stage:&str,request:&str)->Result<()> {
        self.probe_staged_managed_cli_for_version(driver,version,stage,request,true,None)
    }

    fn probe_staged_managed_cli_for_version(&mut self,driver:&str,version:&str,
        stage:&str,request:&str,strict_version_change:bool,action:Option<&instance::ActionRecord>)->Result<()> {
        if let Some(action)=action {
            if action.phase!=instance::ActionPhase::Probing || action.request_id!=request ||
                action.target_version!=version || action.stage_name!=stage {
                return Err(OrchestrationError::AccessDenied);
            }
            self.managed_version_action_resources(driver,Some(action))?;
        } else if strict_version_change {self.strict_managed_version_resources(driver)?;}
        let failure_record=|this:&mut Self,state:&str,raw:String| {
            if let Some(action)=action {
                match this.fail_managed_version_action(driver,state,&raw,action) {
                    Ok(())=>OrchestrationError::V37StoreFailure(raw),
                    Err(error)=>OrchestrationError::V37StoreFailure(format!("{raw}; original action failure record: {error:?}")),
                }
            } else {this.managed_cli_failure(driver,state,raw)}
        };
        let pin=instance::read_official_cli_version(driver,version)
            .ok_or(OrchestrationError::Invalid("compiled CLI version"))?;
        let copy=instance::read_managed_cli(&self.connection,self.root,driver)
            .map_err(|error|failure("stage row",error))?
            .ok_or(OrchestrationError::AccessDenied)?;
        if copy.state!="STAGED"||copy.stage_name.as_deref()!=Some(stage)||
            copy.version.as_deref()!=Some(version){
            return Err(OrchestrationError::AccessDenied);
        }
        let image=match instance::inspect_staged_official_cli_version(self.root,driver,version,stage){
            Ok(image)=>image,
            Err(error)=>{
                let raw=format!("staged archive/image verification: {error:?}");
                return Err(failure_record(self,"BLOCKED",raw));
            },
        };
        let home=match image.ancestors().find(|path|
            path.file_name().and_then(|name|name.to_str())==Some("content")).and_then(Path::parent) {
            Some(stage)=>stage.join("probe-home"),
            None=>return Err(failure_record(self,"BLOCKED","private probe stage parent is absent".into())),
        };
        match fs::create_dir(&home) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // STAGED may survive a stop before the probe was started, or
                // after its original durable stop. The source stage and all
                // prior custody were checked above; reuse only a plain folder.
                let metadata = fs::symlink_metadata(&home)
                    .map_err(|cause|failure_record(self,"BLOCKED",format!("existing private probe home: {cause:?}")))?;
                if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
                    return Err(failure_record(self,"BLOCKED",format!("private probe home identity: directory={}, attributes={:#x}",
                        metadata.is_dir(),metadata.file_attributes())));
                }
            },
            Err(error) => return Err(failure_record(self,"BLOCKED",format!("private probe home: {error:?}"))),
        }
        let system_root=std::env::var("SystemRoot")
            .map_err(|error|failure_record(self,"BLOCKED",format!("SystemRoot: {error:?}")))?;
        let mut launch=ProcessLaunch::new(&image);
        launch.arguments=vec!["--version".into()];
        launch.current_directory=Some(home.clone());
        launch.protocol_stdio=true;
        launch.environment=Some(vec![
            ("SystemRoot".into(),system_root),
            ("HOME".into(),home.to_string_lossy().into_owned()),
            ("USERPROFILE".into(),home.to_string_lossy().into_owned()),
            ("APPDATA".into(),home.to_string_lossy().into_owned()),
            ("LOCALAPPDATA".into(),home.to_string_lossy().into_owned()),
        ]);
        let digest=format!("sha256:{}",pin.image_sha256);
        let binding=NativeBinding{binary_digest_sha256:digest.clone(),
            profile_id:format!("managed-cli-{driver}"),domain_id:"global".into(),
            generation:copy.revision.to_string()};
        // A durable STOPPED prior attempt can be re-probed after a crash in
        // STAGED. Unsettled managed probe attempts were rejected above.
        let attempts=Statement::prepare(self.connection.as_ptr(),
            "SELECT COUNT(*) FROM main.gogoke_coordination_process_custody WHERE profile_id=?1")?;
        attempts.bind_text(1,&format!("managed-cli-{driver}"))?;
        if !attempts.step_row()?{return Err(OrchestrationError::AccessDenied)}
        let attempt=attempts.column_text(0)?.parse::<u64>()
            .map_err(|_|OrchestrationError::Invalid("managed CLI probe attempt"))?
            .checked_add(1).ok_or(OrchestrationError::Invalid("managed CLI probe attempt overflow"))?;
        if attempts.step_row()?{return Err(OrchestrationError::AccessDenied)}
        drop(attempts);
        let operation_id=format!("managed-probe-{driver}-{}-{attempt}",
            &crate::store::digest::sha256_hex(request.as_bytes())[..40]);
        let prepared=match self.process_custodian.prepare(&PrepareRequest{launch,binding}){
            Ok(value)=>value,
            Err(error)=>{
                let state=if matches!(error,crate::process::ProcessCustodyError::LaunchCleanup{..})
                    {"PROBE_UNKNOWN"}else{"BLOCKED"};
                return Err(failure_record(self, state,
                    format!("version probe prepare: {error:?}")));
            },
        };
        if let Err(error)=authority::record_prepared_process(&mut self.connection,&operation_id,&prepared){
            let abort=self.process_custodian.abort_prepared(&prepared);
            let raw=format!("probe prepared record: {error:?}; abort: {abort:?}");
            let state=if abort.is_ok(){"BLOCKED"}else{"PROBE_UNKNOWN"};
            return Err(failure_record(self,state,raw));
        }
        if let Err(error)=self.process_custodian.activate(&prepared){
            let unknown=authority::mark_process_unknown(&mut self.connection,&operation_id,&prepared);
            let raw=format!("probe activate: {error:?}; custody: {unknown:?}");
            return Err(failure_record(self,"PROBE_UNKNOWN",raw));
        }
        if let Err(error)=authority::mark_process_active(&mut self.connection,&operation_id,&prepared){
            let stop=self.process_custodian.stop(&prepared.ticket,StopBudgets::production(),||Ok(()));
            let unknown=authority::mark_process_unknown(&mut self.connection,&operation_id,&prepared);
            let raw=format!("probe active record: {error:?}; stop: {stop:?}; custody: {unknown:?}");
            return Err(failure_record(self,"PROBE_UNKNOWN",raw));
        }
        let output=self.process_custodian.active(&prepared.ticket)
            .ok_or(OrchestrationError::AccessDenied)?
            .read_protocol_frame(Duration::from_secs(30));
        let proof=match self.process_custodian.stop(&prepared.ticket,StopBudgets::production(),||Ok(())){
            Ok(value)=>value,
            Err(error)=>{
                let unknown=authority::mark_process_unknown(&mut self.connection,&operation_id,&prepared);
                let raw=format!("probe stop: {error:?}; custody: {unknown:?}");
                return Err(failure_record(self,"PROBE_UNKNOWN",raw));
            },
        };
        if !proof.errors.is_empty()||!proof.parent_exited||!proof.writer_fence_verified||
            proof.active_job_processes!=Some(0){
            let unknown=authority::mark_process_unknown(&mut self.connection,&operation_id,&prepared);
            let raw=format!("probe stop unresolved: {proof:?}; custody: {unknown:?}");
            return Err(failure_record(self,"PROBE_UNKNOWN",raw));
        }
        // The exact child may finish writing stderr after stdout. The Job
        // writer fence makes this drain final before durable confirmation
        // releases our access to that same child.
        let (stderr,stderr_drain)=match self.process_custodian.active(&prepared.ticket){
            Some(process)=>{
                let drain=process.drain_stderr_after_writers_stopped().err();
                (process.stderr_tail(),drain)
            },
            None=>("original probe stderr custody absent".into(),
                Some("original probe process absent before durable stop".into())),
        };
        let revision=match authority::mark_process_stopped(&mut self.connection,&operation_id,&proof){
            Ok(value)=>value,
            Err(error)=>{
                let unknown=authority::mark_process_unknown(&mut self.connection,&operation_id,&prepared);
                let raw=format!("probe stop record: {error:?}; STDERR_TAIL: {stderr}; custody: {unknown:?}");
                return Err(failure_record(self,"PROBE_UNKNOWN",raw));
            },
        };
        let proof=match self.process_custodian.confirm_stop_durable(&DurableStopConfirmation{
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:proof.proof_hash(),durable_revision:revision,
        }){
            Ok(confirmed)=>confirmed,
            Err(error)=>{
                let raw=format!("probe durable stop confirmation: {error:?}; proof: {proof:?}; STDERR_TAIL: {stderr}");
                return Err(failure_record(self,"PROBE_UNKNOWN",raw));
            },
        };
        if let Some(error)=stderr_drain {
            let raw=format!("version stderr drain: {error}; STDERR_TAIL: {stderr}");
            return Err(failure_record(self,"BLOCKED",raw));
        }
        let stdout=match output {
            Ok(bytes)=>bytes,
            Err(error)=>{
                let raw=format!("version stdout: {error}; STDERR_TAIL: {stderr}");
                return Err(failure_record(self,"BLOCKED",raw));
            },
        };
        let observed=match std::str::from_utf8(&stdout) {
            Ok(value)=>value,
            Err(error)=>{
                let raw=format!("version UTF-8: {error}; STDERR_TAIL: {stderr}");
                return Err(failure_record(self,"BLOCKED",raw));
            },
        };
        if proof.exit_code!=Some(0)||observed.len()>256||
            !observed.split(|ch:char|!ch.is_ascii_alphanumeric()&&ch!='.')
                .any(|token|token==pin.version){
            let raw=format!("version probe refused: exit={:?}; stdout={observed:?}; STDERR_TAIL: {stderr}",proof.exit_code);
            return Err(failure_record(self,"BLOCKED",raw));
        }
        if let Some(action)=action {
            self.version_action_transaction(|this| {
                this.managed_version_action_resources_with_probe(driver,action,&proof)?;
                instance::confirm_managed_cli_launch_for_version_action_in_current_transaction(&this.connection,this.root,
                    &this.owner,driver,version,stage,&digest,pin.version,action,&proof)
                    .map_err(|error|failure("bound PROBED confirmation",error))?;
                let next=instance::managed_version_fingerprints(&this.connection,this.root,driver)
                    .map_err(|error|failure("probe post-effect fingerprints",error))?;
                this.managed_version_action_resources_at_fingerprints_with_probe(driver,action,&next.0,&next.1,&proof)?;
                let receipt=bound_step_response("probe",driver,version,request,&action.operation,"PROBED",Some(stage));
                let receipt_hex=instance::encode_bound_probe_receipt(action,&operation_id,
                    &proof,&stdout,&receipt)
                    .map_err(|error|failure("probe original custody receipt",error))?;
                instance::advance_version_action_in_current_transaction(&this.connection,this.root,&this.owner,
                    request,action.revision,&instance::ActionStep {
                        next_phase:instance::ActionPhase::Probed,raw_hex:&action.probe_hex,receipt_hex:&receipt_hex,
                        stage_name:stage,expected_copy_fingerprint:&action.current_copy_fingerprint,
                        expected_source_fingerprint:&action.current_source_fingerprint,
                        next_copy_fingerprint:&next.0,next_source_fingerprint:&next.1,
                    }).map_err(|error|failure("probe original action receipt",error))?;
                Ok(())
            })
        } else {
            if strict_version_change {self.strict_managed_version_resources(driver)?;}
            instance::confirm_managed_cli_launch_for_version(&mut self.connection,self.root,
                driver,version,stage,&digest,pin.version)
                .map_err(|error|failure("READY confirmation",error))
        }
    }
}
