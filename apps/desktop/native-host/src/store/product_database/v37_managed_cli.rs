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

impl<'a> ProductDatabase<'a> {
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

    fn qualified_managed_source_resources(&self,id:&str,driver:&str)->Result<()> {
        let pending=|sql:&str|->Result<bool>{
            let row=Statement::prepare(self.connection.as_ptr(),sql)?;
            row.bind_text(1,id)?;
            Ok(row.step_row()?)
        };
        for sql in [
            "SELECT 1 FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=g.old_process_operation_id WHERE e.instance_id=?1 AND g.stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_h_operation o JOIN main.gogoke_v37_h_claim c ON c.domain_id=o.domain_id AND c.session_id=o.session_id WHERE c.instance_id=?1 AND o.status!='APPLIED' LIMIT 1",
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
            if state=="RELEASED" && stop.is_empty() && !operation.is_empty() &&
                driver=="codex" && self.completed_codex_holder_release(id,domain,session,operation)? {
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
            if !operation.is_empty() && driver=="codex" &&
                self.completed_codex_holder_release(id,domain,session,operation)? {continue}
            return Err(OrchestrationError::AccessDenied);
        }
        Ok(())
    }

    fn migrate_managed_cli(&mut self,driver:&str,stage:&str)->Result<usize> {
        self.connection.execute("BEGIN IMMEDIATE")?;
        let result=(||{
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)
                .map_err(|error|failure("migration owner",error))?;
            self.no_live_global_cli_owner(driver)?;
            let qualified=self.qualified_managed_source_instances(driver)?;
            instance::migrate_quiescent_legacy_instances(&self.connection,self.root,&self.owner,
                driver,stage,&qualified).map_err(|error|failure("legacy migration",error))
        })();
        match result {
            Ok(count)=>{self.connection.execute("COMMIT")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(count)},
            Err(error)=>{self.connection.execute("ROLLBACK")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;Err(error)},
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
                let state=instance::read_managed_cli(&self.connection,self.root,&driver)
                    .map_err(|error|failure("status",error))?
                    .map_or_else(||"NOT_INSTALLED".to_owned(),|copy|copy.state);
                Ok(result("status",[("driverId",string(&driver)),("state",string(&state))]))
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
                self.no_live_global_cli_owner(&driver)?;
                instance::record_managed_cli_progress(&mut self.connection,&self.owner,
                    &driver,"DOWNLOADING",0).map_err(|error|failure("begin download",error))?;
                Ok(result("begin",[("driverId",string(&driver)),
                    ("state",string("DOWNLOADING"))]))
            },
            "stage" if fields.len()==4=>{
                let driver=field(&fields,"driverId")?;
                let stage=field(&fields,"stageName")?;
                self.no_live_global_cli_owner(&driver)?;
                instance::record_managed_cli_stage(&mut self.connection,self.root,
                    &self.owner,&driver,&stage).map_err(|error|failure("record stage",error))?;
                Ok(result("stage",[("driverId",string(&driver)),("state",string("STAGED"))]))
            },
            "failure" if fields.len()==5=>{
                let driver=field(&fields,"driverId")?;
                let state=field(&fields,"state")?;
                let raw=field(&fields,"raw")?;
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
        let pin=instance::read_fixed_official_cli(driver)
            .ok_or(OrchestrationError::Invalid("fixed CLI driver"))?;
        let copy=instance::read_managed_cli(&self.connection,self.root,driver)
            .map_err(|error|failure("stage row",error))?
            .ok_or(OrchestrationError::AccessDenied)?;
        if copy.state!="STAGED"||copy.stage_name.as_deref()!=Some(stage){
            return Err(OrchestrationError::AccessDenied);
        }
        let image=match instance::inspect_staged_official_cli(self.root,driver,stage){
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
        instance::confirm_managed_cli_launch(&mut self.connection,self.root,
            driver,stage,&digest,pin.version).map_err(|error|failure("READY confirmation",error))
    }
}
