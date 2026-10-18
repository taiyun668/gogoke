//! Synthetic original H/A/ACK boundary fixtures. These are not CLI/model
//! capability evidence; actual fixed 2.1.196 ordering is checked separately.
use super::*;
use crate::root::RootLock;
use crate::store::same_open::{create_new,route_b_test_guard};
use std::time::{SystemTime,UNIX_EPOCH};

const OLD_SEND: &[u8]=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"sendA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"generation":"1","body":"fixture"}}"#;
const RESUME: &[u8]=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"resume","requestId":"resumeB","targetId":"sessionA","domainId":"projectA","expectedRevision":"3","payload":{"generation":"1"}}"#;
const NEW_SEND: &[u8]=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"sendB","targetId":"sessionA","domainId":"projectA","expectedRevision":"4","payload":{"generation":"2","body":"fixture"}}"#;

fn source(db:&VerifiedDatabaseConnection<'_>,generation:&str,cursor:&str,
    raw:&[u8],reason:Option<&str>) {
    let suffix=if generation=="1" {"A"} else {"B"};
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state,no_event_reason) VALUES(?1,?2,?3,'projectA','sessionA',?4,?3,?5,?6,?7,?8)").unwrap();
    q.bind_text(1,&format!("process{suffix}")).unwrap();
    q.bind_text(2,&format!("pct1_ticket{suffix}")).unwrap();
    q.bind_text(3,&format!("nonce{suffix}")).unwrap();q.bind_text(4,generation).unwrap();
    q.bind_text(5,cursor).unwrap();q.bind_blob(6,raw).unwrap();
    q.bind_text(7,if reason.is_some() {"NO_EVENT"} else {"PENDING"}).unwrap();
    if let Some(reason)=reason {q.bind_text(8,reason).unwrap();}
    q.step_done().unwrap();
}

fn rpc(db:&VerifiedDatabaseConnection<'_>,generation:&str,open:&str,step:&str,
    command:&[u8],cursor:&str) {
    let suffix=if generation=="1" {"A"} else {"B"};
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase,source_epoch,source_cursor) VALUES('projectA','sessionA',?1,?2,?3,?4,?5,?9,?10,'fixture','sha256:fixture','profileA',?6,?7,1,'OBSERVED',?5,?8)").unwrap();
    for (index,value) in [open,step,&format!("process{suffix}"),&format!("pct1_ticket{suffix}"),
        &format!("nonce{suffix}"),generation,&hex(command),cursor].iter().enumerate() {
        q.bind_text((index+1) as i32,value).unwrap();
    }
    q.bind_text(9,if generation=="1" {"11"} else {"12"}).unwrap();
    q.bind_text(10,if generation=="1" {"1"} else {"2"}).unwrap();
    q.step_done().unwrap();
}

fn completed(db:&VerifiedDatabaseConnection<'_>,bytes:&[u8],generation:&str,
    open:&str,vendor:&str)->Vec<u8> {
    let suffix=if generation=="1" {"A"} else {"B"};
    let input=StdinRequest {domain_id:"projectA",session_id:"sessionA",
        ticket:&format!("pct1_ticket{suffix}"),generation,request_bytes:bytes};
    let (request,text,identity)=claude_send_request(&input).unwrap();
    source(db,generation,"2",format!("{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{vendor}\"}}\n").as_bytes(),None);
    let echo=format!("{{\"type\":\"user\",\"session_id\":\"{vendor}\",\"uuid\":\"{}\",\"parent_tool_use_id\":null,\"message\":{{\"role\":\"user\",\"content\":\"fixture\"}}}}\n",identity.uuid).into_bytes();
    let result=format!("{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"session_id\":\"{vendor}\"}}\n").into_bytes();
    source(db,generation,"4",&echo,Some("CLAUDE_STDIN_ACK"));
    source(db,generation,"6",&result,Some(CLAUDE_RESULT_NO_EVENT));
    rpc(db,generation,open,&identity.step_id,&commands::encode_claude(
        commands::ClaudeCommand::User {uuid:&identity.uuid,text:&text}).unwrap(),"4");
    let receipt_identity=format!("{}\n{}\n{}\nprocess{suffix}\nnonce{suffix}",
        crate::store::digest::sha256_hex(bytes),crate::store::digest::sha256_hex(&echo),
        crate::store::digest::sha256_hex(&result));
    let string=|value:&str|Json::String(JsonString::from_str(value));
    let fields=std::collections::BTreeMap::from([
        (JsonString::from_str("generation"),string(generation)),
        (JsonString::from_str("receiptId"),string(&format!("claude-{}",&crate::store::digest::sha256_hex(receipt_identity.as_bytes())[..40]))),
        (JsonString::from_str("deliveryBasis"),string("CLAUDE_USER_REPLAY_AND_RESULT")),
        (JsonString::from_str("vendorSessionId"),string(vendor)),
        (JsonString::from_str("userUuid"),string(&identity.uuid)),
        (JsonString::from_str("resultSubtype"),string("success")),
        (JsonString::from_str("sourceEpoch"),string(&format!("nonce{suffix}"))),
        (JsonString::from_str("sourceCursor"),string("6")),
        (JsonString::from_str("rawResultSha256"),string(&crate::store::digest::sha256_hex(&result))),
        (JsonString::from_str("createdTurn"),Json::Bool(true)),
    ]);
    let mut receipt=encode_receipt(&request,V37Status::Applied,request.expected_revision,
        request.expected_revision+1,fields);
    receipt.push(b'\n');
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_stdin_journal(domain_id,request_id,operation,ticket,process_operation_id,custodian_nonce,session_id,generation,request_hex,phase,receipt_hex,receipt_status,expected_revision,receipt_previous_revision,receipt_revision) VALUES('projectA',?1,'send',?2,?3,?4,'sessionA',?5,?6,'RECEIPTED',?7,'APPLIED',?8,?8,?9)").unwrap();
    for (index,value) in [request.request_id.as_str(),&format!("pct1_ticket{suffix}"),
        &format!("process{suffix}"),&format!("nonce{suffix}"),generation,&hex(bytes),
        &hex(&receipt),&request.expected_revision.to_string(),&(request.expected_revision+1).to_string()]
        .iter().enumerate() {q.bind_text((index+1) as i32,value).unwrap();}
    q.step_done().unwrap();receipt
}

fn fixture(db:&mut VerifiedDatabaseConnection<'_>) {
    db.execute("CREATE TABLE gogoke_v37_instances(instance_id TEXT PRIMARY KEY,driver_id TEXT,version TEXT) STRICT").unwrap();
    crate::store::authority::initialize_process_custody_schema(db).unwrap();
    super::super::admission::initialize_admission_schema(db).unwrap();
    db.execute("INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingA','instanceA','projectA','SESSION','sessionA','1','ACTIVE')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA','sessionA','instanceA','homeA','bindingA','1','COMMITTED',1,'processA')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('projectA','openA','sessionA','1',NULL,'6f70656e',0,1,'processA','instanceA','homeA','bindingA','seatA','seatIncarnationA','ACTIVE')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_generation VALUES('projectA','sessionA','1','openA','processA')").unwrap();
    db.execute("INSERT INTO gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','projectA','1','ACTIVE')").unwrap();
    db.execute("ALTER TABLE gogoke_v37_instances ADD COLUMN program_digest TEXT").unwrap();
    db.execute("ALTER TABLE gogoke_v37_instances ADD COLUMN login_state TEXT").unwrap();
    db.execute("INSERT INTO gogoke_v37_instances VALUES('instanceA','claude','2.1.196','sha256:fixture','LOGGED_IN')").unwrap();
    db.execute("CREATE TABLE gogoke_v37_instance_homes(home_id TEXT,instance_id TEXT,domain_id TEXT,owner_id TEXT,generation TEXT,kind TEXT,state TEXT) STRICT").unwrap();
    db.execute("INSERT INTO gogoke_v37_instance_homes VALUES('homeB','instanceA','projectA','sessionA','2','SESSION','ACTIVE')").unwrap();
    crate::store::seat::initialize_schema(db).unwrap();
    super::super::session_binding::initialize_schema(db).unwrap();
    super::super::rpc_journal::initialize_schema(db).unwrap();
    db.execute("CREATE TABLE orchestration_events(event_id TEXT PRIMARY KEY,sequence INTEGER UNIQUE) STRICT").unwrap();
    ledger::initialize_schema(db).unwrap();
    db.execute("INSERT INTO gogoke_v37_h_operation VALUES('projectA','openA','6f70656e','open','sessionA','APPLIED',0,1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_operation VALUES('projectA','stopA','73746f70','stop','sessionA','APPLIED',2,3)").unwrap();
    db.execute("UPDATE gogoke_v37_h_claim SET state='STOPPED',revision=3,stop_fact_id='proofA'").unwrap();
    db.execute("UPDATE gogoke_v37_h_process_episode SET phase='STOPPED',stop_request_id='stopA',stop_fact_id='proofA'").unwrap();
    db.execute("UPDATE gogoke_coordination_process_custody SET state='STOPPED',stop_proof_hash='proofA'").unwrap();
    completed(db,OLD_SEND,"1","openA","vendorA");
    db.execute("INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingB','instanceA','projectA','SESSION','sessionA','2','ACTIVE')").unwrap();
    db.execute("INSERT INTO gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('processB','pct1_ticketB','nonceB','12','2','fixture','sha256:fixture','profileA','projectA','2','ACTIVE')").unwrap();
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('projectA','resumeB','sessionA','2','1',?1,3,NULL,'processB','instanceA','homeB','bindingB','seatA','seatIncarnationA','PREPARED')").unwrap();
    q.bind_text(1,&hex(RESUME)).unwrap();q.step_done().unwrap();drop(q);
    db.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('projectA','seatA','seatIncarnationA','USER','LONG','instanceA','BUSY',1,1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_seat_binding VALUES('projectA','sessionA','seatA','seatIncarnationA','1')").unwrap();
    let (step,id)=super::super::rpc_journal::claude_initialize_identity(RESUME);
    source(db,"2","1",format!("{{\"type\":\"control_response\",\"response\":{{\"subtype\":\"success\",\"request_id\":\"{id}\",\"response\":{{}}}}}}\n").as_bytes(),Some("CLAUDE_STDIN_ACK"));
    rpc(db,"2","resumeB",&step,&commands::encode_claude(commands::ClaudeCommand::Initialize {request_id:&id}).unwrap(),"1");
}

fn with_fixture(test:impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
    let _guard=route_b_test_guard();
    let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let folder=std::env::temp_dir().join(format!("gogoke-claude-resume-{stamp}"));
    std::fs::create_dir(&folder).unwrap();let root=RootLock::acquire(&folder).unwrap();
    let mut db=create_new(&root,&folder.join("state.sqlite")).unwrap();fixture(&mut db);
    test(&mut db);db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(folder).unwrap();
}

fn ready(db:&VerifiedDatabaseConnection<'_>)->Result<Option<ClaudeResumeReady>,JournalError> {
    read_original_claude_resume_ready(db,"projectA","sessionA","resumeB","2","processB")
}
fn init(db:&VerifiedDatabaseConnection<'_>)->Result<Option<RawSourceKey>,JournalError> {
    read_original_claude_resume_init(db,"projectA","sessionA","resumeB","2","processB")
}

fn original_observation_snapshot(db:&VerifiedDatabaseConnection<'_>)->Vec<Vec<String>> {
    let mut rows=Vec::new();
    for sql in [
        "SELECT step_id,command_hex,phase,COALESCE(source_epoch,''),COALESCE(source_cursor,''),COALESCE(original_error,'') FROM gogoke_v37_rpc_steps ORDER BY step_id",
        "SELECT operation_id,source_epoch,source_cursor,hex(raw_bytes),state,COALESCE(no_event_reason,'') FROM v37_ledger_raw_source ORDER BY operation_id,CAST(source_cursor AS INTEGER)",
    ] {
        let q=Statement::prepare(db.as_ptr(),sql).unwrap();
        while q.step_row().unwrap() {rows.push((0..6).map(|i|q.column_text(i).unwrap()).collect());}
    }
    rows
}

#[test]
fn prepared_resume_ack_is_ready_without_inventing_init() {
    with_fixture(|db| {
        let ack=ready(db).unwrap().unwrap();
        assert_eq!(ack.expected_vendor_session_id,"vendorA");
        assert_eq!(ack.ack_source.source_cursor,"1");assert!(init(db).unwrap().is_none());
        db.execute("UPDATE gogoke_v37_rpc_steps SET pid='999' WHERE open_request_id='resumeB'").unwrap();
        assert!(ready(db).unwrap().is_none(),"ACK metadata must match exact physical custody");
        assert!(super::super::episodes::promote_resume(db,"projectA","sessionA","resumeB","processB",3).is_err());
        db.execute("UPDATE gogoke_v37_rpc_steps SET pid='12' WHERE open_request_id='resumeB'").unwrap();
        // Known counterevidence must block admission, even with a successful ACK.
        source(db,"2","2",b"{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"vendorB\"}\n",None);
        assert!(matches!(ready(db),Err(JournalError::Conflict)));
        assert!(super::super::episodes::promote_resume(db,"projectA","sessionA","resumeB","processB",3).is_err());
        db.execute("DELETE FROM v37_ledger_raw_source WHERE operation_id='processB' AND source_cursor='2'").unwrap();
        assert_eq!(super::super::episodes::promote_resume(db,"projectA","sessionA","resumeB","processB",3).unwrap(),4);
        assert!(init(db).unwrap().is_none());assert_eq!(ready(db).unwrap().unwrap().ack_source,ack.ack_source);
        for cursor in ["2","7"] {source(db,"2",cursor,b"{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"vendorA\"}\n",None);}
        assert_eq!(init(db).unwrap().unwrap().source_cursor,"2");
        assert_eq!(ready(db).unwrap().unwrap().ack_source,ack.ack_source);
        source(db,"2","8",b"{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"vendorB\"}\n",None);
        assert!(matches!(ready(db),Err(JournalError::Conflict)),
            "a correct first init cannot hide a later contradictory init");
        assert!(matches!(init(db),Err(JournalError::Conflict)));
    });
}

#[test]
fn unknown_resume_requires_init_and_observed_ack() {
    with_fixture(|db| {
        db.execute("UPDATE gogoke_v37_h_process_episode SET phase='UNKNOWN',result_revision=4 WHERE generation='2'").unwrap();
        db.execute("UPDATE gogoke_v37_h_claim SET revision=4").unwrap();
        assert!(ready(db).unwrap().is_none());
        assert!(super::super::episodes::promote_resume(db,"projectA","sessionA","resumeB","processB",4).is_err());
        source(db,"2","2",b"{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"vendorA\"}\n",None);
        db.execute("UPDATE gogoke_v37_rpc_steps SET phase='UNKNOWN',source_epoch=NULL,source_cursor=NULL,original_error='original initialize read failure' WHERE open_request_id='resumeB'").unwrap();
        assert!(init(db).unwrap().is_some(),"historical captured init remains independent evidence");
        assert!(ready(db).unwrap().is_none(),"late ACK must not masquerade as OBSERVED transport source");
        assert!(super::super::episodes::promote_resume(db,"projectA","sessionA","resumeB","processB",4).is_err());
        db.execute("UPDATE gogoke_v37_rpc_steps SET phase='OBSERVED',source_epoch='nonceB',source_cursor='1',original_error=NULL WHERE open_request_id='resumeB'").unwrap();
        assert_eq!(super::super::episodes::promote_resume(db,"projectA","sessionA","resumeB","processB",4).unwrap(),5);
    });
}

#[test]
fn cold_completion_rejects_new_consistent_but_wrong_vendor_identity() {
    with_fixture(|db| {
        super::super::episodes::promote_resume(db,"projectA","sessionA","resumeB","processB",3).unwrap();
        let stored=completed(db,NEW_SEND,"2","resumeB","vendorB");
        let input=StdinRequest {domain_id:"projectA",session_id:"sessionA",ticket:"pct1_ticketB",generation:"2",request_bytes:NEW_SEND};
        assert!(matches!(read_original_claude_send_completed(db,&input),Err(JournalError::Conflict)),
            "self-consistent new init/echo/result is insufficient when old completed target differs");
        let raw=read_row(db,"projectA","sendB").unwrap().unwrap();
        assert_eq!(raw.receipt_bytes.unwrap(),stored,"rejected readback must not rewrite original receipt");
        assert_eq!(raw.state,JournalState::Receipted);
        assert!(read_original_claude_send_completed(db,&StdinRequest {domain_id:"projectA",session_id:"sessionA",ticket:"pct1_ticketA",generation:"1",request_bytes:OLD_SEND}).unwrap().is_some(),
            "the old stopped completion stays readable");
    });
}

#[test]
fn cold_completed_receipt_keeps_original_bytes_after_custody_unknown() {
    with_fixture(|db| {
        super::super::episodes::promote_resume(db,"projectA","sessionA","resumeB","processB",3).unwrap();
        let stored=completed(db,NEW_SEND,"2","resumeB","vendorA");
        let input=StdinRequest {domain_id:"projectA",session_id:"sessionA",ticket:"pct1_ticketB",generation:"2",request_bytes:NEW_SEND};
        let initial=read_original_claude_send_completed(db,&input).unwrap().unwrap();
        assert_eq!(initial.user.record.receipt_bytes.as_ref().unwrap(),&stored);
        let (request,_,_)=claude_send_request(&input).unwrap();
        assert!(matches!(mark_claude_observation_record_unknown_in_transaction(db,&input,&request),
            Err(JournalError::Denied)),"observation marker requires custody already UNKNOWN");
        db.execute("UPDATE gogoke_coordination_process_custody SET state='UNKNOWN' WHERE operation_id='processB'").unwrap();
        // Exercise the same production record marker used after the public
        // Owner/full-physical-custody verifier. This SQL fixture does not
        // construct a process ticket or claim dynamic validation of that wrapper.
        db.execute("BEGIN IMMEDIATE").unwrap();
        let observed=original_observation_snapshot(db);
        db.execute("SAVEPOINT prepared_observation_fixture").unwrap();
        db.execute("UPDATE gogoke_v37_h_stdin_journal SET phase='PREPARED',receipt_hex=NULL,receipt_status=NULL,receipt_previous_revision=NULL,receipt_revision=NULL WHERE request_id='sendB'").unwrap();
        let fenced=mark_claude_observation_record_unknown_in_transaction(db,&input,&request).unwrap();
        assert_eq!(fenced.record.state,JournalState::Unknown);
        assert_eq!(fenced.record.request_bytes,NEW_SEND);
        assert_eq!(original_observation_snapshot(db),observed,
            "original OBSERVED initialize/echo RPC and A must not be reclassified");
        db.execute("ROLLBACK TO prepared_observation_fixture").unwrap();
        db.execute("RELEASE prepared_observation_fixture").unwrap();
        let completed=mark_claude_observation_record_unknown_in_transaction(db,&input,&request).unwrap();
        assert_eq!(completed.record,initial.user.record,"stored completed receipt remains byte exact");
        db.execute("SAVEPOINT missing_observation_fixture").unwrap();
        db.execute("DELETE FROM gogoke_v37_h_stdin_journal WHERE request_id='sendB'").unwrap();
        assert!(matches!(mark_claude_observation_record_unknown_in_transaction(db,&input,&request),
            Err(JournalError::Unknown)),"missing input is not a new intent");
        assert!(read_row(db,"projectA","sendB").unwrap().is_none());
        assert_eq!(original_observation_snapshot(db),observed);
        db.execute("ROLLBACK TO missing_observation_fixture").unwrap();
        db.execute("RELEASE missing_observation_fixture").unwrap();
        db.execute("COMMIT").unwrap();
        db.execute("UPDATE gogoke_v37_h_claim SET state='UNKNOWN' WHERE generation='2'").unwrap();
        let historical=read_original_claude_send_completed(db,&input).unwrap().unwrap();
        assert_eq!(historical.user.record,initial.user.record,
            "a later custody uncertainty must not rewrite or discard completed history");
        assert!(ready(db).is_err(),"historical read states cannot grant transport readiness");
    });
}
