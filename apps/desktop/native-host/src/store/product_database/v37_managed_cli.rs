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
    fn require_first_managed_copy(&self,driver:&str)->Result<()> {
        require_first_managed_copy(&self.connection,driver)?;
        instance::require_default_first_copy(&self.connection,driver)
            .map_err(|error|failure("default first copy",error))
    }

    fn strict_managed_version_resources(&mut self,driver:&str)->Result<()> {
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
            if !first_claude_copy || !self.completed_claude_managed_pending_operation(id,&domain,&session,&request)? {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        let stdin=Statement::prepare(self.connection.as_ptr(),
            "SELECT j.domain_id,j.request_id FROM main.gogoke_v37_h_stdin_journal j JOIN main.gogoke_v37_h_claim c ON c.domain_id=j.domain_id AND c.session_id=j.session_id WHERE c.instance_id=?1 AND j.phase IN ('PREPARED','UNKNOWN')")?;
        stdin.bind_text(1,id)?;
        let mut retained=Vec::new();
        while stdin.step_row()? {retained.push((stdin.column_text(0)?,stdin.column_text(1)?));}
        drop(stdin);
        for (domain,request) in retained {
            if !first_claude_copy || !self.completed_claude_managed_pending_stdin(id,&domain,&request)? {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        for sql in [
            "SELECT 1 FROM main.gogoke_v37_instance_homes WHERE instance_id=?1 AND state NOT IN ('ACTIVE','CLEANED','CLOSED') LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase!='APPLIED' LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_credential_objects WHERE instance_id=?1 AND phase!='ACTIVE' LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_credential_aliases WHERE instance_id=?1 AND state IN ('PREPARING','REMOVE_PENDING','UNKNOWN') LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_credential_profiles WHERE instance_id=?1 AND state IN ('GRANT_PENDING','REVOKE_PENDING','UNKNOWN') LIMIT 1",
        ] {if pending(sql)? {return Err(OrchestrationError::AccessDenied)}}

        let claims=Statement::prepare(self.connection.as_ptr(),
            "SELECT a.domain_id,a.session_id,a.state,COALESCE(a.process_operation_id,''),COALESCE(a.stop_fact_id,''),COALESCE(c.state,''),COALESCE(c.stop_proof_hash,'') FROM main.gogoke_v37_h_claim a LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=a.process_operation_id AND c.profile_id=a.instance_id AND c.domain_id=a.domain_id AND c.generation=a.generation WHERE a.instance_id=?1")?;
        claims.bind_text(1,id)?;
        let mut rows=Vec::new();
        while claims.step_row()? {rows.push((0..7).map(|n|claims.column_text(n)).collect::<std::result::Result<Vec<_>,_>>()?);}
        drop(claims);
        for row in rows {
            let (domain,session,state,operation,stop,custody,proof)=
                (&row[0],&row[1],&row[2],&row[3],&row[4],&row[5],&row[6]);
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
            "SELECT e.domain_id,e.session_id,COALESCE(e.process_operation_id,''),e.phase,COALESCE(e.stop_fact_id,''),COALESCE(c.state,''),COALESCE(c.stop_proof_hash,'') FROM main.gogoke_v37_h_process_episode e LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE e.instance_id=?1")?;
        episodes.bind_text(1,id)?;
        let mut rows=Vec::new();
        while episodes.step_row()? {rows.push((0..7).map(|n|episodes.column_text(n)).collect::<std::result::Result<Vec<_>,_>>()?);}
        drop(episodes);
        for row in rows {
            let (domain,session,operation,phase,stop,custody,proof)=
                (&row[0],&row[1],&row[2],&row[3],&row[4],&row[5],&row[6]);
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
            "status" if fields.len()==3=>{
                let driver=field(&fields,"driverId")?;
                let copy=instance::read_managed_cli(&self.connection,self.root,&driver)
                    .map_err(|error|failure("status",error))?;
                let prepared=instance::read_prepared_managed_replacement(&self.connection,self.root,&driver)
                    .map_err(|error|failure("status source shape",error))?;
                Ok(match copy {
                    None=>result("status",[("driverId",string(&driver)),("state",string("NOT_INSTALLED")),
                        ("revision",Json::Null),("version",Json::Null),
                        ("stageName",Json::Null),("preparedReplacement",Json::Null),
                        ("progressBytes",Json::Null)]),
                    Some(copy)=>result("status",[("driverId",string(&driver)),("state",string(&copy.state)),
                        ("revision",string(&copy.revision.to_string())),("version",optional(copy.version.as_deref())),
                        ("stageName",optional(copy.stage_name.as_deref())),
                        ("preparedReplacement",prepared.map_or(Json::Null,Json::Bool)),
                        ("progressBytes",Json::Number(copy.progress_bytes.to_string())),
                        ("previousVersion",optional(copy.previous_version.as_deref())),
                        ("verifiedOtherVersions",Json::Array(copy.verified_other_versions.iter().map(|version|string(version)).collect())),
                        ("officialVersion",optional(copy.official_notice.as_deref())),
                        ("checkedAt",optional(copy.checked_at.as_deref())),("raw",optional(copy.raw_error.as_deref()))]),
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

    static PINNED_31_ARCHIVES:OnceLock<Mutex<HashMap<(String,String),PathBuf>>>=OnceLock::new();

    fn real_opencode_31_stage(root:&RootLock)->(String,PathBuf) {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"),
            "two-version official CLI fixture is cloud-only");
        let pin=instance::read_official_cli_version("opencode","1.18.31").unwrap();
        let key=(pin.driver.to_owned(),pin.version.to_owned());
        let mut cache=PINNED_31_ARCHIVES.get_or_init(||Mutex::new(HashMap::new())).lock().unwrap();
        let archive=cache.entry(key).or_insert_with(||{
            let folder=std::env::temp_dir().join(format!(
                "gogoke-opencode-two-pin-{}",std::process::id()));
            fs::create_dir_all(&folder).unwrap();
            let path=folder.join("opencode-1.18.31.download");
            let status=Command::new("curl.exe").args([
                "--fail","--location","--silent","--show-error","--output",
            ]).arg(&path).arg("https://registry.npmjs.org/opencode-windows-x64/-/opencode-windows-x64-1.18.31.tgz")
                .status().expect("fixed OpenCode 1.18.31 archive download");
            assert!(status.success(),"fixed OpenCode 1.18.31 archive download: {status}");
            path
        }).clone();
        drop(cache);
        assert_eq!(crate::store::digest::sha256_hex(&fs::read(&archive).unwrap()),
            pin.archive_sha256,"fixed OpenCode 1.18.31 archive pin");
        let staging=instance::managed_cli_root(root).unwrap();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let stage=format!("opencode-1.18.31-fixture-{nonce}");
        let folder=staging.join(&stage);
        let content=folder.join("content");
        fs::create_dir(&folder).unwrap();
        fs::create_dir(&content).unwrap();
        let source=folder.join("source.download");
        fs::copy(&archive,&source).unwrap();
        let status=Command::new("tar.exe").arg("-xzf").arg(&source).arg("-C").arg(&content)
            .status().expect("fixed OpenCode 1.18.31 archive extraction");
        assert!(status.success(),"fixed OpenCode 1.18.31 archive extraction: {status}");
        let image=instance::inspect_staged_official_cli_version(root,pin.driver,pin.version,&stage)
            .expect("native verified OpenCode 1.18.31 archive and image");
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
        self.probe_staged_managed_cli_for_version(driver,pin.version,stage,request,false)
    }

    fn probe_staged_managed_cli_version(&mut self,driver:&str,version:&str,
        stage:&str,request:&str)->Result<()> {
        self.probe_staged_managed_cli_for_version(driver,version,stage,request,true)
    }

    fn probe_staged_managed_cli_for_version(&mut self,driver:&str,version:&str,
        stage:&str,request:&str,strict_version_change:bool)->Result<()> {
        if strict_version_change {self.strict_managed_version_resources(driver)?;}
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
                return Err(self.managed_cli_failure(driver,"BLOCKED",raw));
            },
        };
        let home=image.ancestors().find(|path|
            path.file_name().and_then(|name|name.to_str())==Some("content"))
            .ok_or(OrchestrationError::AccessDenied)?.parent()
            .ok_or(OrchestrationError::AccessDenied)?.join("probe-home");
        match fs::create_dir(&home) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // STAGED may survive a stop before the probe was started, or
                // after its original durable stop. The source stage and all
                // prior custody were checked above; reuse only a plain folder.
                let metadata = fs::symlink_metadata(&home)
                    .map_err(|cause| failure("existing private probe home", cause))?;
                if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
                    return Err(OrchestrationError::Invalid("private probe home identity"));
                }
            },
            Err(error) => return Err(failure("private probe home", error)),
        }
        let system_root=std::env::var("SystemRoot").map_err(|error|failure("SystemRoot",error))?;
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
                return Err(self.managed_cli_failure(driver, state,
                    format!("version probe prepare: {error:?}")));
            },
        };
        if let Err(error)=authority::record_prepared_process(&mut self.connection,&operation_id,&prepared){
            let abort=self.process_custodian.abort_prepared(&prepared);
            let raw=format!("probe prepared record: {error:?}; abort: {abort:?}");
            let state=if abort.is_ok(){"BLOCKED"}else{"PROBE_UNKNOWN"};
            return Err(self.managed_cli_failure(driver,state,raw));
        }
        if let Err(error)=self.process_custodian.activate(&prepared){
            let unknown=authority::mark_process_unknown(&mut self.connection,&operation_id,&prepared);
            let raw=format!("probe activate: {error:?}; custody: {unknown:?}");
            return Err(self.managed_cli_failure(driver,"PROBE_UNKNOWN",raw));
        }
        if let Err(error)=authority::mark_process_active(&mut self.connection,&operation_id,&prepared){
            let stop=self.process_custodian.stop(&prepared.ticket,StopBudgets::production(),||Ok(()));
            let unknown=authority::mark_process_unknown(&mut self.connection,&operation_id,&prepared);
            let raw=format!("probe active record: {error:?}; stop: {stop:?}; custody: {unknown:?}");
            return Err(self.managed_cli_failure(driver,"PROBE_UNKNOWN",raw));
        }
        let output=self.process_custodian.active(&prepared.ticket)
            .ok_or(OrchestrationError::AccessDenied)?
            .read_protocol_frame(Duration::from_secs(30));
        let proof=match self.process_custodian.stop(&prepared.ticket,StopBudgets::production(),||Ok(())){
            Ok(value)=>value,
            Err(error)=>{
                let unknown=authority::mark_process_unknown(&mut self.connection,&operation_id,&prepared);
                let raw=format!("probe stop: {error:?}; custody: {unknown:?}");
                return Err(self.managed_cli_failure(driver,"PROBE_UNKNOWN",raw));
            },
        };
        if !proof.errors.is_empty()||!proof.parent_exited||!proof.writer_fence_verified||
            proof.active_job_processes!=Some(0){
            let unknown=authority::mark_process_unknown(&mut self.connection,&operation_id,&prepared);
            let raw=format!("probe stop unresolved: {proof:?}; custody: {unknown:?}");
            return Err(self.managed_cli_failure(driver,"PROBE_UNKNOWN",raw));
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
                return Err(self.managed_cli_failure(driver,"PROBE_UNKNOWN",raw));
            },
        };
        if let Err(error)=self.process_custodian.confirm_stop_durable(&DurableStopConfirmation{
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:proof.proof_hash(),durable_revision:revision,
        }){
            let raw=format!("probe durable stop confirmation: {error:?}; proof: {proof:?}; STDERR_TAIL: {stderr}");
            return Err(self.managed_cli_failure(driver,"PROBE_UNKNOWN",raw));
        }
        if let Some(error)=stderr_drain {
            let raw=format!("version stderr drain: {error}; STDERR_TAIL: {stderr}");
            return Err(self.managed_cli_failure(driver,"BLOCKED",raw));
        }
        let stdout=match output {
            Ok(bytes)=>bytes,
            Err(error)=>{
                let raw=format!("version stdout: {error}; STDERR_TAIL: {stderr}");
                return Err(self.managed_cli_failure(driver,"BLOCKED",raw));
            },
        };
        let observed=match std::str::from_utf8(&stdout) {
            Ok(value)=>value,
            Err(error)=>{
                let raw=format!("version UTF-8: {error}; STDERR_TAIL: {stderr}");
                return Err(self.managed_cli_failure(driver,"BLOCKED",raw));
            },
        };
        if proof.exit_code!=Some(0)||observed.len()>256||
            !observed.split(|ch:char|!ch.is_ascii_alphanumeric()&&ch!='.')
                .any(|token|token==pin.version){
            let raw=format!("version probe refused: exit={:?}; stdout={observed:?}; STDERR_TAIL: {stderr}",proof.exit_code);
            return Err(self.managed_cli_failure(driver,"BLOCKED",raw));
        }
        if strict_version_change {self.strict_managed_version_resources(driver)?;}
        instance::confirm_managed_cli_launch_for_version(&mut self.connection,self.root,
            driver,version,stage,&digest,pin.version)
            .map_err(|error|failure("READY confirmation",error))
    }
}
