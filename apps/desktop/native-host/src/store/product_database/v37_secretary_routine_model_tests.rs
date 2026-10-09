//! E.3 model routine controls against the composed ProductDatabase entry.
//! H admission and protocol bytes are fixture facts. Pipe stdout, native Job
//! custody, A capture, the H journal and the model caller factory are real.
//! No CLI, login, model execution or LPAC admission is claimed here.

use super::*;
use crate::store::atomic::Parser;
use super::v37_secretary_routine_model::ResolvedRoutineSchedule;
use crate::process::{NativeBinding, OriginBoundFrame, PrepareRequest, PreparedCustody,
    ProcessCustodian, ProcessLaunch};
use crate::store::{inbox, ledger, seat};
use crate::store::digest::{content_hash, sha256_hex};
use crate::store::same_open::route_b_test_guard;
use crate::store::session_transport::{self as h, codex_rpc, model_call};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BODY: &str = "Please remind me in 10 minutes to review the queue.";
const SPAN: &str = "in 10 minutes";
const THREAD: &str = r#"{"id":3,"result":{"thread":{"id":"threadS","cwd":"fixture-directory"}}}"#;
const TURN: &str = r#"{"id":4,"result":{"turn":{"id":"turnS","status":"inProgress"}}}"#;
const STARTED: &str = r#"{"method":"turn/started","params":{"threadId":"threadS","turn":{"id":"turnS","status":"inProgress"}}}"#;
const CALL: &str = r#"{"id":91,"method":"item/tool/call","params":{"callId":"callS","threadId":"threadS","turnId":"turnS","tool":"gogoke_routine","arguments":{"operation":"create","scheduleSpan":"in 10 minutes","timezone":"HOST_DEFAULT"}}}"#;
const TAKEOVER_QUESTION: &str = r#"{"id":44,"method":"item/tool/requestUserInput","params":{"threadId":"threadS","turnId":"turnS","itemId":"itemS","questions":[{"id":"q","header":"Scope","question":"Which scope?","isOther":true,"isSecret":false,"options":null}]}}"#;
const TAKEOVER_CALL: &str = r#"{"id":92,"method":"item/tool/call","params":{"callId":"takeoverS","threadId":"threadS","turnId":"turnS","tool":"gogoke_takeover","arguments":{"operation":"takeover-answers"}}}"#;

fn hex(bytes: &[u8]) -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() }

fn process(folder: &Path, lines: &[&str]) -> (ProcessCustodian, PreparedCustody) {
    let command = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
        .join("System32").join("cmd.exe");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let file_name = format!("secretary-wire-{stamp}.jsonl");
    fs::write(folder.join(&file_name), format!("{}\n", lines.join("\n"))).unwrap();
    let mut launch = ProcessLaunch::new(command.clone());
    launch.arguments = vec!["/D".into(), "/C".into(), format!("type {file_name}")];
    launch.current_directory = Some(folder.to_owned());
    launch.protocol_stdio = true;
    launch.persistent_protocol_stdio = true;
    let mut custodian = ProcessCustodian::new().unwrap();
    let custody = custodian.prepare(&PrepareRequest {
        binding: NativeBinding {binary_digest_sha256: content_hash(&fs::read(&command).unwrap()),
            profile_id: "profile-secretary-routine-control".into(), domain_id: "global".into(),
            generation: "1".into()}, launch,
    }).unwrap();
    (custodian, custody)
}

fn capture(product: &mut ProductDatabase<'_>, frame: &OriginBoundFrame, cursor: usize)
    -> ledger::RawSourceKey {
    let source = ledger::capture_raw_source(&mut product.connection, frame, "processS", "epochS",
        &cursor.to_string()).expect("production A capture from native pipe");
    assert_eq!(source.raw_bytes, frame.bytes());
    source.key
}

fn observed(product: &ProductDatabase<'_>, custody: &PreparedCustody,
    key: &ledger::RawSourceKey, step: &str, command: &codex_rpc::Command,
    id: codex_rpc::RpcId) {
    let insert = Statement::prepare(product.connection.as_ptr(),
        "INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,
         step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,
         binary_digest,profile_id,generation,command_hex,requires_response,phase,
         source_epoch,source_cursor) VALUES('global','sessionS','openS',?1,'processS',
         ?2,?3,?4,?5,?6,?7,?8,'1',?9,1,'OBSERVED','epochS',?10)").unwrap();
    for (index, value) in [step.to_owned(), custody.ticket.opaque().to_owned(),
        custody.custodian_nonce.clone(), custody.identity.pid.to_string(),
        custody.identity.creation_time_100ns.to_string(),
        custody.identity.image_path.to_string_lossy().into_owned(),
        custody.binding.binary_digest_sha256.clone(), custody.binding.profile_id.clone(),
        hex(&command.encode(Some(&id)).unwrap()), key.source_cursor.clone()]
        .iter().enumerate() {
        insert.bind_text((index + 1) as i32, value).unwrap();
    }
    insert.step_done().unwrap(); drop(insert);
    let update = Statement::prepare(product.connection.as_ptr(),
        "UPDATE main.v37_ledger_raw_source SET state='NO_EVENT',
         no_event_reason='CODEX_RPC_RESPONSE' WHERE operation_id='processS'
         AND source_epoch='epochS' AND source_cursor=?1").unwrap();
    update.bind_text(1, &key.source_cursor).unwrap(); update.step_done().unwrap();
}

fn user_request() -> Vec<u8> {
    format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"sendS","targetId":"sessionS","domainId":"global","expectedRevision":"1","payload":{{"body":"{BODY}","generation":"1"}}}}"#).into_bytes()
}

fn fixture_with_wire(fourth: &str, extra: &[&str], captured: usize,
    purpose: ledger::SessionPurpose,
    action: impl FnOnce(&mut ProductDatabase<'_>, &PreparedCustody,
    &[OriginBoundFrame], &[ledger::RawSourceKey], &[u8])) {
    let _guard = route_b_test_guard();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let folder = std::env::temp_dir().join(format!("gogoke-secretary-routine-{}-{stamp}", std::process::id()));
    fs::create_dir(&folder).unwrap();
    let root = RootLock::acquire(&folder).unwrap();
    let mut product = ProductDatabase::open(&root, &folder.join("state.sqlite")).unwrap();
    let lines = [vec![THREAD, TURN, STARTED, fourth], extra.to_vec()].concat();
    let (mut custodian, custody) = process(&folder, &lines);
    authority::record_prepared_process(&mut product.connection, "processS", &custody).unwrap();
    // Synthetic H admission and E configuration facts are scoped to the
    // physically global CMD child; this never renames a projectA origin.
    let instance = Statement::prepare(product.connection.as_ptr(),
        "INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,
         program_digest,version,install_state,login_state,revision)
         VALUES('instanceS','codex','fixture-home','fixture-home-identity',?1,
         '0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
    instance.bind_text(1, &custody.binding.binary_digest_sha256).unwrap();
    instance.step_done().unwrap(); drop(instance);
    product.connection.execute("INSERT INTO gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES('homeS','instanceS','global','SESSION','sessionS','1','ACTIVE',1)").unwrap();
    product.connection.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('global','seatS','incarnationS','USER','LONG','instanceS','BUSY',1,1)").unwrap();
    if purpose==ledger::SessionPurpose::Secretary {
        product.connection.execute("INSERT INTO gogoke_v37_seat_secretary(singleton,domain_id,seat_id,incarnation,request_id,fingerprint) VALUES(1,'global','seatS','incarnationS','designationS','fixture-fingerprint')").unwrap();
    }
    ledger::register_session(&mut product.connection, &ledger::SessionRegistration {
        domain_id:"global".into(),seat_id:"seatS".into(),session_id:"sessionS".into(),
        purpose,side_id:None,
    }).unwrap();
    product.connection.execute("INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingS','instanceS','global','SESSION','sessionS','1','ACTIVE')").unwrap();
    product.connection.execute("INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('global','sessionS','instanceS','homeS','bindingS','1','COMMITTED',1,'processS')").unwrap();
    product.connection.execute("INSERT INTO gogoke_v37_native_selection VALUES('global','sessionS','seatS','incarnationS',1,'instanceS')").unwrap();
    product.connection.execute("INSERT INTO gogoke_v37_session_binding_v2 VALUES('global','sessionS','seatS','incarnationS',1,'instanceS','NATIVE_V2')").unwrap();
    product.connection.execute("INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('global','openS','sessionS','1','6f70656e',0,1,'processS','instanceS','homeS','bindingS','seatS','incarnationS','ACTIVE')").unwrap();
    product.connection.execute("INSERT INTO gogoke_v37_h_generation VALUES('global','sessionS','1','openS','processS')").unwrap();
    product.connection.execute("INSERT INTO gogoke_v37_h_operation VALUES('global','openS','6f70656e','open','sessionS','APPLIED',0,1)").unwrap();
    custodian.activate(&custody).unwrap();
    authority::mark_process_active(&mut product.connection, "processS", &custody).unwrap();
    let frames: Vec<_> = lines.iter().map(|_| custodian.read_persistent_child_frame(
        &custody.ticket, Duration::from_secs(5)).expect("native stdout frame")).collect();
    for (frame, line) in frames.iter().zip(&lines) {
        assert_eq!(frame.bytes(), format!("{line}\n").as_bytes());
    }
    let keys: Vec<_> = frames[..captured].iter().enumerate()
        .map(|(index, frame)| capture(&mut product, frame, index + 1)).collect();
    observed(&product, &custody, &keys[0], "thread-start",
        &codex_rpc::Command::ThreadStart {cwd:"fixture-directory".into(),model:"m".into()},
        codex_rpc::RpcId::Number(3));
    let command = codex_rpc::Command::TurnStart {thread_id:"threadS".into(),
        cwd:"fixture-directory".into(),model:"m".into(),effort:"high".into(),
        text:BODY.into(),network_access:Some(false)};
    let user = user_request();
    let input = h::StdinRequest {domain_id:"global",session_id:"sessionS",
        ticket:custody.ticket.opaque(),generation:"1",request_bytes:&user};
    h::prepare_codex_request(&mut product.connection, &input).unwrap();
    let source_id = "H-USER:global:sessionS:sendS";
    let user_ms = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap();
    seat::record_user_presence(&mut product.connection, &product.owner, source_id,
        seat::UserPresenceKind::Input, "processS", &custody.custodian_nonce, "sendS",
        user_ms, user_ms).unwrap();
    observed(&product, &custody, &keys[1], &format!("send-{}", &sha256_hex(&user)[..40]),
        &command, codex_rpc::RpcId::Number(4));
    h::complete_codex_turn_request(&mut product.connection, &input, &frames[1],
        &codex_rpc::RpcId::Number(4), &command, "threadS").unwrap();
    action(&mut product, &custody, &frames, &keys, &user);
    drop(custodian); drop(product); drop(root); fs::remove_dir_all(folder).unwrap();
}

fn fixture(extra: &[&str], action: impl FnOnce(&mut ProductDatabase<'_>, &PreparedCustody,
    &[OriginBoundFrame], &[ledger::RawSourceKey], &[u8])) {
    fixture_with_wire(CALL,extra,4,ledger::SessionPurpose::Secretary,action);
}

fn caller(product: &ProductDatabase<'_>, custody: &PreparedCustody,
    frame: &OriginBoundFrame, key: &ledger::RawSourceKey) -> seat::NativeSeatCall {
    model_call::observe_model_call(&product.connection, custody, frame, key,
        "threadS", "turnS").expect("original H/A factory must seal global secretary call")
}

fn rows(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    let q = Statement::prepare(product.connection.as_ptr(),
        "SELECT routine_id,original_text,source_operation_id,source_epoch,source_cursor,
         schedule_raw,timezone,CAST(next_due_ms AS TEXT),state,last_result
         FROM main.gogoke_v37_seat_secretary_routines ORDER BY routine_id").unwrap();
    let mut result = Vec::new();
    while q.step_row().unwrap() {
        result.push((0..10).map(|i| q.column_text(i).unwrap()).collect());
    }
    result
}

fn operation_count(product: &ProductDatabase<'_>) -> String {
    let q = Statement::prepare(product.connection.as_ptr(),
        "SELECT CAST(count(*) AS TEXT) FROM main.gogoke_v37_seat_secretary_routine_operations").unwrap();
    assert!(q.step_row().unwrap());
    q.column_text(0).unwrap()
}

fn future(original: &str, span: &str, zone: &str, input_ms: i64, now: i64)
    -> Result<ResolvedRoutineSchedule> {
    assert_eq!((original, span, zone), (BODY, SPAN, "HOST_DEFAULT"));
    assert!(input_ms > 0 && input_ms <= now);
    // Exercise the same production parser as the shared model dispatcher;
    // this fixture's observed host zone is explicit, not proposed by a model.
    let parsed=seat::secretary_schedule::resolve_user_schedule(original,span,zone,input_ms,"UTC")
        .expect("real host parser must accept the original fixture USER rule");
    assert_eq!(parsed.next_due_ms,input_ms+600_000);
    Ok(ResolvedRoutineSchedule {schedule_raw:span.into(), timezone:parsed.timezone,
        next_due_ms:parsed.next_due_ms})
}

#[test]
fn original_global_user_call_writes_one_routine_and_exact_replay_skips_resolution() {
    fixture(&[], |product, custody, frames, keys, _| {
        let key = &keys[3];
        let sealed = caller(product, custody, &frames[3], key);
        assert_eq!((sealed.domain_id(), sealed.tool()), ("global", Some("gogoke_routine")));
        let applied = product.dispatch_model_secretary_routine(&sealed, future).unwrap();
        assert!(String::from_utf8_lossy(&applied).contains("\"APPLIED\""));
        let before = rows(product);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0][1], BODY);
        assert_eq!(before[0][2], "processS");
        assert_eq!(before[0][3], "epochS");
        assert_eq!(before[0][4], "2");
        assert_eq!(before[0][5], SPAN);
        assert_eq!(before[0][6], "UTC");
        assert_eq!(before[0][8], "ACTIVE");
        assert_eq!(before[0][9], "NONE");
        let replay = product.dispatch_model_secretary_routine(&sealed, |_,_,_,_,_| panic!("replay must not resolve time again")).unwrap();
        assert!(String::from_utf8_lossy(&replay).contains("\"REPLAYED\""));
        assert_eq!(rows(product), before);
    });
}

#[test]
fn changed_user_marker_h_response_and_current_authority_never_write() {
    for (sql, boundary) in [
            ("DELETE FROM gogoke_v37_seat_secretary_presence", "missing INPUT marker"),
            ("UPDATE gogoke_v37_seat_secretary_presence SET source_cursor='other'", "wrong INPUT marker"),
            ("UPDATE gogoke_v37_h_stdin_journal SET request_hex='00' WHERE request_id='sendS'", "changed USER bytes"),
            ("UPDATE v37_ledger_raw_source SET raw_bytes=CAST(replace(CAST(raw_bytes AS TEXT),'\"id\":\"turnS\"','\"id\":\"otherTurn\"') AS BLOB) WHERE operation_id='processS' AND source_cursor='2'", "changed observed RPC response"),
            ("UPDATE gogoke_v37_h_claim SET generation='2' WHERE session_id='sessionS'", "wrong H generation"),
            ("UPDATE gogoke_v37_seats SET state='IDLE' WHERE seat_id='seatS'", "no current BUSY seat"),
            ("UPDATE v37_ledger_session SET purpose='WORK' WHERE session_id='sessionS'", "project purpose"),
            ("UPDATE v37_ledger_session SET purpose='SIDE_CHAT',side_id='sideS' WHERE session_id='sessionS'", "side purpose"),
        ] {
        fixture(&[], |product, custody, frames, keys, _| {
            let sealed = caller(product, custody, &frames[3], &keys[3]);
            product.connection.execute(sql).unwrap();
            let result = product.dispatch_model_secretary_routine(&sealed, future);
            assert!(result.is_err(), "{boundary}: {result:?}");
            assert!(rows(product).is_empty(), "{boundary}: E write escaped refusal");
        });
    }
    fixture(&[], |product, custody, frames, keys, _| {
        let wrong = PreparedCustody {custodian_nonce:"wrong-nonce".into(), ..custody.clone()};
        assert!(model_call::observe_model_call(&product.connection, &wrong, &frames[3], &keys[3], "threadS", "turnS").is_err());
        assert!(rows(product).is_empty());
    });
}

#[test]
fn model_extra_authority_or_unanchored_time_and_bad_host_resolution_do_not_write() {
    let extras = [
        CALL.replace("\"id\":91", "\"id\":92").replace("\"timezone\":\"HOST_DEFAULT\"", "\"timezone\":\"HOST_DEFAULT\",\"nextDueMs\":\"9999999999999\""),
        CALL.replace("\"id\":91", "\"id\":93").replace("\"timezone\":\"HOST_DEFAULT\"", "\"timezone\":\"HOST_DEFAULT\",\"nowMs\":\"1\""),
        CALL.replace("\"id\":91", "\"id\":94").replace("\"timezone\":\"HOST_DEFAULT\"", "\"timezone\":\"HOST_DEFAULT\",\"callerSeatId\":\"Owner\""),
        CALL.replace("\"id\":91", "\"id\":95").replace(SPAN, "every Tuesday at 09:00"),
        CALL.replace("\"id\":91", "\"id\":96").replace("HOST_DEFAULT", "America/New_York"),
    ];
    fixture(&[&extras[0], &extras[1], &extras[2], &extras[3], &extras[4]], |product, custody, frames, keys, _| {
        let sealed = caller(product, custody, &frames[3], &keys[3]);
        for (index, label) in [(4,"model due"),(5,"model now"),(6,"model identity"),
            (7,"different span"),(8,"different zone")] {
            let key = capture(product, &frames[index], index + 1);
            let candidate = model_call::observe_model_call(&product.connection, custody,
                &frames[index], &key, "threadS", "turnS").expect("H/A must seal the distinct model proposal");
            assert!(product.dispatch_model_secretary_routine(&candidate, future).is_err(), "{label}");
            assert!(rows(product).is_empty(), "{label}: no E write");
        }
        let overdue = product.dispatch_model_secretary_routine(&sealed, |_,span,_,_,now| {
            Ok(ResolvedRoutineSchedule {schedule_raw:span.into(),timezone:"UTC".into(),next_due_ms:now-1})
        });
        assert!(overdue.is_err());
        assert!(rows(product).is_empty());
        let failed = product.dispatch_model_secretary_routine(&sealed, |_,_,_,_,_| {
            Err(OrchestrationError::Invalid("host resolver failure"))
        });
        assert!(failed.is_err());
        assert!(rows(product).is_empty());
        product.connection.execute("CREATE TEMP TRIGGER secretary_operation_failure BEFORE INSERT ON main.gogoke_v37_seat_secretary_routine_operations BEGIN SELECT RAISE(FAIL,'injected E operation failure'); END").unwrap();
        assert!(product.dispatch_model_secretary_routine(&sealed, future).is_err(),
            "a failure after the routine INSERT must roll back the entire E write");
        assert!(rows(product).is_empty());
        assert_eq!(operation_count(product), "0", "failure must leave no reusable operation receipt");
        product.connection.execute("DROP TRIGGER temp.secretary_operation_failure").unwrap();
        let applied = product.dispatch_model_secretary_routine(&sealed, future).unwrap();
        assert!(String::from_utf8_lossy(&applied).contains("\"APPLIED\""),
            "the rolled-back attempt must not be mistaken for replay or UNKNOWN");
    });
}

// Synthetic C/H rows exercise the production takeover dispatcher. The CMD pipe
// supplies original A frames; this fixture does not prove a provider consumed
// the answer or that a real model made the call.
fn native_takeover_fixture(card_session: &str,
    action: impl FnOnce(&mut ProductDatabase<'_>, &seat::NativeSeatCall, &V37Request, &str, &str)) {
    fixture_with_wire(TAKEOVER_QUESTION,&[TAKEOVER_CALL],3,ledger::SessionPurpose::Work,
        |product,custody,frames,_,_| {
        product.connection.execute("INSERT INTO main.gogoke_v37_seat_settings(domain_id,seat_id,template_id,settings_json) VALUES('global','seatS','templateS','{\"model\":\"m\",\"effort\":\"high\",\"permissionTier\":\"READ_ONLY\",\"orchestrationScope\":{\"instanceIds\":[\"instanceS\"],\"models\":[\"m\"],\"reasoningEfforts\":[\"high\"],\"maxPermissionTier\":\"READ_ONLY\"},\"takeoverQuestions\":[{\"id\":\"q\",\"prompt\":\"Which scope?\"}]}')").unwrap();
        let question=ledger::capture_raw_source(&mut product.connection,&frames[3],"processS",
            &custody.custodian_nonce,"4").expect("original native question frame in A");
        let source=&question.key;
        let digest=sha256_hex(format!("global\n{card_session}\n{}\n{}\n{}",
            source.operation_id,source.source_epoch,source.source_cursor).as_bytes());
        let card_id=format!("card{digest}");
        let raise_id=format!("raise{digest}");
        let descriptor=Json::Object(BTreeMap::from([
            (JsonString::from_str("operationId"),Json::String(JsonString::from_str(&source.operation_id))),
            (JsonString::from_str("sourceEpoch"),Json::String(JsonString::from_str(&source.source_epoch))),
            (JsonString::from_str("sourceCursor"),Json::String(JsonString::from_str(&source.source_cursor))),
            (JsonString::from_str("frameSha256"),Json::String(JsonString::from_str(&sha256_hex(&question.raw_bytes)))),
        ])).canonical();
        let Json::Object(question_json)=Parser::parse(std::str::from_utf8(&question.raw_bytes).unwrap()).unwrap()
            else {panic!("native question frame must decode");};
        let payload=question_json.get(&JsonString::from_str("params")).unwrap().canonical();
        let card=inbox::NativeQuestion {vendor_request_id:"44",vendor_thread_id:"threadS",
            vendor_item_id:"itemS",auto_resolution_ms:"",question_payload:&payload,
            question_id:"q",header:"Scope",question:"Which scope?",
            answer_shape:inbox::NativeAnswerShape::FreeText,options:&[],seat_id:"seatS",
            turn_id:"turnS",generation:"1"};
        let raise=inbox::CardEnvelope {domain_id:"global",card_id:&card_id,request_id:&raise_id,
            request_bytes:descriptor.as_bytes(),expected_revision:0};
        assert_eq!(inbox::raise_native_card(&mut product.connection,&raise,&card,|_|Ok(true))
            .unwrap().phase,"RAISED");
        let answer_bytes=format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-QCARD","operation":"answer","requestId":"answerS","targetId":"{card_id}","domainId":"global","expectedRevision":"1","payload":{{"generation":"1","answers":{{"q":["Private testbed only"]}}}}}}"#);
        let answer=inbox::CardEnvelope {domain_id:"global",card_id:&card_id,request_id:"answerS",
            request_bytes:answer_bytes.as_bytes(),expected_revision:1};
        let command=codex_rpc::Command::QuestionAnswer {request_id:codex_rpc::RpcId::Number(44),
            answers:BTreeMap::from([("q".into(),vec!["Private testbed only".into()])])};
        let wire=command.encode(None).unwrap();
        let wire_text=std::str::from_utf8(wire.strip_suffix(b"\n").unwrap()).unwrap();
        assert_eq!(inbox::begin_native_answer_intent(&mut product.connection,&answer,"44",
            "seatS","turnS","1",inbox::NativeAnswer::Wire(wire_text),|_|Ok(true))
            .unwrap().disposition,inbox::NativeAnswerDisposition::New);
        let written=Statement::prepare(product.connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,
             step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,
             binary_digest,profile_id,generation,command_hex,requires_response,phase)
             VALUES('global','sessionS','openS','qanswerS','processS',?1,?2,?3,?4,?5,?6,?7,'1',?8,0,'WRITTEN')").unwrap();
        for (index,value) in [custody.ticket.opaque(),custody.custodian_nonce.as_str(),
            &custody.identity.pid.to_string(),&custody.identity.creation_time_100ns.to_string(),
            custody.identity.image_path.to_str().unwrap(),custody.binding.binary_digest_sha256.as_str(),
            custody.binding.profile_id.as_str(),&hex(&wire)].iter().enumerate() {
            written.bind_text((index+1) as i32,value).unwrap();
        }
        written.step_done().unwrap();drop(written);
        let settled=inbox::settle_native_answer_written(&mut product.connection,&product.owner,
            &answer,"sessionS","qanswerS").expect("production C settles exact H WRITTEN proof");
        assert_eq!(settled.phase,"ANSWERED");
        let call_key=capture(product,&frames[4],5);
        let sealed=caller(product,custody,&frames[4],&call_key);
        assert_eq!(sealed.tool(),Some("gogoke_takeover"));
        let request_bytes=format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-SEAT","operation":"takeover-answers","requestId":"takeoverS","targetId":"seatS","domainId":"global","expectedRevision":"1","payload":{{"cardId":"{card_id}","cardAnswerRequestId":"answerS","answerRevision":"0"}}}}"#);
        let request=h::decode_request(request_bytes.as_bytes()).unwrap();
        action(product,&sealed,&request,&card_id,&settled.native_receipt_id);
    });
}

fn native_takeover_stored_answer(product: &ProductDatabase<'_>) -> Option<(String,String,String)> {
    let row=Statement::prepare(product.connection.as_ptr(),
        "SELECT answer,source_ref,CAST(revision AS TEXT) FROM main.gogoke_v37_seat_takeover_answers WHERE domain_id='global' AND seat_id='seatS' AND question_id='q'").unwrap();
    let result=if row.step_row().unwrap() {Some((row.column_text(0).unwrap(),row.column_text(1).unwrap(),row.column_text(2).unwrap()))}
        else {None};
    assert!(!row.step_row().unwrap(),"one takeover answer per question");
    result
}

fn native_takeover_operation_count(product: &ProductDatabase<'_>) -> String {
    let row=Statement::prepare(product.connection.as_ptr(),
        "SELECT CAST(count(*) AS TEXT) FROM main.gogoke_v37_seat_continuity_operations WHERE domain_id='global' AND request_id='takeoverS'").unwrap();
    assert!(row.step_row().unwrap());
    row.column_text(0).unwrap()
}

#[test]
fn native_takeover_original_answer_applies_and_exact_raw_replays() {
    native_takeover_fixture("sessionS",|product,caller,request,card_id,receipt_id| {
        let applied=h::decode_receipt(&product.dispatch_native_takeover_answer(request,caller).unwrap()).unwrap();
        assert_eq!(applied.status,V37Status::Applied);
        assert_eq!(native_takeover_stored_answer(product),Some(("Private testbed only".into(),
            format!("C-QCARD:{card_id}:answerS:{receipt_id}"),"1".into())));
        assert_eq!(native_takeover_operation_count(product),"1");
        let replay=h::decode_receipt(&product.dispatch_native_takeover_answer(request,caller).unwrap()).unwrap();
        assert_eq!(replay.status,V37Status::Replayed,"only the original raw request may replay");
        assert_eq!(native_takeover_stored_answer(product),Some(("Private testbed only".into(),
            format!("C-QCARD:{card_id}:answerS:{receipt_id}"),"1".into())));
        assert_eq!(native_takeover_operation_count(product),"1");
    });
}

#[test]
fn native_takeover_rejects_another_session_card_identity() {
    native_takeover_fixture("sessionOther",|product,caller,request,_,_| {
        assert_eq!(caller.session_id(),Some("sessionS"));
        let denied=h::decode_receipt(&product.dispatch_native_takeover_answer(request,caller).unwrap()).unwrap();
        assert_eq!(denied.status,V37Status::Denied);
        assert_eq!(native_takeover_stored_answer(product),None);
        assert_eq!(native_takeover_operation_count(product),"0");
    });
}

#[test]
fn native_takeover_rejects_a_receipt_that_does_not_match_written_h() {
    native_takeover_fixture("sessionS",|product,caller,request,_,receipt_id| {
        assert!(receipt_id.starts_with("h-qanswer-"));
        product.connection.execute("UPDATE main.gogoke_v37_qcard_native_operations SET native_receipt_id='h-qanswer-wrong' WHERE domain_id='global' AND request_id='answerS'").unwrap();
        let denied=h::decode_receipt(&product.dispatch_native_takeover_answer(request,caller).unwrap()).unwrap();
        assert_eq!(denied.status,V37Status::Denied);
        assert_eq!(native_takeover_stored_answer(product),None);
        assert_eq!(native_takeover_operation_count(product),"0");
    });
}
