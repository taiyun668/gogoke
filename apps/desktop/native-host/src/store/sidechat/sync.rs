use super::*;
use crate::store::session_transport::{self, JournalState, StdinJournalKey};

pub(crate) enum SyncMode { Append, Question }
impl SyncMode { fn name(&self)->&'static str { match self {Self::Append=>"APPEND",Self::Question=>"QUESTION"} } }

fn load_sync(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str)->Result<Option<(Sync,String)>> {
    let row=Statement::prepare(db.as_ptr(),"SELECT side_id,mode,generation,epoch,after_cursor,through_cursor,state,native_receipt_id,request_digest,session_id,process_operation_id FROM main.gogoke_v37_side_sync WHERE domain_id=?1 AND sync_id=?2")?;
    row.bind_text(1,domain)?;row.bind_text(2,id)?;
    if !row.step_row()? {return Ok(None);}
    Ok(Some((Sync {sync_id:id.into(),side_id:row.column_text(0)?,mode:row.column_text(1)?,generation:row.column_text(2)?,epoch:row.column_text(3)?,
        after:number(row.column_text(4)?)?,through:number(row.column_text(5)?)?,state:row.column_text(6)?,native_receipt_id:row.column_text(7)?,
        session_id:row.column_text(9)?,process_operation_id:row.column_text(10)?,may_submit:false},row.column_text(8)?)))
}

/// The raw request is the exact H send/append that will carry the references.
/// Root assembles reference material and the Owner's question first, then calls
/// this before H prepare -> beginCommitted -> completion. No turn is invented.
/// Append proof must be a native observation for the current session generation
/// AND program pin; a schema, vendor name, older experiment or model claim fails.
pub(crate) fn begin_sync(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,domain:&str,id:&str,
    request:&V37Request,mode:SyncMode,through:u64,
    append_verified:impl FnOnce(&VerifiedDatabaseConnection<'_>,&str,&str)->Result<bool>)->Result<Sync> {
    transact(db,|db| {
        authority::check_owner_in_current_transaction(db,owner)?;
        let s=side(db,domain,id)?;check_history(db,&s)?;
        if request.family!="K-SESSION" || request.domain_id!=domain {
            return Err(SideError::Denied);
        }
        let operation=match mode {SyncMode::Append=>"append-without-turn",SyncMode::Question=>"send"};
        if request.operation!=operation || request.payload.len()!=2 || field(request,"body")?.is_empty() {
            return Err(SideError::Invalid("sync request"));
        }
        if let Some((prior,raw))=load_sync(db,domain,&request.request_id)? {
            if raw!=crate::store::digest::sha256_hex(&request.raw_bytes) || prior.side_id!=id || prior.mode!=mode.name() || prior.through!=through || prior.session_id!=request.target_id || prior.generation!=field(request,"generation")? {return Err(SideError::Conflict);}
            return Ok(prior); // Even PREPARED after restart never grants a resend.
        }
        if unresolved(db,domain,id)? {return Err(SideError::Unknown);}
        if s.state!="ACTIVE" {return Err(SideError::Conflict);}
        let live=current_binding(db,&s)?;
        // Rebinding precedes assembly of the input; begin cannot silently reset
        // a range after Root has already constructed a question from it.
        if live.session_id!=s.session_id || live.source_session_id!=s.source_session_id || live.generation!=s.binding_generation ||
            live.instance_id!=s.binding_instance_id || live.process_operation_id!=s.binding_process_operation_id {return Err(SideError::Unknown);}
        let generation=live.generation;
        if request.target_id!=s.session_id || field(request,"generation")?!=generation {return Err(SideError::Denied);}
        if through<s.synced_cursor || through>s.cursor {return Err(SideError::Stale);}
        if through==s.synced_cursor && matches!(mode,SyncMode::Append) {return Err(SideError::Conflict);}
        // End exactly on a persisted pending boundary, never discard part of an
        // unmaterialized page. Pending ranges are contiguous by construction.
        if through>s.synced_cursor {
            let boundary=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_side_pending WHERE domain_id=?1 AND side_id=?2 AND epoch=?3 AND through_cursor=?4")?;
            boundary.bind_text(1,domain)?;boundary.bind_text(2,id)?;boundary.bind_text(3,&s.epoch)?;boundary.bind_text(4,&through.to_string())?;
            if !boundary.step_row()? {return Err(SideError::Conflict);}
        }
        if matches!(mode,SyncMode::Append) && !append_verified(db,&s.session_id,&generation)? {return Err(SideError::Unknown);}
        // Starting a side chat still needs H's held admission, no local cap.
        let live=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND state='COMMITTED'")?;
        live.bind_text(1,domain)?;live.bind_text(2,&s.session_id)?;live.bind_text(3,&generation)?;
        if !live.step_row()? {return Err(SideError::Denied);}drop(live);
        let row=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_side_sync(domain_id,sync_id,side_id,request_digest,mode,generation,epoch,after_cursor,through_cursor,state,native_receipt_id,session_id,process_operation_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'PREPARED','',?10,?11)")?;
        for (i,v) in [domain,&request.request_id,id,&crate::store::digest::sha256_hex(&request.raw_bytes),mode.name(),&generation,&s.epoch,&s.synced_cursor.to_string(),&through.to_string(),&s.session_id,&live.process_operation_id].iter().enumerate() {row.bind_text((i+1) as i32,v)?;}
        row.step_done()?;
        Ok(Sync {sync_id:request.request_id.clone(),side_id:id.into(),mode:mode.name().into(),generation,session_id:s.session_id,process_operation_id:live.process_operation_id,epoch:s.epoch,after:s.synced_cursor,through,
            state:"PREPARED".into(),native_receipt_id:String::new(),may_submit:true})
    })
}

/// Reconcile only the original trusted H journal; caller-supplied ACKs and a
/// successful pipe write cannot move the side's sync cursor. No second send.
pub(crate) fn settle_sync(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,domain:&str,sync_id:&str)->Result<Sync> {
    transact(db,|db| {
        authority::check_owner_in_current_transaction(db,owner)?;
        let (mut sync,raw)=load_sync(db,domain,sync_id)?.ok_or(SideError::Conflict)?;
        let s=side(db,domain,&sync.side_id)?;check_history(db,&s)?;
        if matches!(sync.state.as_str(),"DELIVERED"|"FAILED") {return Ok(sync);}
        // Obtain the original ticket from H, then use H's own custody read.
        let ticket=Statement::prepare(db.as_ptr(),"SELECT ticket FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND request_id=?2")?;
        ticket.bind_text(1,domain)?;ticket.bind_text(2,sync_id)?;
        let original=if ticket.step_row()? {Some(ticket.column_text(0)?)} else {None};drop(ticket);
        let no_original=original.is_none();
        let record=match original {
            Some(ticket)=>session_transport::read_stdin_journal(db,&StdinJournalKey {domain_id:domain,request_id:sync_id,session_id:&sync.session_id,ticket:&ticket,generation:&sync.generation})
                .map_err(|e|SideError::Corrupt(format!("side H receipt: {e:?}")))?,
            None=>None,
        };
        sync.state="UNKNOWN".into();
        // H must durably prepare its original journal before writing a byte.
        // Only its absence plus this exact generation's native StopFact proves
        // a D intent never reached H. A present uncertain H row stays UNKNOWN.
        if no_original {
            if let Some(proof)=stop_proof(db,&s,&sync.session_id,&sync.generation,&sync.process_operation_id)? {
                sync.state="FAILED".into();sync.native_receipt_id=proof;
            }
        }
        if let Some(record)=record {
            if crate::store::digest::sha256_hex(&record.request_bytes)!=raw || record.process_operation_id!=sync.process_operation_id {return Err(SideError::Conflict);}
            if record.state==JournalState::Receipted {
                let receipt=decode_receipt(record.receipt_bytes.as_deref().ok_or(SideError::Unknown)?)
                    .map_err(|e|SideError::Corrupt(format!("side H receipt decode: {e:?}")))?;
                let expected=if sync.mode=="APPEND" {"append-without-turn"} else {"send"};
                if receipt.family!="K-SESSION" || receipt.operation!=expected || receipt.request_id!=sync_id || receipt.target_id!=sync.session_id {
                    return Err(SideError::Conflict);
                }
                match receipt.status {
                    V37Status::Applied|V37Status::Replayed=>{
                        let body=receipt.into_result();
                        if body.get(&JsonString::from_str("createdTurn"))!=Some(&Json::Bool(sync.mode=="QUESTION")) ||
                            body.get(&JsonString::from_str("generation"))!=Some(&text(&sync.generation)) ||
                            (sync.mode=="APPEND" && body.get(&JsonString::from_str("deliveryBasis"))!=Some(&text("NATIVE_INJECT_ITEMS_ACK"))) {
                            return Err(SideError::Unknown);
                        }
                        let Some(Json::String(receipt_id))=body.get(&JsonString::from_str("receiptId")) else {return Err(SideError::Unknown);};
                        sync.native_receipt_id=receipt_id.to_well_formed_string().ok_or(SideError::Unknown)?;
                        required(&sync.native_receipt_id)?;sync.state="DELIVERED".into();
                    },
                    V37Status::Denied|V37Status::Failed|V37Status::Unsupported|V37Status::Conflict|V37Status::Stale=>{
                        sync.state="FAILED".into();sync.native_receipt_id=sync_id.into();
                    },
                    V37Status::Unknown=>{},
                }
            }
        }
        if sync.state=="DELIVERED" && sync.session_id==s.session_id && sync.generation==s.binding_generation && sync.process_operation_id==s.binding_process_operation_id {
            if s.epoch!=sync.epoch || s.synced_cursor!=sync.after || sync.through>s.cursor {return Err(SideError::Stale);}
            let revision=s.revision.checked_add(1).filter(|n|*n<=i64::MAX as u64).ok_or(SideError::Invalid("revision overflow"))?;
            let row=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_side_registry SET synced_cursor=?3,revision=?4 WHERE domain_id=?1 AND side_id=?2")?;
            row.bind_text(1,domain)?;row.bind_text(2,&s.side_id)?;row.bind_text(3,&sync.through.to_string())?;row.bind_text(4,&revision.to_string())?;row.step_done()?;drop(row);
            let rows=Statement::prepare(db.as_ptr(),"DELETE FROM main.gogoke_v37_side_pending WHERE domain_id=?1 AND side_id=?2 AND epoch=?3 AND CAST(through_cursor AS INTEGER)<=?4")?;
            rows.bind_text(1,domain)?;rows.bind_text(2,&s.side_id)?;rows.bind_text(3,&sync.epoch)?;rows.bind_i64(4,sync.through as i64)?;rows.step_done()?;
        }
        let row=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_side_sync SET state=?3,native_receipt_id=?4 WHERE domain_id=?1 AND sync_id=?2")?;
        row.bind_text(1,domain)?;row.bind_text(2,sync_id)?;row.bind_text(3,&sync.state)?;row.bind_text(4,&sync.native_receipt_id)?;row.step_done()?;
        Ok(sync)
    })
}
