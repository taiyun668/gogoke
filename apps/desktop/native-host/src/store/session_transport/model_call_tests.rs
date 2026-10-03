//! Permission boundary controls for the production H/A caller factory.
//! Protocol bytes and H admission rows are synthetic; stdout, custody and A
//! capture are real. These controls do not prove CLI login or LPAC admission.

use super::*;
use super::super::{admission, journal};
use crate::process::{NativeBinding, PrepareRequest, ProcessCustodian, ProcessLaunch, StopBudgets};
use crate::root::RootLock;
use crate::store::{authority, instance};
use crate::store::digest::content_hash;
use crate::store::same_open::{create_new, route_b_test_guard};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USER: &[u8] = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"sendA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"body":"hello","generation":"1"}}"#;
const THREAD: &str = r#"{"id":3,"result":{"thread":{"id":"threadA","cwd":"fixture-directory"}}}"#;
const TURN: &str = r#"{"id":4,"result":{"turn":{"id":"turnA","status":"inProgress"}}}"#;
const STARTED: &str = r#"{"method":"turn/started","params":{"threadId":"threadA","turn":{"id":"turnA","status":"inProgress"}}}"#;
const CALL: &str = r#"{"id":91,"method":"item/tool/call","params":{"callId":"callA","threadId":"threadA","turnId":"turnA","tool":"gogoke_seat","arguments":{"callerSeatId":"Owner","callerGrant":"Owner","action":"self-authorize"}}}"#;
const ENDED: &str = r#"{"method":"turn/completed","params":{"threadId":"threadA","turn":{"id":"turnA","status":"completed"}}}"#;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// Same signed-system-command / suspended Job custody approach as A's
// raw_process_fixture. No constructor fabricates an OriginBoundFrame.
fn process(folder: &Path, lines: &[&str]) -> (ProcessCustodian, PreparedCustody) {
    let command = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
        .join("System32").join("cmd.exe");
    // Relay bytes from a file so CMD/CRT quote handling never transforms JSON.
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let file_name = format!("wire-{stamp}.jsonl");
    fs::write(folder.join(&file_name), format!("{}\n", lines.join("\n"))).unwrap();
    let mut launch = ProcessLaunch::new(command.clone());
    launch.arguments = vec!["/D".into(), "/C".into(), format!("type {file_name}")];
    launch.current_directory = Some(folder.to_owned());
    launch.protocol_stdio = true;
    launch.persistent_protocol_stdio = true;
    let mut custodian = ProcessCustodian::new().expect("native custodian");
    let custody = custodian.prepare(&PrepareRequest {
        binding: NativeBinding {
            binary_digest_sha256: content_hash(&fs::read(&command).expect("signed cmd bytes")),
            profile_id: "profile-model-call-test".into(), domain_id: "projectA".into(),
            generation: "1".into(),
        }, launch,
    }).expect("native suspended process");
    (custodian, custody)
}

fn capture(db: &mut VerifiedDatabaseConnection<'_>, frame: &OriginBoundFrame,
    cursor: usize) -> RawSourceKey {
    let source = ledger::capture_raw_source(db, frame, "processA", "epochA",
        &cursor.to_string()).expect("production A capture of native stdout");
    assert_eq!(source.raw_bytes, frame.bytes(), "A must retain the original pipe bytes");
    source.key
}

fn observed(db: &mut VerifiedDatabaseConnection<'_>, custody: &PreparedCustody,
    key: &RawSourceKey, step: &str, command: &codex_rpc::Command, id: codex_rpc::RpcId) {
    // Reuse journal's historical OBSERVED-step fixture. Production encoding
    // and receipt completion are used; this does not claim a CLI write.
    let insert = Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,
         step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,
         binary_digest,profile_id,generation,command_hex,requires_response,phase,
         source_epoch,source_cursor) VALUES('projectA','sessionA','openA',?1,'processA',
         ?2,?3,?4,?5,?6,?7,?8,'1',?9,1,'OBSERVED','epochA',?10)").unwrap();
    for (index, value) in [step.to_owned(), custody.ticket.opaque().to_owned(),
        custody.custodian_nonce.clone(), custody.identity.pid.to_string(),
        custody.identity.creation_time_100ns.to_string(),
        custody.identity.image_path.to_string_lossy().into_owned(),
        custody.binding.binary_digest_sha256.clone(), custody.binding.profile_id.clone(),
        hex(&command.encode(Some(&id)).unwrap()), key.source_cursor.clone()]
        .iter().enumerate() {
        insert.bind_text((index + 1) as i32, value).unwrap();
    }
    insert.step_done().unwrap();
    drop(insert);
    let update = Statement::prepare(db.as_ptr(),
        "UPDATE main.v37_ledger_raw_source SET state='NO_EVENT',
         no_event_reason='CODEX_RPC_RESPONSE' WHERE operation_id='processA'
         AND source_epoch='epochA' AND source_cursor=?1").unwrap();
    update.bind_text(1, &key.source_cursor).unwrap(); update.step_done().unwrap();
}

fn with_source(extra: &[&str], action: impl FnOnce(&mut VerifiedDatabaseConnection<'_>,
    &mut ProcessCustodian, &PreparedCustody, &[OriginBoundFrame], &[RawSourceKey], &Path)) {
    let _guard = route_b_test_guard();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let folder = std::env::temp_dir().join(format!("gogoke-model-call-{}-{stamp}", std::process::id()));
    fs::create_dir(&folder).unwrap();
    let root = RootLock::acquire(&folder).unwrap();
    let mut db = create_new(&root, &folder.join("state.sqlite")).unwrap();
    instance::initialize_schema(&mut db).unwrap();
    seat::initialize_schema(&mut db).unwrap();
    admission::initialize_admission_schema(&mut db).unwrap();
    authority::initialize_process_custody_schema(&mut db).unwrap();
    rpc_journal::initialize_schema(&mut db).unwrap();
    db.execute("CREATE TABLE orchestration_events(sequence INTEGER PRIMARY KEY,event_id TEXT UNIQUE,stream_id TEXT,occurred_at TEXT,event_type TEXT,payload_json TEXT)").unwrap();
    ledger::initialize_schema(&mut db).unwrap();

    let lines = [vec![THREAD, TURN, STARTED, CALL], extra.to_vec()].concat();
    let (mut custodian, custody) = process(&folder, &lines);
    authority::record_prepared_process(&mut db, "processA", &custody).unwrap();
    // Synthetic H admission controls in production schemas. The cmd digest
    // labels this fixture only, never a fixed CLI catalog or login result.
    let insert = Statement::prepare(db.as_ptr(),
        "INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,
         program_digest,version,install_state,login_state,revision)
         VALUES('instanceA','codex','fixture-home','fixture-home-identity',?1,
         '0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
    insert.bind_text(1, &custody.binding.binary_digest_sha256).unwrap();
    insert.step_done().unwrap(); drop(insert);
    db.execute("INSERT INTO gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES('homeA','instanceA','projectA','SESSION','sessionA','1','ACTIVE',1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('projectA','seatA','incarnationA','USER','LONG','instanceA','BUSY',1,1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingA','instanceA','projectA','SESSION','sessionA','1','ACTIVE')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA','sessionA','instanceA','homeA','bindingA','1','COMMITTED',1,'processA')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_seat_binding VALUES('projectA','sessionA','seatA','incarnationA','1')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('projectA','openA','sessionA','1','6f70656e',0,1,'processA','instanceA','homeA','bindingA','seatA','incarnationA','ACTIVE')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_generation VALUES('projectA','sessionA','1','openA','processA')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_operation VALUES('projectA','openA','6f70656e','open','sessionA','APPLIED',0,1)").unwrap();
    custodian.activate(&custody).unwrap();
    authority::mark_process_active(&mut db, "processA", &custody).unwrap();
    let frames: Vec<_> = lines.iter().map(|_| custodian.read_persistent_child_frame(
        &custody.ticket, Duration::from_secs(5)).expect("original native stdout frame")).collect();
    for (frame, line) in frames.iter().zip(&lines) {
        assert_eq!(frame.bytes(), format!("{line}\n").as_bytes(), "relay must preserve exact protocol bytes");
    }
    let keys: Vec<_> = frames[..4].iter().enumerate()
        .map(|(index, frame)| capture(&mut db, frame, index + 1)).collect();
    observed(&mut db, &custody, &keys[0], "thread-start",
        &codex_rpc::Command::ThreadStart {cwd: "fixture-directory".into(), model: "m".into()},
        codex_rpc::RpcId::Number(3));
    let command = codex_rpc::Command::TurnStart {thread_id: "threadA".into(),
        cwd: "fixture-directory".into(), model: "m".into(), effort: "high".into(),
        text: "hello".into(), network_access: Some(false)};
    let input = journal::StdinRequest {domain_id: "projectA", session_id: "sessionA",
        ticket: custody.ticket.opaque(), generation: "1", request_bytes: USER};
    journal::prepare_codex_request(&mut db, &input).unwrap();
    observed(&mut db, &custody, &keys[1], &format!("send-{}", &sha256_hex(USER)[..40]),
        &command, codex_rpc::RpcId::Number(4));
    journal::complete_codex_turn_request(&mut db, &input, &frames[1],
        &codex_rpc::RpcId::Number(4), &command, "threadA").unwrap();
    action(&mut db, &mut custodian, &custody, &frames, &keys, &folder);
    drop(custodian);
    db.close_checked().unwrap(); drop(root); fs::remove_dir_all(folder).unwrap();
}

fn caller(db: &VerifiedDatabaseConnection<'_>, custody: &PreparedCustody,
    frame: &OriginBoundFrame, key: &RawSourceKey) -> NativeSeatCall {
    observe_model_call(db, custody, frame, key, "threadA", "turnA")
        .expect("production factory must seal the original live H/A call")
}

fn denied<T: std::fmt::Debug>(result: Result<T>, boundary: &str) {
    assert!(matches!(result, Err(ModelCallError::Denied | ModelCallError::Conflict)
        | Err(ModelCallError::Rpc(rpc_journal::RpcJournalError::Denied))),
        "{boundary}: expected an authority denial, got {result:?}");
}

#[test]
fn original_a_factory_seals_h_identity_and_stable_typed_rpc_id() {
    let string_id = CALL.replacen("\"id\":91", "\"id\":\"91\"", 1);
    with_source(&[CALL, &string_id], |db, _, custody, frames, keys, _| {
        let sealed = caller(db, custody, &frames[3], &keys[3]);
        assert_eq!((sealed.domain_id(), sealed.seat_id(), sealed.incarnation(),
            sealed.generation(), sealed.turn_id()), ("projectA", "seatA", "incarnationA", 1, "turnA"),
            "model arguments claiming Owner must never select or authorize the caller");
        assert_eq!(sealed.thread_id(), Some("threadA"), "thread authority comes from H/A");
        assert_eq!(sealed.raw_request_bytes(), Some(frames[3].bytes()), "seal binds actual captured bytes");
        assert_eq!(sealed.source_locator(), Some(&keys[3]), "seal binds actual A location");
        assert_eq!(recover_model_call_from_source(db, custody, &keys[3], "threadA", "turnA").unwrap(), sealed,
            "durable reread must preserve the original sealed caller");
        let later_key = capture(db, &frames[4], 5);
        let later = caller(db, custody, &frames[4], &later_key);
        assert_ne!(sealed.source_locator(), later.source_locator(), "control actually moves the raw cursor");
        assert_eq!(sealed.host_request_id(), later.host_request_id(), "same original typed RPC must not gain a new host request on redelivery");
        let string_key = capture(db, &frames[5], 6);
        let string_call = caller(db, custody, &frames[5], &string_key);
        assert_eq!(sealed.typed_rpc_id(), Some(&codex_rpc::RpcId::Number(91)));
        assert_eq!(string_call.typed_rpc_id(), Some(&codex_rpc::RpcId::String("91".into())));
        assert_ne!(sealed.host_request_id(), string_call.host_request_id(), "numeric and string RPC identities cannot share authority");
    });
}

#[test]
fn missing_source_wrong_turn_and_another_native_child_cannot_mint_caller() {
    with_source(&[CALL], |db, _, custody, frames, keys, folder| {
        let sealed = caller(db, custody, &frames[3], &keys[3]);
        let missing = RawSourceKey {source_cursor: "5".into(), ..keys[3].clone()};
        denied(observe_model_call(db, custody, &frames[4], &missing, "threadA", "turnA"), "uncaptured output cannot become authority");
        denied(observe_model_call(db, custody, &frames[0], &keys[3], "threadA", "turnA"), "source key cannot substitute other pipe bytes");
        for (thread, turn) in [("otherThread", "turnA"), ("threadA", "otherTurn")] {
            denied(observe_model_call(db, custody, &frames[3], &keys[3], thread, turn), "caller-selected thread or turn cannot replace the original turn");
        }
        let forged = PreparedCustody {custodian_nonce: "forged-nonce".into(), ..custody.clone()};
        denied(observe_model_call(db, &forged, &frames[3], &keys[3], "threadA", "turnA"), "claimed custody cannot replace native frame custody");
        let (mut other, other_custody) = process(folder, &[CALL]);
        other.activate(&other_custody).unwrap();
        let other_frame = other.read_persistent_child_frame(&other_custody.ticket, Duration::from_secs(5)).unwrap();
        assert_eq!(other_frame.bytes(), frames[3].bytes(), "different-origin control must retain identical protocol bytes");
        denied(observe_model_call(db, custody, &other_frame, &keys[3], "threadA", "turnA"), "identical bytes from another child carry no original H authority");
        denied(recover_model_call_from_source(db, &other_custody, &keys[3], "threadA", "turnA"), "durable recovery cannot relabel another physical custody");
        drop(other);
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert_eq!(revalidate_model_call_in_transaction(db, &sealed).unwrap().seat_id, "seatA", "denial controls must leave original authorization intact");
        db.execute("COMMIT").unwrap();
    });
}

#[test]
fn changed_bytes_under_same_typed_rpc_and_completed_turn_revoke_authority() {
    let changed = CALL.replace("self-authorize", "different-action");
    with_source(&[&changed, ENDED], |db, _, custody, frames, keys, _| {
        let sealed = caller(db, custody, &frames[3], &keys[3]);
        let changed_key = capture(db, &frames[4], 5);
        assert!(matches!(observe_model_call(db, custody, &frames[4], &changed_key, "threadA", "turnA"), Err(ModelCallError::Conflict)),
            "same typed RPC identity with different raw bytes cannot acquire a second meaning");
        assert!(matches!(recover_model_call_from_source(db, custody, &keys[3], "threadA", "turnA"), Err(ModelCallError::Conflict)),
            "ambiguous original identity cannot be recovered as an authorized call");
        // Remove only this synthetic conflicting observation to independently
        // exercise the turn-end boundary against the original production seal.
        db.execute("DELETE FROM main.v37_ledger_raw_source WHERE operation_id='processA' AND source_cursor='5'").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        revalidate_model_call_in_transaction(db, &sealed).expect("positive control before turn end");
        db.execute("COMMIT").unwrap();
        capture(db, &frames[5], 6);
        db.execute("BEGIN IMMEDIATE").unwrap();
        denied(revalidate_model_call_in_transaction(db, &sealed), "a sealed model caller cannot write after its original turn completed");
        db.execute("COMMIT").unwrap();
        denied(observe_model_call(db, custody, &frames[3], &keys[3], "threadA", "turnA"), "ended turn cannot mint a fresh caller");
    });
}

#[test]
fn sealed_call_rechecks_current_h_claim_source_and_actual_native_stop() {
    with_source(&[], |db, custodian, custody, frames, keys, _| {
        let sealed = caller(db, custody, &frames[3], &keys[3]);
        for (sql, boundary) in [
            ("UPDATE gogoke_v37_h_claim SET state='UNKNOWN'", "uncertain current claim carries no write authority"),
            ("UPDATE gogoke_v37_h_claim SET generation='2'", "replacement generation cannot inherit the old seal"),
            ("UPDATE gogoke_v37_h_owner_binding SET state='REVOKED'", "revoked Owner binding cannot authorize model output"),
            ("UPDATE gogoke_v37_seats SET state='IDLE'", "inactive seat cannot spend an old seal"),
            ("UPDATE gogoke_v37_h_process_episode SET seat_incarnation='replacement'", "replacement incarnation cannot reuse caller identity"),
            ("DELETE FROM gogoke_v37_h_stdin_journal WHERE request_id='sendA'", "model turn strings cannot replace the original user authorization"),
            ("DELETE FROM v37_ledger_raw_source WHERE operation_id='processA' AND source_cursor='3'", "call bytes alone cannot manufacture a started turn"),
        ] {
            db.execute("BEGIN IMMEDIATE").unwrap(); db.execute(sql).unwrap();
            denied(revalidate_model_call_in_transaction(db, &sealed), boundary);
            db.execute("ROLLBACK").unwrap();
            db.execute("BEGIN IMMEDIATE").unwrap();
            revalidate_model_call_in_transaction(db, &sealed).expect("restored H positive control");
            db.execute("COMMIT").unwrap();
        }
        db.execute("BEGIN IMMEDIATE").unwrap();
        let changed = CALL.replace("self-authorize", "different-action");
        let update = Statement::prepare(db.as_ptr(), "UPDATE main.v37_ledger_raw_source SET raw_bytes=?1 WHERE operation_id='processA' AND source_cursor='4'").unwrap();
        update.bind_blob(1, changed.as_bytes()).unwrap(); update.step_done().unwrap(); drop(update);
        denied(revalidate_model_call_in_transaction(db, &sealed), "captured byte changes invalidate the original sealed permission");
        db.execute("ROLLBACK").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        revalidate_model_call_in_transaction(db, &sealed).expect("restored original A positive control");
        db.execute("COMMIT").unwrap();
        let stop = custodian.stop(&custody.ticket, StopBudgets::production(), || Ok(())).unwrap();
        authority::mark_process_stopped(db, "processA", &stop).expect("actual native stop proof");
        db.execute("BEGIN IMMEDIATE").unwrap();
        denied(revalidate_model_call_in_transaction(db, &sealed), "actual stopped process cannot authorize further model writes");
        db.execute("COMMIT").unwrap();
    });
}
