//! Read only H's original process and A's original vendor response. A new H
//! generation is not itself evidence that the vendor cache changed or survived.
use super::*;
use crate::store::session_transport::{self, rpc_journal};

struct ObservedCache { id:String, receipt:String }

fn unhex(value:&str)->Result<Vec<u8>> {
    if value.len()%2!=0 {return Err(SideError::Corrupt("odd original hex".into()));}
    value.as_bytes().chunks_exact(2).map(|pair| {
        let text=std::str::from_utf8(pair).map_err(|error|SideError::Corrupt(format!("original hex UTF-8: {error}")))?;
        u8::from_str_radix(text,16).map_err(|error|SideError::Corrupt(format!("original hex digit: {error}")))
    }).collect()
}

fn observed(db:&VerifiedDatabaseConnection<'_>,s:&Side,session:&str,generation:&str,
    instance:&str,operation:&str,old:bool)->Result<Option<ObservedCache>> {
    let episode=Statement::prepare(db.as_ptr(),"SELECT e.request_id,e.raw_hex,COALESCE(e.old_generation,''),e.phase,
        COALESCE(e.stop_fact_id,''),c.ticket,c.custodian_nonce,c.state,COALESCE(c.stop_proof_hash,''),i.driver_id
        FROM main.gogoke_v37_h_process_episode e
        JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id AND g.session_id=e.session_id
          AND g.generation=e.generation AND g.request_id=e.request_id AND g.process_operation_id=e.process_operation_id
        JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id
          AND c.domain_id=e.domain_id AND c.generation=e.generation
        JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
        WHERE e.domain_id=?1 AND e.session_id=?2 AND e.generation=?3 AND e.instance_id=?4
          AND e.process_operation_id=?5 AND e.seat_id=?6 AND e.seat_incarnation=?7")?;
    for (index,value) in [s.domain_id.as_str(),session,generation,instance,operation,
        s.seat_id.as_str(),s.seat_incarnation.as_str()].iter().enumerate() {episode.bind_text((index+1) as i32,value)?;}
    if !episode.step_row()? {return Ok(None);}
    let fields=(0..10).map(|index|episode.column_text(index)).collect::<std::result::Result<Vec<_>,_>>()?;
    if episode.step_row()? {return Err(SideError::Conflict);}
    drop(episode);
    let (open_id,raw,prior,phase,stop,ticket,nonce,custody,proof,driver)=
        (&fields[0],&fields[1],&fields[2],&fields[3],&fields[4],&fields[5],&fields[6],&fields[7],&fields[8],&fields[9]);
    if old {
        if phase!="STOPPED" || custody!="STOPPED" || stop.is_empty() || stop!=proof {return Ok(None);}
    } else if phase!="ACTIVE" || custody!="ACTIVE" {return Ok(None);}
    let original=session_transport::decode_request(&unhex(raw)?)
        .map_err(|error|SideError::Corrupt(format!("original H open/resume: {error:?}")))?;
    if original.family!="K-SESSION" || original.domain_id!=s.domain_id || original.target_id!=session
        || original.request_id!=open_id.as_str() ||
        (prior.is_empty() && original.operation!="open") ||
        (!prior.is_empty() && !matches!(original.operation.as_str(),"resume"|"compact"|"renew-session")) {
        return Err(SideError::Conflict);
    }
    if matches!(original.operation.as_str(),"compact"|"renew-session") {
        if session!=s.session_id || prior!=&s.binding_generation {return Err(SideError::Conflict);}
        let change=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_h_generation_change
            WHERE domain_id=?1 AND request_id=?2 AND raw_hex=?3 AND operation=?4
              AND session_id=?5 AND old_generation=?6 AND old_process_operation_id=?7
              AND seat_id=?8 AND stage IN ('OLD_STOPPED','APPLIED')")?;
        for (index,value) in [s.domain_id.as_str(),open_id.as_str(),raw.as_str(),
            original.operation.as_str(),session,prior.as_str(),s.binding_process_operation_id.as_str(),
            s.seat_id.as_str()].iter().enumerate() {change.bind_text((index+1) as i32,value)?;}
        if !change.step_row()? || change.step_row()? {return Ok(None);}
    }
    if prior.is_empty() {
        let operation_row=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_h_operation
            WHERE domain_id=?1 AND request_id=?2 AND session_id=?3 AND raw_hex=?4
              AND operation='open' AND status='APPLIED'")?;
        for (index,value) in [s.domain_id.as_str(),open_id.as_str(),session,raw.as_str()].iter().enumerate() {
            operation_row.bind_text((index+1) as i32,value)?;
        }
        if !operation_row.step_row()? || operation_row.step_row()? {return Ok(None);}
    }
    let step=if prior.is_empty() {"thread-start".to_owned()}
        else if driver=="codex" {format!("{operation}-thread-resume")}
        else {format!("{operation}-session-resume")};
    let source=Statement::prepare(db.as_ptr(),"SELECT r.source_epoch,r.source_cursor,s.command_hex,hex(r.raw_bytes)
        FROM main.gogoke_v37_rpc_steps s JOIN main.v37_ledger_raw_source r
          ON r.operation_id=s.process_operation_id AND r.source_epoch=s.source_epoch
          AND r.source_cursor=s.source_cursor AND r.process_ticket=s.ticket
          AND r.custodian_nonce=s.custodian_nonce AND r.domain_id=s.domain_id
          AND r.session_id=s.session_id AND r.generation=s.generation
        WHERE s.domain_id=?1 AND s.session_id=?2 AND s.generation=?3
          AND s.process_operation_id=?4 AND s.open_request_id=?5 AND s.ticket=?6
          AND s.custodian_nonce=?7 AND s.step_id=?8 AND s.phase='OBSERVED'
          AND r.state='NO_EVENT' AND r.no_event_reason=?9")?;
    let reason=if driver=="codex" {"CODEX_RPC_RESPONSE"} else {"ACP_RPC_RESPONSE"};
    for (index,value) in [s.domain_id.as_str(),session,generation,operation,open_id,
        ticket,nonce,&step,reason].iter().enumerate() {source.bind_text((index+1) as i32,value)?;}
    if !source.step_row()? {return Ok(None);}
    let observed=(0..4).map(|index|source.column_text(index)).collect::<std::result::Result<Vec<_>,_>>()?;
    if source.step_row()? {return Err(SideError::Conflict);}
    if !matches!(driver.as_str(),"codex"|"opencode"|"grok") {return Ok(None);}
    let cache=rpc_journal::observed_thread_id(db,&s.domain_id,session,operation,generation,
        open_id,ticket,nonce).map_err(|error|SideError::Corrupt(format!("original provider session: {error:?}")))?;
    if cache.is_empty() {return Err(SideError::Conflict);}
    Ok(Some(ObservedCache {id:format!("{driver}:{instance}:{cache}"),
        receipt:format!("{operation}:{}:{}",observed[0],observed[1])}))
}

/// Suitable as `rebind_current`'s trusted closure. A missing original source
/// remains Unknown; no synthetic continuity or new provider send is created.
pub(crate) fn read_current_cache_continuity(db:&VerifiedDatabaseConnection<'_>,s:&Side,
    live:&CurrentBinding)->Result<CacheContinuity> {
    let Some(old)=observed(db,s,&s.session_id,&s.binding_generation,&s.binding_instance_id,
        &s.binding_process_operation_id,true)? else {return Ok(CacheContinuity::Unknown)};
    let Some(new)=observed(db,s,&live.session_id,&live.generation,&live.instance_id,
        &live.process_operation_id,false)? else {return Ok(CacheContinuity::Unknown)};
    let fact=NativeCacheFact {receipt_id:format!("{} -> {}",old.receipt,new.receipt),
        old_session_id:s.session_id.clone(),old_generation:s.binding_generation.clone(),
        old_instance_id:s.binding_instance_id.clone(),old_process_operation_id:s.binding_process_operation_id.clone(),
        new_session_id:live.session_id.clone(),new_generation:live.generation.clone(),
        new_instance_id:live.instance_id.clone(),new_process_operation_id:live.process_operation_id.clone(),
        old_cache_id:old.id,new_cache_id:new.id};
    if fact.old_cache_id==fact.new_cache_id && fact.old_instance_id==fact.new_instance_id {
        Ok(CacheContinuity::Preserved(fact))
    } else {Ok(CacheContinuity::Replaced(fact))}
}
