//! Native storage integration on the real product opener. H/E source rows are
//! synthetic observations; this test is not a provider, StopFact or LPAC run.
use super::*;
use crate::root::RootLock;
use crate::store::same_open::route_b_test_guard;
use crate::store::seat::{self, CreateSeat, Kind, NativeOrigin, StoreTemplate};
use crate::store::session_transport::decode_request;
use crate::store::session::open_product_database;
use std::time::{SystemTime,UNIX_EPOCH};

fn request(verb:&str,id:&str,revision:u64)->V37Request {
    let payload=if verb=="create" {r#"{"sourceCursor":"0"}"#} else {"{}"};
    decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-SIDE","operation":"{verb}","requestId":"{id}","targetId":"sideA","domainId":"projectA","expectedRevision":"{revision}","payload":{payload}}}"#).as_bytes()).unwrap()
}
fn status(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,verb:&str,id:&str,rev:u64)->V37Status {
    decode_receipt(&execute(db,owner,&request(verb,id,rev),None).unwrap()).unwrap().status
}
fn append(db:&mut VerifiedDatabaseConnection<'_>,session:&str,seat:&str,tier:ledger::Tier,source:u64,event:&str) {
    ledger::record(db,&ledger::EventInput {event_id:event.into(),source_epoch:"syntheticStream".into(),source_cursor:source.to_string(),
        domain_id:"projectA".into(),seat_id:seat.into(),session_id:session.into(),tier,side_id:if tier==ledger::Tier::Side {Some("sideA".into())} else {None},
        occurred_at:"syntheticTime".into(),update_json:r#"{"content":{"text":"reference marker","type":"text"},"sessionUpdate":"agent_message_chunk"}"#.into()}).unwrap();
}
fn bind_current_fixture(db:&mut VerifiedDatabaseConnection<'_>,session:&str,seat_id:&str,purpose:ledger::SessionPurpose) {
    let seat=seat::get(db,"projectA",seat_id).unwrap().unwrap();let generation=seat.generation.to_string();
    let row=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_owner_binding VALUES(?1,'instanceA','projectA','SESSION',?1,?2,'ACTIVE')").unwrap();
    row.bind_text(1,session).unwrap();row.bind_text(2,&generation).unwrap();row.step_done().unwrap();drop(row);
    let row=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA',?1,'instanceA','syntheticHome',?1,?2,'COMMITTED',2,?3)").unwrap();
    row.bind_text(1,session).unwrap();row.bind_text(2,&generation).unwrap();row.bind_text(3,&format!("process{session}")).unwrap();row.step_done().unwrap();drop(row);
    let row=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_seat_binding VALUES('projectA',?1,?2,?3,?4)").unwrap();
    row.bind_text(1,session).unwrap();row.bind_text(2,seat_id).unwrap();row.bind_text(3,&seat.incarnation).unwrap();row.bind_text(4,&generation).unwrap();row.step_done().unwrap();drop(row);
    ledger::register_session(db,&ledger::SessionRegistration {domain_id:"projectA".into(),seat_id:seat_id.into(),session_id:session.into(),purpose,
        side_id:if purpose==ledger::SessionPurpose::SideChat {Some("sideA".into())} else {None}}).unwrap();
}
fn cache_fact(old:&Side,new:&CurrentBinding)->NativeCacheFact {
    NativeCacheFact {receipt_id:"syntheticNativeCacheReceipt".into(),old_session_id:old.session_id.clone(),old_generation:old.binding_generation.clone(),old_instance_id:old.binding_instance_id.clone(),
        new_session_id:new.session_id.clone(),new_generation:new.generation.clone(),new_instance_id:new.instance_id.clone(),old_process_operation_id:old.binding_process_operation_id.clone(),new_process_operation_id:new.process_operation_id.clone(),
        old_cache_id:"syntheticCacheOld".into(),new_cache_id:"syntheticCacheNew".into()}
}
fn old_ack_fixture(db:&mut VerifiedDatabaseConnection<'_>,s:&Side,send:&V37Request) {
    // Synthetic durable H/custody/RPC observations exercise original-key
    // reconciliation. They are not evidence of an actual CLI or native stop.
    let generation=field(send,"generation").unwrap();let operation=format!("process{}",send.target_id);let ticket="syntheticTicket";
    let row=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_coordination_process_custody VALUES(?1,?2,'syntheticNonce','101','202','syntheticImage','syntheticDigest','syntheticProfile','projectA',?3,'STOPPED','syntheticStop')").unwrap();
    row.bind_text(1,&operation).unwrap();row.bind_text(2,ticket).unwrap();row.bind_text(3,&generation).unwrap();row.step_done().unwrap();drop(row);
    let row=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES('projectA','syntheticOpen',?1,?2,'00',1,?3,'instanceA','syntheticHome',?1,'sideSeat',?4,'STOPPED','syntheticStop')").unwrap();
    row.bind_text(1,&send.target_id).unwrap();row.bind_text(2,&generation).unwrap();row.bind_text(3,&operation).unwrap();row.bind_text(4,&s.seat_incarnation).unwrap();row.step_done().unwrap();drop(row);
    let row=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_generation VALUES('projectA',?1,?2,'syntheticOpen',?3)").unwrap();
    row.bind_text(1,&send.target_id).unwrap();row.bind_text(2,&generation).unwrap();row.bind_text(3,&operation).unwrap();row.step_done().unwrap();drop(row);
    let mut receipt=encode_receipt(send,V37Status::Applied,send.expected_revision,send.expected_revision+1,BTreeMap::from([
        (JsonString::from_str("createdTurn"),Json::Bool(true)),(JsonString::from_str("generation"),text(&generation)),
        (JsonString::from_str("receiptId"),text("syntheticRpcReceipt")),(JsonString::from_str("turnId"),text("syntheticTurn"))]));receipt.push(b'\n');
    let row=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_stdin_journal(domain_id,request_id,operation,ticket,process_operation_id,custodian_nonce,session_id,generation,request_hex,phase,receipt_hex,receipt_status,expected_revision,receipt_previous_revision,receipt_revision) VALUES('projectA',?1,'send',?2,?3,'syntheticNonce',?4,?5,?6,'RECEIPTED',?7,'APPLIED',?8,?8,?9)").unwrap();
    for (i,v) in [send.request_id.as_str(),ticket,&operation,&send.target_id,&generation,&hex(&send.raw_bytes),&hex(&receipt),&send.expected_revision.to_string(),&(send.expected_revision+1).to_string()].iter().enumerate() {row.bind_text((i+1) as i32,v).unwrap();}
    row.step_done().unwrap();
}

#[test]
fn registry_ranges_and_unknown_sync_survive_actual_same_open_reopen() {
    let _guard=route_b_test_guard();
    let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path=std::env::temp_dir().join(format!("gogoke-side-store-{}-{nonce}",std::process::id()));
    std::fs::create_dir(&path).unwrap();let root=RootLock::acquire(&path).unwrap();let database=path.join("state.sqlite");
    let mut db=open_product_database(&root,&database).unwrap();
    let owner=authority::initialize_profile(&mut db,&root).unwrap();
    authority::initialize_process_custody_schema(&mut db).unwrap();initialize_schema(&mut db).unwrap();
    db.execute("INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','syntheticHome','syntheticIdentity','sha256:fixture','fixture','INSTALLED','LOGGED_OUT',1)").unwrap();
    seat::store_template(&mut db,NativeOrigin::user(&owner),StoreTemplate {domain_id:"projectA",template_id:"templateA",settings_json:br#"{"model":"gpt-6-sol"}"#}).unwrap();
    for (seat_id,session_id,purpose) in [("leadA","mainA",ledger::SessionPurpose::Work),("sideSeat","sideSession",ledger::SessionPurpose::SideChat),("otherSeat","otherSession",ledger::SessionPurpose::Work)] {
        seat::create(&mut db,NativeOrigin::user(&owner),CreateSeat {domain_id:"projectA",seat_id,template_id:"templateA",instance_id:Some("instanceA"),kind:Kind::Long,request_id:seat_id,request_bytes:seat_id.as_bytes()}).unwrap();
        let seat=seat::get(&db,"projectA",seat_id).unwrap().unwrap();
        let generation=seat.generation.to_string();
        let owner_row=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_owner_binding VALUES(?1,'instanceA','projectA','SESSION',?2,?3,'ACTIVE')").unwrap();
        owner_row.bind_text(1,session_id).unwrap();owner_row.bind_text(2,session_id).unwrap();owner_row.bind_text(3,&generation).unwrap();owner_row.step_done().unwrap();drop(owner_row);
        let claim=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA',?1,'instanceA','syntheticHome',?1,?2,'COMMITTED',2,?3)").unwrap();
        claim.bind_text(1,session_id).unwrap();claim.bind_text(2,&generation).unwrap();claim.bind_text(3,&format!("process{session_id}")).unwrap();claim.step_done().unwrap();drop(claim);
        let binding=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_seat_binding VALUES('projectA',?1,?2,?3,?4)").unwrap();
        binding.bind_text(1,session_id).unwrap();binding.bind_text(2,seat_id).unwrap();binding.bind_text(3,&seat.incarnation).unwrap();binding.bind_text(4,&generation).unwrap();binding.step_done().unwrap();drop(binding);
        ledger::register_session(&mut db,&ledger::SessionRegistration {domain_id:"projectA".into(),seat_id:seat_id.into(),session_id:session_id.into(),purpose,
            side_id:if purpose==ledger::SessionPurpose::SideChat {Some("sideA".into())} else {None}}).unwrap();
    }
    let create=request("create","createA",0);
    let binding=CreateBinding {source_seat_id:"leadA".into(),source_session_id:"mainA".into(),seat_id:"sideSeat".into(),session_id:"sideSession".into()};
    assert_eq!(decode_receipt(&execute(&mut db,&owner,&create,Some(&binding)).unwrap()).unwrap().status,V37Status::Applied);
    assert_eq!(status(&mut db,&owner,"archive","archiveA",1),V37Status::Applied);
    db.close_checked().unwrap();let mut db=open_product_database(&root,&database).unwrap();initialize_schema(&mut db).unwrap();
    assert_eq!(decode_receipt(&execute(&mut db,&owner,&create,None).unwrap()).unwrap().status,V37Status::Replayed);
    assert_eq!(status(&mut db,&owner,"read-thread","archivedRead",2),V37Status::Applied);
    assert_eq!(status(&mut db,&owner,"restore","restoreA",2),V37Status::Applied);
    let read=request("pending-delta","readBefore",3);execute(&mut db,&owner,&read,None).unwrap();
    append(&mut db,"mainA","leadA",ledger::Tier::Seat,1,"mainFirst");
    append(&mut db,"otherSession","otherSeat",ledger::Tier::Seat,1,"otherFirst");
    append(&mut db,"sideSession","sideSeat",ledger::Tier::Side,1,"ownTurn");
    append(&mut db,"mainA","leadA",ledger::Tier::Seat,2,"mainTail");
    let own_epoch=side(&db,"projectA","sideA").unwrap().epoch;
    let own_page=read_thread(&mut db,&owner,"projectA","sideA",&LedgerPosition {epoch:own_epoch,cursor:0},2).unwrap();
    assert_eq!(own_page.events.iter().map(|event|event.input.event_id.as_str()).collect::<Vec<_>>(),vec!["ownTurn"]);
    assert_eq!(own_page.cursor,4,"unrelated rows cannot consume the side transcript page");
    assert_eq!(decode_receipt(&execute(&mut db,&owner,&read,None).unwrap()).unwrap().status,V37Status::Stale);
    for cursor in 1..=4 {assert_eq!(collect(&mut db,&owner,"projectA","sideA",1).unwrap().cursor,cursor);}
    let unrelated=materialize(&mut db,&owner,"projectA","sideA",1,4,1).unwrap();
    assert!(unrelated.events.is_empty());assert_eq!(unrelated.cursor,2,"filtering cannot conceal page continuation");
    let own=materialize(&mut db,&owner,"projectA","sideA",2,4,1).unwrap();assert!(own.events.is_empty());assert_eq!(own.cursor,3);
    let tail=materialize(&mut db,&owner,"projectA","sideA",3,4,1).unwrap();assert_eq!(tail.events[0].input.event_id,"mainTail");
    let s=side(&db,"projectA","sideA").unwrap();
    assert_eq!(status(&mut db,&owner,"delete","liveDelete",s.revision),V37Status::Denied,"live H claim has no StopFact");
    let generation=current_binding(&db,&s).unwrap().generation;
    let send=decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"questionA","targetId":"sideSession","domainId":"projectA","expectedRevision":"2","payload":{{"generation":"{generation}","body":"synthetic explicit question"}}}}"#).as_bytes()).unwrap();
    let intent=begin_sync(&mut db,&owner,"projectA","sideA",&send,SyncMode::Question,4,|_,_,_|Ok(false)).unwrap();assert!(intent.may_submit);
    // Even before an H row exists, composing the same H body cannot change
    // the original User request identity by whitespace or a different body.
    let changed_origin=format!(" {}",std::str::from_utf8(&send.raw_bytes).unwrap());
    assert!(matches!(begin_sync_from_user(&mut db,&owner,"projectA","sideA",&send,
        changed_origin.as_bytes(),SyncMode::Question,4,|_,_,_|Ok(false)),Err(SideError::Conflict)));
    assert!(!begin_sync_from_user(&mut db,&owner,"projectA","sideA",&send,
        &send.raw_bytes,SyncMode::Question,4,|_,_,_|Ok(false)).unwrap().may_submit);
    let unknown=settle_sync(&mut db,&owner,"projectA","questionA").unwrap();assert_eq!(unknown.state,"UNKNOWN");
    db.close_checked().unwrap();let mut db=open_product_database(&root,&database).unwrap();initialize_schema(&mut db).unwrap();
    let retry=begin_sync(&mut db,&owner,"projectA","sideA",&send,SyncMode::Question,4,|_,_,_|Ok(false)).unwrap();assert!(!retry.may_submit);
    assert_eq!(side(&db,"projectA","sideA").unwrap().synced_cursor,0);
    let newer=decode_request(std::str::from_utf8(&send.raw_bytes).unwrap().replace("questionA","questionB").as_bytes()).unwrap();
    assert!(matches!(begin_sync(&mut db,&owner,"projectA","sideA",&newer,SyncMode::Question,4,|_,_,_|Ok(false)),Err(SideError::Unknown)));
    assert_eq!(status(&mut db,&owner,"archive","unknownArchive",s.revision),V37Status::Applied);
    assert_eq!(status(&mut db,&owner,"restore","unknownRestore",s.revision+1),V37Status::Applied);
    db.execute("UPDATE gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id='sideSession'").unwrap();
    db.execute("UPDATE gogoke_v37_h_claim SET state='RELEASED' WHERE session_id IN ('mainA','sideSession')").unwrap();
    db.execute("UPDATE gogoke_v37_seats SET generation=generation+1 WHERE seat_id IN ('leadA','sideSeat')").unwrap();
    assert_eq!(decode_receipt(&execute(&mut db,&owner,&create,None).unwrap()).unwrap().status,V37Status::Replayed,"Owner historical replay does not depend on an old H cache");
    let historical=side(&db,"projectA","sideA").unwrap();
    assert_eq!(status(&mut db,&owner,"read-thread","releasedRead",historical.revision),V37Status::Applied);
    assert_eq!(collect(&mut db,&owner,"projectA","sideA",32).unwrap().cursor,4);
    assert_eq!(read_thread(&mut db,&owner,"projectA","sideA",&LedgerPosition {epoch:historical.epoch.clone(),cursor:0},32).unwrap().events[0].input.event_id,"ownTurn");
    assert_eq!(settle_sync(&mut db,&owner,"projectA","questionA").unwrap().session_id,"sideSession");
    assert!(!begin_sync(&mut db,&owner,"projectA","sideA",&send,SyncMode::Question,4,|_,_,_|Ok(false)).unwrap().may_submit);
    assert!(matches!(derive_current(&mut db,&owner,"projectA","sideA"),Err(SideError::Denied)),"a historical Owner read does not grant a new live binding");
    bind_current_fixture(&mut db,"mainB","leadA",ledger::SessionPurpose::Work);
    bind_current_fixture(&mut db,"sideSessionB","sideSeat",ledger::SessionPurpose::SideChat);
    let current=derive_current(&mut db,&owner,"projectA","sideA").unwrap();assert_eq!(current.source_session_id,"mainB");assert_eq!(current.session_id,"sideSessionB");
    assert!(matches!(read_current_cache_continuity(&db,&s,&current),Ok(CacheContinuity::Unknown)),
        "new H binding without original process and A responses proves no cache identity");
    assert!(matches!(rebind_current(&mut db,&owner,"projectA","sideA",|_,_,_|Ok(CacheContinuity::Unknown)),Err(SideError::Unknown)));
    assert_eq!(side(&db,"projectA","sideA").unwrap().session_id,"sideSession","a generation change alone proves no cache identity");
    db.execute("UPDATE gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id='mainB'").unwrap();
    assert!(matches!(rebind_current(&mut db,&owner,"projectA","sideA",|_,old,new|Ok(CacheContinuity::Replaced(cache_fact(old,new)))),Err(SideError::Denied)),"new source permission is current");
    db.execute("UPDATE gogoke_v37_h_owner_binding SET state='ACTIVE' WHERE binding_id='mainB'").unwrap();
    let rebound=rebind_current(&mut db,&owner,"projectA","sideA",|_,old,new|Ok(CacheContinuity::Replaced(cache_fact(old,new)))).unwrap();
    assert_eq!(rebound.session_id,"sideSessionB");assert_eq!(rebound.synced_cursor,0);assert_eq!(rebound.cursor,4);
    assert!(!begin_sync(&mut db,&owner,"projectA","sideA",&send,SyncMode::Question,4,|_,_,_|Ok(false)).unwrap().may_submit,"old retry remains read-only after rebind");
    let original=settle_sync(&mut db,&owner,"projectA","questionA").unwrap();assert_eq!(original.session_id,"sideSession");assert_eq!(original.process_operation_id,"processsideSession");assert_eq!(original.state,"UNKNOWN");
    old_ack_fixture(&mut db,&rebound,&send);
    assert!(matches!(read_current_cache_continuity(&db,&s,&current),Err(SideError::Corrupt(_))),
        "an H episode with invalid original open bytes cannot prove cache continuity");
    db.execute("UPDATE gogoke_v37_instances SET driver_id='opencode' WHERE instance_id='instanceA'").unwrap();
    assert!(settle_sync(&mut db,&owner,"projectA","questionA").is_err(),
        "ACP delivery requires the original typed User/A response, not a generic H receipt");
    db.execute("UPDATE gogoke_v37_instances SET driver_id='codex' WHERE instance_id='instanceA'").unwrap();
    assert_eq!(settle_sync(&mut db,&owner,"projectA","questionA").unwrap().state,"DELIVERED","late ACK resolves only its old H intent");
    assert_eq!(side(&db,"projectA","sideA").unwrap().synced_cursor,0,"old ACK cannot advance the new reader");
    assert_eq!(side(&db,"projectA","sideA").unwrap().session_id,"sideSessionB");
    db.execute("UPDATE gogoke_v37_seats SET incarnation='syntheticDifferentIncarnation' WHERE seat_id='sideSeat'").unwrap();
    assert!(matches!(derive_current(&mut db,&owner,"projectA","sideA"),Err(SideError::Denied)),"seat-name reuse cannot inherit live authority");
    assert_eq!(status(&mut db,&owner,"read-thread","reincarnatedHistory",rebound.revision),V37Status::Applied);
    db.close_checked().unwrap();drop(root);
    if let Err(error)=std::fs::remove_dir_all(&path) {eprintln!("owned side store fixture retained: {error}");}
}
