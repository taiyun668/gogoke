use super::*;
use crate::root::RootLock;
use crate::store::seat::{CreateSeat, Kind, StoreTemplate};
use crate::store::same_open::route_b_test_guard;
use std::time::{SystemTime, UNIX_EPOCH};

fn operation(family: &str, verb: &str, id: &str, target: &str, revision: u64, payload: &str) -> V37Request {
    decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"{family}","operation":"{verb}","requestId":"{id}","targetId":"{target}","domainId":"projectA","expectedRevision":"{revision}","payload":{payload}}}"#).as_bytes()).unwrap()
}

#[test]
fn actual_pinned_codex_product_open_records_rpc_and_durable_stop_without_model_call() {
    let _guard = route_b_test_guard();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-v37-real-session-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    let pin = instance::discover_program("codex").expect("actual cloud-pinned CLI catalog");
    instance::register_instance(&mut product.connection, &root, &instance::Registration {
        request_id: "register-cli", request_bytes: b"real cloud CLI registration",
        instance_id: "instanceA", driver_id: "codex", program: &pin,
    }).unwrap();
    // Arrange the login-admission field only. This test never claims Owner
    // authentication or a model response: its real CLI calls stop at thread/start.
    for (id, revision, observation) in [
        ("real-install-observation", 1, instance::InstanceObservation::Installed),
        ("fixture-login-admission", 2, instance::InstanceObservation::LoggedIn),
    ] {
        instance::record_observation(&mut product.connection, &root, &instance::ObservationRequest {
            request_id: id, request_bytes: id.as_bytes(), instance_id: "instanceA",
            expected_revision: revision, observation,
        }).unwrap();
    }
    instance::set_instance_concurrency_cap(&mut product.connection, &product.owner, "instanceA", 1).unwrap();
    seat::set_project_parallel_cap(&mut product.connection, &product.owner, "projectA", 1).unwrap();
    seat::store_template(&mut product.connection, NativeOrigin::user(&product.owner), StoreTemplate {
        domain_id: "projectA", template_id: "templateA",
        settings_json: br#"{"effort":"high","model":"gpt-6-sol","permissionTier":"NETWORKED_WRITE"}"#,
    }).unwrap();
    seat::create(&mut product.connection, NativeOrigin::user(&product.owner), CreateSeat {
        domain_id: "projectA", seat_id: "seatA", template_id: "templateA", instance_id: Some("instanceA"),
        kind: Kind::Long, request_id: "create-seat", request_bytes: b"real session seat fixture",
    }).unwrap();
    let source = crate::store::worktree::tests::make_source_fixture(&mut product.connection, &root,
        &product.owner, &mut product.process_custodian);
    let git = std::env::var_os("GOGOKE_CONTROLLED_GIT_PATH").unwrap();
    let configuration = Json::Object(BTreeMap::from([
        (JsonString::from_str("schema"), Json::String(JsonString::from_str("gogoke.37.owner-configuration.v1"))),
        (JsonString::from_str("command"), Json::String(JsonString::from_str("worktree-source"))),
        (JsonString::from_str("repositoryId"), Json::String(JsonString::from_str("fixtureRepo"))),
        (JsonString::from_str("sourcePath"), Json::String(JsonString::from_str(source.to_str().unwrap()))),
        (JsonString::from_str("gitPath"), Json::String(JsonString::from_str(git.to_str().unwrap()))),
    ])).canonical();
    product.configure_user_v37(configuration.as_bytes()).unwrap();
    let create = operation("K-WORKTREE", "create", "create-tree", "treeA", 0,
        r#"{"repositoryId":"fixtureRepo","seatId":"seatA"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&create).unwrap()).unwrap().status, V37Status::Applied);
    for (verb, id, revision) in [("admission-reserve", "reserve-session", 0), ("admission-commit", "commit-session", 1)] {
        let request = operation("K-SESSION", verb, id, "sessionA", revision, r#"{"seatId":"seatA","generation":"2"}"#);
        assert_eq!(h::decode_receipt(&product.dispatch_user_request(&request).unwrap()).unwrap().status, V37Status::Applied);
    }
    let open = operation("K-SESSION", "open", "open-session", "sessionA", 2,
        r#"{"seatId":"seatA","generation":"2","repositoryId":"fixtureRepo","worktreeId":"treeA"}"#);
    let receipt = product.dispatch_user_request(&open).expect("actual CLI open original native error");
    assert_eq!(h::decode_receipt(&receipt).unwrap().status, V37Status::Applied);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&open).unwrap()).unwrap().status, V37Status::Replayed);
    let output=operation("K-SESSION","output-stream","read-native-output","sessionA",2,
        r#"{"generation":"2","afterCursor":"0"}"#);
    let actual_output=h::decode_receipt(&product.dispatch_user_request(&output).unwrap()).unwrap();
    assert_eq!(actual_output.status,V37Status::Applied);
    assert_eq!(actual_output.previous_revision,actual_output.revision,"output read never advances H");
    // A SQLite highwater control checks read-receipt currentness. This is not
    // another provider output or authentication observation.
    product.connection.execute("INSERT INTO sqlite_sequence(name,seq) SELECT 'v37_ledger_index',1 WHERE NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='v37_ledger_index')").unwrap();
    product.connection.execute("UPDATE sqlite_sequence SET seq=seq+1 WHERE name='v37_ledger_index'").unwrap();
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&output).unwrap()).unwrap().status,V37Status::Stale);
    let collision=operation("K-SESSION","stop","read-native-output","sessionA",2,
        r#"{"seatId":"seatA","generation":"2"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&collision).unwrap()).unwrap().status,V37Status::Conflict);
    let row = Statement::prepare(product.connection.as_ptr(), "SELECT count(*) FROM main.gogoke_v37_rpc_steps WHERE phase='OBSERVED'").unwrap();
    assert!(row.step_row().unwrap());
    assert_eq!(row.column_text(0).unwrap(), "3", "initialize, effective config and real thread response; initialized has no ACK");
    drop(row);
    let key = ("projectA".to_owned(), "sessionA".to_owned());
    let live = product.native_sessions.get(&key).unwrap();
    let custody = live.custody.clone();
    let reused_open = operation("K-SESSION", "send", "open-session", "sessionA", 2,
        r#"{"generation":"2","body":"must never reach the provider"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&reused_open).unwrap()).unwrap().status,
        V37Status::Conflict);
    let reserved_send = operation("K-SESSION", "send", "reserved-send", "sessionA", 2,
        r#"{"generation":"2","body":"intention only"}"#);
    let reserved_input = h::StdinRequest { domain_id: "projectA", session_id: "sessionA",
        ticket: custody.ticket.opaque(), generation: "2", request_bytes: &reserved_send.raw_bytes };
    h::prepare_codex_request(&mut product.connection, &reserved_input).unwrap();
    let reused_send = operation("K-SESSION", "stop", "reserved-send", "sessionA", 2,
        r#"{"seatId":"seatA","generation":"2"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&reused_send).unwrap()).unwrap().status,
        V37Status::Conflict);
    assert!(product.native_sessions.contains_key(&key), "conflict does not stop the actual child");
    let cwd=product.native_sessions.get(&key).unwrap().evidence.cwd().to_string_lossy().into_owned();
    product.connection.execute("CREATE TRIGGER inject_pending_capture_failure BEFORE INSERT ON v37_ledger_raw_source BEGIN SELECT RAISE(ABORT,'injected retained provider capture failure'); END").unwrap();
    assert!(product.native_rpc(&key,"capture-fault-config",Some(9),&Command::ConfigRead {cwd}).is_err());
    assert!(product.native_sessions.get(&key).unwrap().raw_capture.has_pending(),
        "the actual fourth CLI response remains in original native custody");
    let stop = operation("K-SESSION", "stop", "stop-session", "sessionA", 2, r#"{"seatId":"seatA","generation":"2"}"#);
    assert!(product.dispatch_user_request(&stop).is_err(),"original capture failure still reported after actual stop");
    let retained=product.native_sessions.get(&key).unwrap();
    let proof=retained.stop_proof.as_ref().expect("stop reached the actual Job despite capture SQL failure");
    assert!(proof.parent_exited && proof.active_job_processes==Some(0) && proof.writer_fence_verified);
    assert!(proof.errors.is_empty(),"original native stop evidence: {proof:?}");
    assert!(retained.raw_capture.has_pending(),"stop cannot drop the original uncaptured response");
    assert!(product.process_custodian.active(&custody.ticket).is_some(),"proof and guards retained until durable confirmation");
    product.connection.execute("DROP TRIGGER inject_pending_capture_failure").unwrap();
    // Inject a same-store write failure after the real native stop and its
    // custody proof. The original request must finish without a second stop.
    product.connection.execute("CREATE TRIGGER inject_h_stop_failure BEFORE UPDATE ON gogoke_v37_h_claim WHEN NEW.state='STOPPED' BEGIN SELECT RAISE(ABORT,'injected H stop receipt failure'); END").unwrap();
    assert!(product.dispatch_user_request(&stop).is_err());
    assert!(product.native_sessions.get(&key).unwrap().stop_proof.is_some());
    assert!(!product.native_sessions.get(&key).unwrap().raw_capture.has_pending(),"same original response captured after SQL fault removed");
    product.connection.execute("DROP TRIGGER inject_h_stop_failure").unwrap();
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Applied);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Replayed);
    let fact = runtime::observe_stop_fact(&product.connection, "projectA", "sessionA").unwrap();
    assert!(fact.is_some(), "actual native Job and same-store durable stop fact");
    // Model a lost H commit across restart, retaining the actually persisted
    // native STOPPED proof. Recovery must not require live OS handles.
    product.connection.execute("UPDATE gogoke_v37_h_operation SET status='UNKNOWN',revision=2 WHERE request_id='stop-session'").unwrap();
    product.connection.execute("UPDATE gogoke_v37_h_claim SET state='COMMITTED',revision=2,stop_fact_id=NULL WHERE session_id='sessionA'").unwrap();
    product.close_checked().unwrap();
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&open).unwrap()).unwrap().status, V37Status::Replayed);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Replayed);
    let release = operation("K-SESSION", "admission-release", "release-session", "sessionA", 3,
        r#"{"seatId":"seatA","generation":"2"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&release).unwrap()).unwrap().status, V37Status::Applied);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Replayed);
    product.close_checked().unwrap();
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Replayed);
    product.close_checked().unwrap();
    drop(root);
    std::fs::remove_dir_all(path).unwrap();
}
