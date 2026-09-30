//! Capability reads use the loaded native thread's original feature response.
//! Flags describe configuration, not an authenticated model behaviour result.
use super::*;
use crate::store::seat::NativeOrigin;
use crate::store::session_transport::{runtime, codex_rpc::Reply};
use std::collections::BTreeSet;

fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}
const REQUIRED:[(&str,bool);3]=[("memories",false),("multi_agent_v2",false),("default_mode_request_user_input",true)];

fn project_flags(pages:&[(String,bool)]) -> Result<BTreeMap<JsonString,Json>> {
    let mut flags=BTreeMap::new();
    for (name,enabled) in pages {
        if REQUIRED.iter().any(|(required,_)|name==required) {
            if flags.insert(JsonString::from_str(name),Json::Bool(*enabled)).is_some() {
                return Err(OrchestrationError::Invalid("duplicate loaded thread feature"));
            }
        }
    }
    for (name,expected) in REQUIRED {
        if !matches!(flags.get(&JsonString::from_str(name)),Some(Json::Bool(actual)) if *actual==expected) {
            return Err(OrchestrationError::V37StoreFailure(format!("native loaded thread feature mismatch: {name}; expected={expected}; actual={:?}",flags.get(&JsonString::from_str(name)).map(Json::canonical))));
        }
    }
    Ok(flags)
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn dispatch_native_capability(&mut self,request:&V37Request)->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        if request.payload.len()!=1 {return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,request.expected_revision,Default::default()));}
        let generation=user_payload_string(request,"generation")?;
        let key=(request.domain_id.clone(),request.target_id.clone());
        let Some(run)=self.native_sessions.get(&key) else {return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,request.expected_revision,Default::default()));};
        let seat=run.evidence.seat_id().to_owned();
        let operation=run.operation_id.clone();
        let thread=run.thread_id.clone().ok_or(OrchestrationError::AccessDenied)?;
        let claim=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),&key.0,&seat,&key.1)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("native capability claim: {error:?}")))?
            .ok_or(OrchestrationError::AccessDenied)?;
        let revision=u64::try_from(claim.revision).map_err(|error|OrchestrationError::V37StoreFailure(format!("native capability revision: {error}")))?;
        if claim.generation!=generation {return Ok(encode_receipt(request,V37Status::Conflict,revision,revision,Default::default()));}
        if revision!=request.expected_revision {return Ok(encode_receipt(request,V37Status::Stale,revision,revision,Default::default()));}
        run.evidence.verify_live(&mut self.connection,self.root,&self.owner,&operation,claim.revision).map_err(OrchestrationError::V37StoreFailure)?;
        let pin=runtime::current_instance_pin(&self.connection,&claim.instance_id).map_err(|error|OrchestrationError::V37StoreFailure(format!("native capability pin: {error:?}")))?;
        let digest=crate::store::digest::sha256_hex(&request.raw_bytes);
        let mut cursor=None;
        let mut seen=BTreeSet::new();
        let mut features=Vec::new();
        let mut page=0u64;
        loop {
            let step=format!("feature-{}-{page}",&digest[..40]);
            let reply=self.native_feature_rpc(&key,&step,cursor)?;
            match reply {
                Some(Reply::FeaturePage {features:values,next_cursor,..})=> {
                    features.extend(values);
                    if REQUIRED.iter().all(|(name,_)|features.iter().any(|(actual,_)|actual==name)) {break;}
                    let Some(next)=next_cursor else {break;};
                    if !seen.insert(next.clone()) {return Err(OrchestrationError::Invalid("native feature pagination repeated cursor"));}
                    cursor=Some(next);
                    page=page.checked_add(1).ok_or(OrchestrationError::Invalid("native feature page overflow"))?;
                }
                other=>return Ok(encode_receipt(request,V37Status::Unknown,revision,revision,BTreeMap::from([(JsonString::from_str("error"),text(&format!("native feature response: {other:?}")))]))),
            }
        }
        let flags=match project_flags(&features) {
            Ok(flags)=>flags,
            Err(error)=> {
                // Cloud test assertions must retain the error already returned
                // by the real operation, rather than printing status alone.
                #[cfg(test)]
                eprintln!("native loaded thread feature result: {error:?}; original feature page values: {features:?}");
                return Ok(encode_receipt(request,V37Status::Unknown,revision,revision,BTreeMap::from([(JsonString::from_str("error"),text(&format!("native capability: {error:?}")))])));
            }
        };
        let result=BTreeMap::from([
            (JsonString::from_str("generation"),text(&generation)),
            (JsonString::from_str("threadId"),text(&thread)),
            (JsonString::from_str("driverId"),text(&pin.driver_id)),
            (JsonString::from_str("version"),text(&pin.version)),
            (JsonString::from_str("binaryDigest"),text(&pin.digest)),
            (JsonString::from_str("processOperationId"),text(&operation)),
            (JsonString::from_str("loadedThreadFeatures"),Json::Object(flags)),
            (JsonString::from_str("capabilities"),Json::Object(BTreeMap::from([
                (JsonString::from_str("resume"),text("SOURCE_PRESENT_RUNTIME_UNVERIFIED")),
                (JsonString::from_str("inTurnSteer"),text("SOURCE_PRESENT_RUNTIME_UNVERIFIED")),
                (JsonString::from_str("appendWithoutTurn"),text("SOURCE_PRESENT_RUNTIME_UNVERIFIED")),
                (JsonString::from_str("nativeQuestionCard"),text("CONFIGURED_MODEL_BEHAVIOUR_NOT_RUN")),
                (JsonString::from_str("manualCompaction"),text("SOURCE_PRESENT_RUNTIME_UNVERIFIED")),
                (JsonString::from_str("memoryOffLaunch"),text("LOADED_THREAD_MEMORIES_FALSE")),
            ]))),
            (JsonString::from_str("evidenceBasis"),text("NATIVE_LOADED_THREAD_FEATURE_RESPONSE")),
            (JsonString::from_str("modelBehaviour"),text("NOT_RUN")),
        ]);
        let signature=Json::Object(result).canonical();
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let read=(||->Result<Vec<u8>> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let now=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),&key.0,&seat,&key.1)
                .map_err(|error|OrchestrationError::V37StoreFailure(format!("native capability current claim: {error:?}")))?
                .ok_or(OrchestrationError::AccessDenied)?;
            if now!=claim {return Err(OrchestrationError::OperationConflict);}
            let prior=Statement::prepare(self.connection.as_ptr(),"SELECT request_bytes,receipt_bytes FROM main.v37_ledger_receipt WHERE family='K-SESSION' AND domain_id=?1 AND request_id=?2")?;
            prior.bind_text(1,&key.0)?;prior.bind_text(2,&request.request_id)?;
            if prior.step_row()? {
                if prior.column_text(0)?.as_bytes()!=request.raw_bytes {return Ok(encode_receipt(request,V37Status::Conflict,revision,revision,Default::default()));}
                let bytes=prior.column_text(1)?.into_bytes();drop(prior);
                let old=crate::store::session_transport::decode_receipt(&bytes).map_err(|error|OrchestrationError::V37StoreFailure(format!("native capability receipt: {error:?}")))?;
                let original=old.into_result();
                let unchanged=Json::Object(original).canonical()==signature;
                let Json::Object(fields)=crate::store::atomic::Parser::parse(&signature)? else {return Err(OrchestrationError::Invalid("native capability result"));};
                return Ok(encode_receipt(request,if unchanged {V37Status::Replayed} else {V37Status::Stale},revision,revision,if unchanged {fields} else {Default::default()}));
            }
            drop(prior);
            let Json::Object(fields)=crate::store::atomic::Parser::parse(&signature)? else {return Err(OrchestrationError::Invalid("native capability result"));};
            let bytes=encode_receipt(request,V37Status::Applied,revision,revision,fields);
            if bytes.len()+1>crate::ipc::MAX_FRAME_BYTES {return Err(OrchestrationError::Invalid("native capability replay bound"));}
            let insert=Statement::prepare(self.connection.as_ptr(),"INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes) VALUES('K-SESSION',?1,?2,?3,?4)")?;
            insert.bind_text(1,&key.0)?;insert.bind_text(2,&request.request_id)?;insert.bind_blob(3,&request.raw_bytes)?;insert.bind_blob(4,&bytes)?;insert.step_done()?;
            Ok(bytes)
        })();
        match read {
            Ok(bytes)=> {self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(bytes)},
            Err(primary)=> {if let Err(rollback)=self.connection.execute("ROLLBACK") {return Err(OrchestrationError::V37StoreFailure(format!("native capability: {primary:?}; rollback: {rollback:?}")));}Err(primary)},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_flags_are_original_values_not_presence_or_defaults() {
        let values=REQUIRED.iter().map(|(name,value)|(name.to_string(),*value)).collect::<Vec<_>>();
        assert!(project_flags(&values).is_ok());
        assert!(project_flags(&values[..2]).is_err());
        for index in 0..values.len() {let mut wrong=values.clone();wrong[index].1=!wrong[index].1;assert!(project_flags(&wrong).is_err());}
        let mut duplicate=values.clone();duplicate.push(values[0].clone());assert!(project_flags(&duplicate).is_err());
    }
}
