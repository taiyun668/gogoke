//! Owner-private display facts from the current seat generation's actual H/A
//! source. These facts grant no input, health action, login or instance change.
use super::*;
use crate::store::atomic::Parser;
use crate::store::ledger;
use crate::store::session_transport::{codex_rpc,provider_evidence::stream_json};

fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}
fn field<'a>(fields:&'a BTreeMap<JsonString,Json>,name:&str)->Option<&'a Json> {
    fields.get(&JsonString::from_str(name))
}
fn value(fields:&BTreeMap<JsonString,Json>,name:&str)->Option<String> {
    match field(fields,name) {Some(Json::String(value))=>value.to_well_formed_string(),_=>None}
}
fn error_text(value:&Json)->String {
    match value {Json::String(item)=>item.to_well_formed_string().filter(|text|!text.trim().is_empty())
            .unwrap_or_else(||value.canonical()),
        _=>value.canonical()}
}

// None means no lifecycle fact. Some(None) is a successful terminal, which
// clears the same session's prior error. Assistant prose never sets an issue.
fn cli_failure(driver:&str,raw:&[u8])->Result<Option<Option<String>>> {
    let source=std::str::from_utf8(raw).map_err(|error|OrchestrationError::V37StoreFailure(
        format!("original CLI status UTF-8: {error}")))?;
    let Json::Object(root)=Parser::parse(source)? else {return Err(OrchestrationError::Invalid("original CLI status object"));};
    if matches!(driver,"codex"|"opencode"|"grok")&&field(&root,"id").is_some()
        &&field(&root,"method").is_none() {
        if let Some(error)=field(&root,"error") {return Ok(Some(Some(error_text(error))));}
        if matches!(driver,"opencode"|"grok") {
            if let Some(Json::Object(result))=field(&root,"result") {
                if value(result,"stopReason").as_deref()==Some("end_turn") {return Ok(Some(None));}
            }
        }
    }
    if driver=="codex"&&value(&root,"method").as_deref()==Some("turn/completed") {
        let codex_rpc::Reply::TurnNotification {status,..}=codex_rpc::decode(raw,None)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("original Codex terminal status: {error:?}")))?
            else {return Err(OrchestrationError::Invalid("original Codex terminal status"));};
        if status==codex_rpc::TurnStatus::Completed {return Ok(Some(None));}
        if status!=codex_rpc::TurnStatus::Failed {return Ok(None);}
        let Some(Json::Object(params))=field(&root,"params") else {return Err(OrchestrationError::Invalid("original Codex terminal params"));};
        let Some(Json::Object(turn))=field(params,"turn") else {return Err(OrchestrationError::Invalid("original Codex terminal turn"));};
        let error=field(turn,"error").ok_or(OrchestrationError::Invalid("original Codex terminal error"))?;
        return Ok(Some(Some(error_text(error))));
    }
    if driver=="claude"&&value(&root,"type").as_deref()==Some("result") {
        let stream_json::ClaudeData::Result {subtype,is_error,..}=stream_json::decode_claude_line(raw)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("original Claude terminal status: {error:?}")))?
            else {return Err(OrchestrationError::Invalid("original Claude terminal status"));};
        if !is_error&&subtype=="success" {return Ok(Some(None));}
        if is_error {
            let reason=field(&root,"errors").or_else(||field(&root,"error"))
                .or_else(||field(&root,"result")).map(error_text).unwrap_or(subtype);
            return Ok(Some(Some(reason)));
        }
    }
    Ok(None)
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn read_user_instance_runtime_issues(&self,instance:&str,driver:&str)->Result<Vec<Json>> {
        let episodes=Statement::prepare(self.connection.as_ptr(),
            "SELECT e.domain_id,e.session_id,e.seat_id,e.generation,e.process_operation_id,
                    c.ticket,c.custodian_nonce
               FROM main.gogoke_v37_h_process_episode e
               JOIN main.gogoke_v37_h_claim h ON h.domain_id=e.domain_id AND h.session_id=e.session_id
                 AND h.generation=e.generation AND h.process_operation_id=e.process_operation_id
                 AND h.instance_id=e.instance_id AND h.state<>'RELEASED'
               JOIN main.gogoke_coordination_process_custody c ON c.domain_id=e.domain_id
                 AND c.operation_id=e.process_operation_id AND c.generation=e.generation
              WHERE e.instance_id=?1 AND e.phase IN ('ACTIVE','UNKNOWN','STOPPED')
              ORDER BY e.domain_id,e.seat_id,e.session_id")?;
        episodes.bind_text(1,instance)?;
        let mut issues=Vec::new();
        while episodes.step_row()? {
            let domain=episodes.column_text(0)?;let session=episodes.column_text(1)?;
            let seat=episodes.column_text(2)?;let generation=episodes.column_text(3)?;
            let relationship=crate::store::session_transport::session_binding::current_relationship(
                &self.connection,&domain,&session).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("instance issue current relationship: {error:?}")))?;
            if relationship.as_ref().map_or(true,|current|
                current.seat_id!=seat || current.instance_id!=instance || current.session_generation!=generation) {
                continue;
            }
            let operation=episodes.column_text(4)?;let ticket=episodes.column_text(5)?;
            let epoch=episodes.column_text(6)?;
            let sources=Statement::prepare(self.connection.as_ptr(),
                "SELECT source_cursor FROM main.v37_ledger_raw_source
                  WHERE operation_id=?1 AND source_epoch=?2 AND domain_id=?3 AND session_id=?4
                    AND generation=?5 AND process_ticket=?6 AND custodian_nonce=?2
                  ORDER BY CAST(source_cursor AS INTEGER)")?;
            for (index,item) in [operation.as_str(),epoch.as_str(),domain.as_str(),session.as_str(),
                generation.as_str(),ticket.as_str()].iter().enumerate() {sources.bind_text((index+1) as i32,item)?;}
            let mut latest=None;
            while sources.step_row()? {
                let cursor=sources.column_text(0)?;
                let source=ledger::read_captured_raw_source(&self.connection,&operation,&epoch,&cursor)?
                    .ok_or(OrchestrationError::OperationConflict)?;
                if let Some(fact)=cli_failure(driver,&source.raw_bytes)? {
                    latest=fact.map(|reason|(reason,cursor));
                }
            }
            if let Some((reason,cursor))=latest {
                if reason.is_empty() {return Err(OrchestrationError::Invalid("original CLI error reason empty"));}
                issues.push(Json::Object(BTreeMap::from([
                    (JsonString::from_str("seatId"),text(&seat)),(JsonString::from_str("sessionId"),text(&session)),
                    (JsonString::from_str("generation"),text(&generation)),(JsonString::from_str("reason"),text(&reason)),
                    (JsonString::from_str("sourceEpoch"),text(&epoch)),(JsonString::from_str("sourceCursor"),text(&cursor)),
                ])));
            }
        }
        Ok(issues)
    }
}
