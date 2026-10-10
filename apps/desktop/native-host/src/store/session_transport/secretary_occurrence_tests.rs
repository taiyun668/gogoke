//! Persisted-shape transaction controls. The H/A rows are synthetic fixture
//! facts; these tests do not claim a native child, model, or USER pipe ran.
use super::*;
use crate::store::{ledger,seat};
use crate::store::seat::{SecretaryRoutineChange,SecretaryRoutineCommand};
use crate::store::digest::sha256_hex;
use crate::root::RootLock;
use crate::store::authority;
use crate::store::same_open::route_b_test_guard;
use std::time::{SystemTime,UNIX_EPOCH};

const ORIGINAL:&str="每1小时提醒我检查队列";
const DUE:i64=4_600_000;
const NOW:i64=DUE+1;

fn count(db:&VerifiedDatabaseConnection<'_>,table:&str)->i64 {
    let q=Statement::prepare(db.as_ptr(),&format!("SELECT CAST(count(*) AS TEXT) FROM main.{table}"))
        .unwrap();
    assert!(q.step_row().unwrap());
    q.column_text(0).unwrap().parse().unwrap()
}

fn setup(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
    original:&str,rule:&str,timezone:&str,due:i64,presence_ms:i64)->OwnerIssuer {
    authority::initialize_process_custody_schema(db).unwrap();
    let owner=authority::initialize_profile(db,root).unwrap();
    super::super::session_binding::project_legacy(db).unwrap();
    super::super::rpc_journal::initialize_schema(db).unwrap();
    db.execute("INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceS','codex','fixture-home','fixture-home-identity','sha256:fixture','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_instance_profiles(instance_id,display_name,enabled,tombstoned,revision) VALUES('instanceS','fixture',1,0,1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES('homeS','instanceS','global','SESSION','sessionS','1','ACTIVE',1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('global','seatS','incarnationS','USER','LONG','instanceS','BUSY',1,1)").unwrap();
    let settings=Parser::parse(r#"{"model":"m","effort":"high","permissionTier":"READ_ONLY","orchestrationScope":{"instanceIds":["instanceS"],"models":["m"],"reasoningEfforts":["high"],"maxPermissionTier":"READ_ONLY","maxConcurrent":4}}"#).unwrap().canonical();
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_seat_settings(domain_id,seat_id,template_id,settings_json) VALUES('global','seatS','templateS',?1)").unwrap();
    q.bind_text(1,&settings).unwrap();q.step_done().unwrap();drop(q);
    db.execute("INSERT INTO gogoke_v37_seat_secretary(singleton,domain_id,seat_id,incarnation,request_id,fingerprint) VALUES(1,'global','seatS','incarnationS','designationS','fixture-fingerprint')").unwrap();
    db.execute("INSERT INTO gogoke_v37_seat_secretary_absence_policy VALUES(1,1,86400000,'PRODUCT_DEFAULT_V1:ABSENCE_24_HOURS')").unwrap();
    ledger::register_session(db,&ledger::SessionRegistration {domain_id:"global".into(),
        seat_id:"seatS".into(),session_id:"sessionS".into(),
        purpose:ledger::SessionPurpose::Secretary,side_id:None}).unwrap();
    db.execute("INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingS','instanceS','global','SESSION','sessionS','1','ACTIVE')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('global','sessionS','instanceS','homeS','bindingS','1','COMMITTED',2,'processS')").unwrap();
    db.execute("INSERT INTO gogoke_v37_native_selection VALUES('global','sessionS','seatS','incarnationS',1,'instanceS')").unwrap();
    db.execute("INSERT INTO gogoke_v37_session_binding_v2 VALUES('global','sessionS','seatS','incarnationS',1,'instanceS','NATIVE_V2')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('global','openS','sessionS','1','6f70656e',0,1,'processS','instanceS','homeS','bindingS','seatS','incarnationS','ACTIVE')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_generation VALUES('global','sessionS','1','openS','processS')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_operation VALUES('global','openS','6f70656e','open','sessionS','APPLIED',0,1)").unwrap();
    db.execute("INSERT INTO gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('processS','pct1_ticketS','nonceS','11','1','fixture','sha256:fixture','profileS','global','1','ACTIVE')").unwrap();
    let raw=Json::Object(std::collections::BTreeMap::from([
        (JsonString::from_str("schema"),Json::String(JsonString::from_str("gogoke.37.operations.v1"))),
        (JsonString::from_str("family"),Json::String(JsonString::from_str("K-SESSION"))),
        (JsonString::from_str("operation"),Json::String(JsonString::from_str("send"))),
        (JsonString::from_str("requestId"),Json::String(JsonString::from_str("sendS"))),
        (JsonString::from_str("targetId"),Json::String(JsonString::from_str("sessionS"))),
        (JsonString::from_str("domainId"),Json::String(JsonString::from_str("global"))),
        (JsonString::from_str("expectedRevision"),Json::String(JsonString::from_str("1"))),
        (JsonString::from_str("payload"),Json::Object(std::collections::BTreeMap::from([
            (JsonString::from_str("body"),Json::String(JsonString::from_str(original))),
            (JsonString::from_str("generation"),Json::String(JsonString::from_str("1"))),
        ]))),
    ])).canonical().into_bytes();
    let request=decode_request(&raw).unwrap();
    let command=codex_rpc::Command::TurnStart {thread_id:"threadS".into(),
        cwd:"fixture-directory".into(),model:"m".into(),effort:"high".into(),
        text:original.into(),network_access:Some(false)};
    let command_bytes=command.encode(Some(&codex_rpc::RpcId::Number(4))).unwrap();
    let response=b"{\"id\":4,\"result\":{\"turn\":{\"id\":\"turnS\",\"status\":\"inProgress\"}}}\n";
    let receipt_identity=format!("{}\n{}\n{}\n{}",sha256_hex(&raw),sha256_hex(&command_bytes),"processS","nonceS");
    let result=std::collections::BTreeMap::from([
        (JsonString::from_str("receiptId"),Json::String(JsonString::from_str(&format!("rpc-{}",&sha256_hex(receipt_identity.as_bytes())[..40])))),
        (JsonString::from_str("turnId"),Json::String(JsonString::from_str("turnS"))),
        (JsonString::from_str("createdTurn"),Json::Bool(true)),
    ]);
    let receipt=encode_receipt(&request,V37Status::Applied,1,2,result);
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_stdin_journal(domain_id,request_id,operation,ticket,process_operation_id,custodian_nonce,session_id,generation,request_hex,phase,receipt_hex,receipt_status,expected_revision,receipt_previous_revision,receipt_revision) VALUES('global','sendS','send','pct1_ticketS','processS','nonceS','sessionS','1',?1,'RECEIPTED',?2,'APPLIED',1,1,2)").unwrap();
    q.bind_text(1,&hex(&raw)).unwrap();q.bind_text(2,&hex(&receipt)).unwrap();q.step_done().unwrap();drop(q);
    let thread_command=codex_rpc::Command::ThreadStart {cwd:"fixture-directory".into(),model:"m".into()};
    let thread_raw=thread_command.encode(Some(&codex_rpc::RpcId::Number(3))).unwrap();
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state,no_event_reason) VALUES('processS','pct1_ticketS','nonceS','global','sessionS','1','epochS','1',?1,'NO_EVENT','CODEX_RPC_RESPONSE')").unwrap();
    q.bind_blob(1,b"{\"id\":3,\"result\":{\"thread\":{\"id\":\"threadS\",\"cwd\":\"fixture-directory\"}}}\n").unwrap();q.step_done().unwrap();drop(q);
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase,source_epoch,source_cursor) VALUES('global','sessionS','openS','thread-start','processS','pct1_ticketS','nonceS','11','1','fixture','sha256:fixture','profileS','1',?1,1,'OBSERVED','epochS','1')").unwrap();
    q.bind_text(1,&hex(&thread_raw)).unwrap();q.step_done().unwrap();drop(q);
    let step_id=format!("send-{}",&sha256_hex(&raw)[..40]);
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state,no_event_reason) VALUES('processS','pct1_ticketS','nonceS','global','sessionS','1','epochS','2',?1,'NO_EVENT','CODEX_RPC_RESPONSE')").unwrap();
    q.bind_blob(1,response).unwrap();q.step_done().unwrap();drop(q);
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase,source_epoch,source_cursor) VALUES('global','sessionS','openS',?1,'processS','pct1_ticketS','nonceS','11','1','fixture','sha256:fixture','profileS','1',?2,1,'OBSERVED','epochS','2')").unwrap();
    q.bind_text(1,&step_id).unwrap();q.bind_text(2,&hex(&command_bytes)).unwrap();q.step_done().unwrap();drop(q);
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_seat_secretary_presence VALUES('H-USER:global:sessionS:sendS','INPUT','processS','nonceS','sendS',?1,?2)").unwrap();
    q.bind_i64(1,presence_ms).unwrap();q.bind_i64(2,presence_ms).unwrap();q.step_done().unwrap();drop(q);
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_seat_secretary_routines VALUES('routineS','seatS','incarnationS',?1,'processS','epochS','2',?2,?3,?4,'ACTIVE',1,'','NONE','')").unwrap();
    q.bind_text(1,original).unwrap();q.bind_text(2,rule).unwrap();q.bind_text(3,timezone).unwrap();
    q.bind_i64(4,due).unwrap();q.step_done().unwrap();drop(q);
    owner
}

fn with_fixture_schedule(original:&str,rule:&str,timezone:&str,due:i64,presence_ms:i64,
    action:impl FnOnce(&mut VerifiedDatabaseConnection<'_>,&OwnerIssuer)) {
    let _guard=route_b_test_guard();
    let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let folder=std::env::temp_dir().join(format!("gogoke-secretary-occurrence-{}-{stamp}",std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let root=RootLock::acquire(&folder).unwrap();
    let mut db=crate::store::session::open_product_database(&root,&folder.join("state.sqlite")).unwrap();
    let owner=setup(&mut db,&root,original,rule,timezone,due,presence_ms);
    action(&mut db,&owner);
    db.close_checked().unwrap();drop(root);
    std::fs::remove_dir_all(folder).unwrap();
}

fn with_fixture(action:impl FnOnce(&mut VerifiedDatabaseConnection<'_>,&OwnerIssuer)) {
    with_fixture_schedule(ORIGINAL,"每1小时","UTC",DUE,1_000_000,action)
}

fn prepare(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,revision:i64)
    ->Result<ScheduledSecretaryDecision,JournalError> {
    prepare_at(db,owner,revision,NOW)
}

fn prepare_at(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,revision:i64,now_ms:i64)
    ->Result<ScheduledSecretaryDecision,JournalError> {
    prepare_scheduled_secretary_occurrence(db,&ScheduledSecretaryInput {
        owner,routine_id:"routineS",expected_routine_revision:revision,now_ms,
        session_id:"sessionS",ticket:"pct1_ticketS",generation:"1",expected_h_revision:2,
        provider:ScheduledSecretaryProvider::Codex,
    })
}

#[test]
fn due_prepare_requires_exact_original_source_and_current_secretary_scope() {
    with_fixture(|db,owner| {
        db.execute("DROP TABLE gogoke_v37_seat_secretary_schedule_errors").unwrap();
        seat::initialize_schema(db).unwrap();
        assert_eq!(count(db,"gogoke_v37_seat_secretary_routines"),1,
            "exact prior schema migration retains the original E routine");
        db.execute("UPDATE v37_ledger_session SET purpose='WORK' WHERE session_id='sessionS'").unwrap();
        assert!(prepare(db,owner,1).is_err());
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),0);
        db.execute("UPDATE v37_ledger_session SET purpose='SECRETARY' WHERE session_id='sessionS'").unwrap();
        db.execute("UPDATE gogoke_v37_seat_secretary_routines SET source_cursor='missing' WHERE routine_id='routineS'").unwrap();
        assert!(prepare(db,owner,1).is_err());
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),0);
        assert_eq!(count(db,"gogoke_v37_h_stdin_journal"),1);
        db.execute("UPDATE gogoke_v37_seat_secretary_routines SET source_cursor='2' WHERE routine_id='routineS'").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        seat::require_secretary_session(db,owner,"seatS","incarnationS").unwrap();
        db.execute("ROLLBACK").unwrap();
        let ScheduledSecretaryDecision::NewWrite(permit)=prepare(db,owner,1).unwrap()
            else {panic!("exact original source must reserve one H write")};
        let ScheduledSecretaryWrite::Codex {request_bytes,occurrence_id,..}=permit.into_write()
            else {panic!("fixed Codex driver must use Codex encoding")};
        assert_eq!(decode_request(&request_bytes).unwrap().request_id,occurrence_id);
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),1);
        assert_eq!(count(db,"gogoke_v37_h_stdin_journal"),2);
        assert!(prepare(db,owner,1).is_err());
        assert!(prepare(db,owner,2).is_err());
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),1);
        let q=Statement::prepare(db.as_ptr(),"UPDATE gogoke_v37_h_stdin_journal SET phase='UNKNOWN' WHERE request_id=?1").unwrap();
        q.bind_text(1,&occurrence_id).unwrap();q.step_done().unwrap();
        assert!(prepare(db,owner,2).is_err(),"UNKNOWN cannot mint a second permit");
        assert_eq!(count(db,"gogoke_v37_h_stdin_journal"),2);
        let unknown=mark_scheduled_secretary_occurrence_unknown(db,owner,"routineS",
            &occurrence_id,2,"sessionS","pct1_ticketS","1",
            "original physical write uncertain",NOW).unwrap();
        assert_eq!(unknown.last_result,"UNKNOWN");
        assert_eq!(unknown.last_reason,"original physical write uncertain");
        assert_eq!(unknown.state,"WAITING_NEXT");
        assert!(mark_scheduled_secretary_occurrence_unknown(db,owner,"routineS",
            &occurrence_id,3,"sessionS","pct1_ticketS","1",
            "different later reason",NOW).is_err());
        assert!(settle_scheduled_secretary_occurrence(db,owner,"routineS",
            &occurrence_id,3,NOW,"sessionS","pct1_ticketS","1").unwrap().is_none());
    });
}

fn capture_due_codex_response(db:&mut VerifiedDatabaseConnection<'_>,request_bytes:&[u8],
    occurrence_id:&str) {
    let request=decode_request(request_bytes).unwrap();
    let text=payload_string(&request,"body").unwrap();
    let command=codex_rpc::Command::TurnStart {thread_id:"threadS".into(),
        cwd:"fixture-directory".into(),model:"m".into(),effort:"high".into(),
        text,network_access:Some(false)};
    let rpc_id=codex_rpc::RpcId::String(occurrence_id.to_owned());
    let command_bytes=command.encode(Some(&rpc_id)).unwrap();
    let response=format!(r#"{{"id":"{occurrence_id}","result":{{"turn":{{"id":"turnDue","status":"inProgress"}}}}}}"#);
    let step_id=format!("send-{}",&sha256_hex(request_bytes)[..40]);
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state,no_event_reason) VALUES('processS','pct1_ticketS','nonceS','global','sessionS','1','epochS','3',?1,'NO_EVENT','CODEX_RPC_RESPONSE')").unwrap();
    q.bind_blob(1,format!("{response}\n").as_bytes()).unwrap();q.step_done().unwrap();drop(q);
    let q=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase,source_epoch,source_cursor) VALUES('global','sessionS','openS',?1,'processS','pct1_ticketS','nonceS','11','1','fixture','sha256:fixture','profileS','1',?2,1,'OBSERVED','epochS','3')").unwrap();
    q.bind_text(1,&step_id).unwrap();q.bind_text(2,&hex(&command_bytes)).unwrap();q.step_done().unwrap();drop(q);
    let input=StdinRequest {domain_id:"global",session_id:"sessionS",ticket:"pct1_ticketS",
        generation:"1",request_bytes};
    let completed=recover_codex_turn_request(db,&input).unwrap().unwrap();
    assert_eq!(completed.record.state,JournalState::Receipted);
    assert_eq!(completed.record.receipt_status,Some(V37Status::Applied));
}

#[test]
fn only_original_h_receipt_settles_and_user_pause_or_delete_wins() {
    for command in [SecretaryRoutineCommand::Pause,SecretaryRoutineCommand::Delete] {
        with_fixture(|db,owner| {
            let ScheduledSecretaryDecision::NewWrite(permit)=prepare(db,owner,1).unwrap()
                else {panic!("exact due source must prepare one write")};
            let ScheduledSecretaryWrite::Codex {request_bytes,occurrence_id,..}=permit.into_write()
                else {panic!("fixed Codex driver")};
            assert!(settle_scheduled_secretary_occurrence(db,owner,"routineS",
                &occurrence_id,2,NOW,"sessionS","pct1_ticketS","1").unwrap().is_none(),
                "PREPARED cannot be described as delivered");
            let user_raw=if command==SecretaryRoutineCommand::Pause {
                b"original USER pause".as_slice()
            } else {b"original USER delete".as_slice()};
            let request_id=if command==SecretaryRoutineCommand::Pause {"pauseS"} else {"deleteS"};
            let (changed,_)=seat::change_secretary_routine(db,owner,SecretaryRoutineChange {
                routine_id:"routineS",expected_revision:2,request_id,
                request_bytes:user_raw,command,next_due_ms:None,now_ms:NOW,
            }).unwrap();
            assert_eq!(changed.revision,3);
            capture_due_codex_response(db,&request_bytes,&occurrence_id);
            assert!(settle_scheduled_secretary_occurrence(db,owner,"routineS",
                &occurrence_id,2,NOW,"sessionS","pct1_ticketS","1").is_err(),
                "stale pre-USER CAS cannot settle the occurrence");
            let settled=settle_scheduled_secretary_occurrence(db,owner,"routineS",
                &occurrence_id,3,NOW,"sessionS","pct1_ticketS","1").unwrap().unwrap();
            assert!(settled.next_schedule_error.is_none(),"paused/deleted never compute another due");
            assert_eq!(settled.routine.state,if command==SecretaryRoutineCommand::Pause {"PAUSED"} else {"DELETED"});
            assert_eq!(settled.routine.last_result,"DELIVERED");
            assert_eq!(settled.routine.next_due_ms,DUE,"USER pause/delete retains its selected due without activation");
            assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),1);
        });
    }
}

#[test]
fn terminal_receipt_settles_even_when_next_local_clock_does_not_exist() {
    let prior=chrono::DateTime::parse_from_rfc3339("2026-03-07T10:30:00Z").unwrap().timestamp_millis();
    let now=chrono::DateTime::parse_from_rfc3339("2026-03-07T11:00:00Z").unwrap().timestamp_millis();
    with_fixture_schedule("每天 02:30 提醒我检查队列","每天 02:30",
        "America/Los_Angeles",prior,now-1_000,|db,owner| {
        let ScheduledSecretaryDecision::NewWrite(permit)=prepare_at(db,owner,1,now).unwrap()
            else {panic!("due preparation")};
        let ScheduledSecretaryWrite::Codex {request_bytes,occurrence_id,..}=permit.into_write()
            else {panic!("fixed Codex driver")};
        capture_due_codex_response(db,&request_bytes,&occurrence_id);
        let settled=settle_scheduled_secretary_occurrence(db,owner,"routineS",
            &occurrence_id,2,now,"sessionS","pct1_ticketS","1").unwrap().unwrap();
        assert_eq!(settled.routine.last_result,"DELIVERED");
        assert_eq!(settled.routine.state,"WAITING_NEXT");
        assert_eq!(settled.routine.next_due_ms,0);
        assert_eq!(settled.next_schedule_error,
            Some(seat::secretary_schedule::ScheduleError::NonexistentLocalTime));
        assert_eq!(count(db,"gogoke_v37_seat_secretary_occurrences"),1);
        db.execute("BEGIN IMMEDIATE").unwrap();
        let errors=seat::read_secretary_schedule_errors_in_transaction(db,owner,"routineS").unwrap();
        db.execute("ROLLBACK").unwrap();
        assert_eq!(errors.len(),1);
        assert_eq!(errors[0].occurrence_id,occurrence_id);
        assert_eq!(errors[0].diagnostic,"NonexistentLocalTime");
        let q=Statement::prepare(db.as_ptr(),"SELECT state FROM gogoke_v37_seat_secretary_occurrences WHERE occurrence_id=?1").unwrap();
        q.bind_text(1,&occurrence_id).unwrap();assert!(q.step_row().unwrap());
        assert_eq!(q.column_text(0).unwrap(),"DELIVERED");
    });
    with_fixture_schedule("每天 02:30 提醒我检查队列","每天 02:30",
        "America/Los_Angeles",prior,now-1_000,|db,owner| {
        let ScheduledSecretaryDecision::NewWrite(permit)=prepare_at(db,owner,1,now).unwrap()
            else {panic!("due preparation")};
        let ScheduledSecretaryWrite::Codex {request_bytes,occurrence_id,..}=permit.into_write()
            else {panic!("fixed Codex driver")};
        seat::change_secretary_routine(db,owner,SecretaryRoutineChange {
            routine_id:"routineS",expected_revision:2,request_id:"pauseDailyS",
            request_bytes:b"original USER pause daily",command:SecretaryRoutineCommand::Pause,
            next_due_ms:None,now_ms:now,
        }).unwrap();
        capture_due_codex_response(db,&request_bytes,&occurrence_id);
        let settled=settle_scheduled_secretary_occurrence(db,owner,"routineS",
            &occurrence_id,3,now,"sessionS","pct1_ticketS","1").unwrap().unwrap();
        assert_eq!(settled.routine.state,"PAUSED");
        assert!(settled.next_schedule_error.is_none(),"USER pause precludes next-time calculation");
        assert_eq!(count(db,"gogoke_v37_seat_secretary_schedule_errors"),0);
    });
}

#[test]
fn failure_reason_keeps_original_provider_excerpt_and_truncation_fact() {
    let raw="provider original error: quota exhausted\ntrace line";
    let result=std::collections::BTreeMap::from([
        (JsonString::from_str("providerFailureRawExcerpt"),Json::String(JsonString::from_str(raw))),
        (JsonString::from_str("providerFailureTruncated"),Json::Bool(true)),
        (JsonString::from_str("stopReason"),Json::String(JsonString::from_str("error"))),
    ]);
    let reason=scheduled_failure_reason(V37Status::Failed,&result).unwrap();
    assert_eq!(reason,format!("providerFailureTruncated=true\n{raw}"));
    assert!(reason.contains(raw));
    let mut missing=result;
    missing.remove(&JsonString::from_str("providerFailureTruncated"));
    assert!(matches!(scheduled_failure_reason(V37Status::Failed,&missing),Err(JournalError::Unknown)));
}

#[test]
fn uncertain_h_delivery_records_original_reason_without_reviving_user_tombstones() {
    for command in [SecretaryRoutineCommand::Pause,SecretaryRoutineCommand::Delete] {
        with_fixture(|db,owner| {
            let ScheduledSecretaryDecision::NewWrite(permit)=prepare(db,owner,1).unwrap()
                else {panic!("due preparation")};
            let ScheduledSecretaryWrite::Codex {occurrence_id,..}=permit.into_write()
                else {panic!("fixed Codex driver")};
            let request_id=if command==SecretaryRoutineCommand::Pause {"pauseUnknownS"} else {"deleteUnknownS"};
            seat::change_secretary_routine(db,owner,SecretaryRoutineChange {
                routine_id:"routineS",expected_revision:2,request_id,
                request_bytes:b"original USER routine control",command,
                next_due_ms:None,now_ms:NOW,
            }).unwrap();
            let q=Statement::prepare(db.as_ptr(),"UPDATE gogoke_v37_h_stdin_journal SET phase='UNKNOWN' WHERE request_id=?1").unwrap();
            q.bind_text(1,&occurrence_id).unwrap();q.step_done().unwrap();drop(q);
            let marked=mark_scheduled_secretary_occurrence_unknown(db,owner,"routineS",
                &occurrence_id,3,"sessionS","pct1_ticketS","1",
                "original provider remote error",NOW).unwrap();
            assert_eq!(marked.state,if command==SecretaryRoutineCommand::Pause {"PAUSED"} else {"DELETED"});
            assert_eq!(marked.last_result,"UNKNOWN");
            assert_eq!(marked.last_reason,"original provider remote error");
            assert_eq!(count(db,"gogoke_v37_seat_secretary_schedule_errors"),0);
        });
    }
}
