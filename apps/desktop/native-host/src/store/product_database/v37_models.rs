//! USER metadata read from the already bound Codex H process. Each page is
//! retained by H as original OBSERVED RPC bytes before F accepts a model.
use super::*;
use crate::store::seat::NativeOrigin;
use crate::store::session_transport::{runtime,codex_rpc::Reply};
use std::collections::BTreeSet;
use std::time::{SystemTime,UNIX_EPOCH};

fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}

impl<'root> ProductDatabase<'root> {
    pub(super) fn dispatch_native_model_list(&mut self,request:&V37Request)->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        if request.payload.len()!=1 {
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        let generation=user_payload_string(request,"generation")?;
        let key=(request.domain_id.clone(),request.target_id.clone());
        let Some(run)=self.native_sessions.get(&key) else {
            return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                request.expected_revision,Default::default()));
        };
        if run.evidence.driver_id()!="codex" {
            return Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        let seat=run.evidence.seat_id().to_owned();
        let operation=run.operation_id.clone();
        let claim=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &key.0,&seat,&key.1)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("model claim: {error:?}")))?
            .ok_or(OrchestrationError::AccessDenied)?;
        let revision=u64::try_from(claim.revision)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("model revision: {error}")))?;
        if claim.generation!=generation || run.custody.binding.generation!=generation {
            return Ok(encode_receipt(request,V37Status::Conflict,revision,revision,Default::default()));
        }
        if revision!=request.expected_revision {
            return Ok(encode_receipt(request,V37Status::Stale,revision,revision,Default::default()));
        }
        run.evidence.verify_live(&mut self.connection,self.root,&self.owner,&operation,claim.revision)
            .map_err(OrchestrationError::V37StoreFailure)?;
        // An identical durable request never triggers another vendor RPC.
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_bytes,receipt_bytes FROM main.v37_ledger_receipt WHERE family='K-SESSION' AND domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1,&key.0)?;prior.bind_text(2,&request.request_id)?;
        if prior.step_row()? {
            let same=prior.column_text(0)?.as_bytes()==request.raw_bytes;
            let bytes=prior.column_text(1)?.into_bytes();
            return Ok(if same {bytes} else {encode_receipt(request,V37Status::Conflict,
                revision,revision,Default::default())});
        }
        drop(prior);
        let digest=crate::store::digest::sha256_hex(&request.raw_bytes);
        let mut expected=None;
        let mut seen=BTreeSet::new();
        let mut steps=Vec::new();
        let mut complete=false;
        for page in 0..64 {
            let step=format!("models-{}-{page}",&digest[..40]);
            let (actual_step,reply)=self.native_model_list_rpc(&key,&step,expected.clone())?;
            match reply {
                Some(Reply::ModelPage {next_cursor,..})=> {
                    steps.push(actual_step);
                    match next_cursor {
                        None=>{complete=true;break;},
                        Some(cursor) if seen.insert(cursor.clone())=>expected=Some(cursor),
                        Some(_)=>return Err(OrchestrationError::Invalid("model list repeated cursor")),
                    }
                }
                _=>return Ok(encode_receipt(request,V37Status::Unknown,revision,revision,
                    BTreeMap::from([(JsonString::from_str("error"),text("model/list response unavailable"))]))),
            }
        }
        if !complete {
            return Ok(encode_receipt(request,V37Status::Unknown,revision,revision,
                BTreeMap::from([(JsonString::from_str("error"),text("model/list pagination incomplete"))])));
        }
        let observed_at=SystemTime::now().duration_since(UNIX_EPOCH)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("model clock: {error}")))?
            .as_millis().to_string();
        instance::record_verified_models_from_original_rpc_source(&mut self.connection,
            &claim.instance_id,&key.0,&key.1,&steps,&observed_at)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("model original source: {error:?}")))?;
        let evidence=instance::read_instance_evidence(&self.connection,&claim.instance_id)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("model evidence: {error:?}")))?
            .ok_or(OrchestrationError::AccessDenied)?;
        let mut result=BTreeMap::new();
        result.insert(JsonString::from_str("instanceId"),text(&claim.instance_id));
        result.insert(JsonString::from_str("modelsSource"),
            evidence.models_source.as_deref().map(text).unwrap_or(Json::Null));
        result.insert(JsonString::from_str("modelsObservedAt"),text(&observed_at));
        result.insert(JsonString::from_str("pageCount"),text(&steps.len().to_string()));
        let bytes=encode_receipt(request,V37Status::Applied,revision,revision,result);
        if bytes.len()+1>crate::ipc::MAX_FRAME_BYTES {
            return Err(OrchestrationError::Invalid("model list receipt bound"));
        }
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let save=(||->Result<()> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let current=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
                &key.0,&seat,&key.1)
                .map_err(|error|OrchestrationError::V37StoreFailure(format!("model current claim: {error:?}")))?
                .ok_or(OrchestrationError::AccessDenied)?;
            if current!=claim {return Err(OrchestrationError::OperationConflict);}
            let write=Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes) VALUES('K-SESSION',?1,?2,?3,?4)")?;
            write.bind_text(1,&key.0)?;write.bind_text(2,&request.request_id)?;
            write.bind_blob(3,&request.raw_bytes)?;write.bind_blob(4,&bytes)?;write.step_done()?;
            Ok(())
        })();
        match save {
            Ok(())=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(bytes)},
            Err(error)=>{self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;Err(error)},
        }
    }
}
