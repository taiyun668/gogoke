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
    assert_eq!(h::decode_receipt(&receipt).unwrap().previous_revision,2);
    assert_eq!(h::decode_receipt(&receipt).unwrap().revision,3,"open write and receipt advance the owning H revision together");
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&open).unwrap()).unwrap().status, V37Status::Replayed);
    let stale_output=operation("K-SESSION","output-stream","old-open-revision","sessionA",2,
        r#"{"generation":"2","afterCursor":"0"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stale_output).unwrap()).unwrap().status,V37Status::Stale);
    let output=operation("K-SESSION","output-stream","read-native-output","sessionA",3,
        r#"{"generation":"2","afterCursor":"0"}"#);
    let actual_output=h::decode_receipt(&product.dispatch_user_request(&output).unwrap()).unwrap();
    assert_eq!(actual_output.status,V37Status::Applied,"original receipt: {}",String::from_utf8_lossy(&actual_output.raw_bytes));
    assert_eq!(actual_output.previous_revision,actual_output.revision,"output read never advances H");
    // A SQLite highwater control checks read-receipt currentness. This is not
    // another provider output or authentication observation.
    product.connection.execute("INSERT INTO sqlite_sequence(name,seq) SELECT 'v37_ledger_index',1 WHERE NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='v37_ledger_index')").unwrap();
    product.connection.execute("UPDATE sqlite_sequence SET seq=seq+1 WHERE name='v37_ledger_index'").unwrap();
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&output).unwrap()).unwrap().status,V37Status::Stale);
    let collision=operation("K-SESSION","stop","read-native-output","sessionA",3,
        r#"{"seatId":"seatA","generation":"2"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&collision).unwrap()).unwrap().status,V37Status::Conflict);
    let row = Statement::prepare(product.connection.as_ptr(), "SELECT count(*) FROM main.gogoke_v37_rpc_steps WHERE phase='OBSERVED'").unwrap();
    assert!(row.step_row().unwrap());
    assert_eq!(row.column_text(0).unwrap(), "3", "initialize, effective config and real thread response; initialized has no ACK");
    drop(row);
    let native_flags=operation("K-SESSION","capability-probe","loaded-features-open","sessionA",3,
        r#"{"generation":"2"}"#);
    let observed_flags=h::decode_receipt(&product.dispatch_user_request(&native_flags).unwrap()).unwrap();
    assert_eq!(observed_flags.status,V37Status::Applied,"original receipt: {}",String::from_utf8_lossy(&observed_flags.raw_bytes));
    let observed_flags=observed_flags.into_result();
    let Some(Json::Object(features))=observed_flags.get(&JsonString::from_str("loadedThreadFeatures")) else {
        panic!("loaded thread feature response absent");
    };
    assert!(matches!(features.get(&JsonString::from_str("memories")),Some(Json::Bool(false))));
    assert!(matches!(features.get(&JsonString::from_str("multi_agent_v2")),Some(Json::Bool(false))));
    assert!(matches!(features.get(&JsonString::from_str("default_mode_request_user_input")),Some(Json::Bool(true))));
    let key = ("projectA".to_owned(), "sessionA".to_owned());
    // Instrument-only contradictory candidate generation: a mutable H claim
    // alone cannot authorize reading a different physical session as old.
    let physical_generation=product.native_sessions.get(&key).unwrap().custody.binding.generation.clone();
    product.native_sessions.get_mut(&key).unwrap().custody.binding.generation="3".into();
    let mismatched_output=operation("K-SESSION","output-stream","physical-output-mismatch","sessionA",3,
        r#"{"generation":"2","afterCursor":"0"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&mismatched_output).unwrap()).unwrap().status,V37Status::Conflict);
    let mismatched_capability=operation("K-SESSION","capability-probe","physical-capability-mismatch","sessionA",3,
        r#"{"generation":"2"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&mismatched_capability).unwrap()).unwrap().status,V37Status::Conflict);
    product.native_sessions.get_mut(&key).unwrap().custody.binding.generation=physical_generation;
    let live = product.native_sessions.get(&key).unwrap();
    let custody = live.custody.clone();
    let process_operation=live.operation_id.clone();
    let thread=live.thread_id.clone().unwrap();
    // Fixed 0.149 starts with deferred history. Native inject_response_items
    // explicitly flushes its recorded items, so arrange real durable history
    // through the existing production RPC before testing cross-process resume.
    // This is no-model setup; it neither fabricates rollout files nor proves
    // authenticated model behavior or a user-facing append receipt.
    let history=product.native_append_rpc(&key,"materialize-original-history",&thread,
        "cloud no-model durable history marker".into()).unwrap();
    assert!(matches!(history,Some(Reply::Ack {..})),"actual original history injection ACK: {history:?}");
    // These are explicitly synthetic A/C ordering controls, not provider
    // questions, EOF observations, Owner login or model-delivery evidence.
    // The production composition below still uses the real opened session.
    let maximum=Statement::prepare(product.connection.as_ptr(),"SELECT COALESCE(MAX(CAST(source_cursor AS INTEGER)),0) FROM main.v37_ledger_raw_source WHERE operation_id=?1").unwrap();
    maximum.bind_text(1,&process_operation).unwrap();assert!(maximum.step_row().unwrap());
    let mut cursor=maximum.column_text(0).unwrap().parse::<u64>().unwrap();drop(maximum);
    let question=|id:i64| format!("{{\"id\":{id},\"method\":\"item/tool/requestUserInput\",\"params\":{{\"threadId\":{},\"turnId\":\"syntheticTurn\",\"itemId\":\"syntheticItem\",\"questions\":[{{\"id\":\"q\",\"header\":\"Choose\",\"question\":\"Which?\",\"isOther\":true,\"isSecret\":false,\"options\":null}}]}}}}\n",Json::String(JsonString::from_str(&thread)).canonical());
    let resolved=format!("{{\"method\":\"serverRequest/resolved\",\"params\":{{\"threadId\":{},\"requestId\":1001}}}}\n",Json::String(JsonString::from_str(&thread)).canonical());
    let tail=format!("{{\"method\":\"item/agentMessage/delta\",\"params\":{{\"threadId\":{},\"turnId\":\"syntheticTurn\",\"itemId\":\"syntheticItem\",\"delta\":\"synthetic EOF tail\"}}}}\n",Json::String(JsonString::from_str(&thread)).canonical());
    let append=|product:&mut ProductDatabase<'_>,cursor:&mut u64,bytes:&[u8]| {
        *cursor+=1;
        let raw=Statement::prepare(product.connection.as_ptr(),"INSERT INTO main.v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state) VALUES(?1,?2,?3,'projectA','sessionA','2',?3,?4,?5,'PENDING')").unwrap();
        raw.bind_text(1,&process_operation).unwrap();raw.bind_text(2,custody.ticket.opaque()).unwrap();
        raw.bind_text(3,&custody.custodian_nonce).unwrap();raw.bind_text(4,&cursor.to_string()).unwrap();raw.bind_blob(5,bytes).unwrap();raw.step_done().unwrap();
    };
    // Synthetic compound intent on the actual opened native session. This
    // checks C's production fence without sending compact or a model input.
    product.native_sessions.get_mut(&key).unwrap().turn_id=Some("syntheticTurn".into());
    let queued=operation("K-INBOX","enqueue","enqueue-before-change","fenced-message",0,
        r#"{"seatId":"seatA","turnId":"syntheticTurn","generation":"2","body":"retained queue marker"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&queued).unwrap()).unwrap().status,V37Status::Applied);
    let modeled_change=operation("K-SESSION","compact","synthetic-c-fence","sessionA",3,
        r#"{"generation":"2"}"#);
    crate::store::session_transport::generation_change::begin(&product.connection,
        "projectA","synthetic-c-fence",&modeled_change.raw_bytes,"compact","sessionA","2",
        &process_operation,custody.ticket.opaque(),&custody.custodian_nonce,&thread,"seatA",3,cursor as i64).unwrap();
    let blocked_queue=operation("K-INBOX","enqueue","enqueue-during-change","new-fenced-message",0,
        r#"{"seatId":"seatA","turnId":"syntheticTurn","generation":"2","body":"must not reserve"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&blocked_queue).unwrap()).unwrap().status,V37Status::Denied);
    let blocked_delivery=operation("K-INBOX","deliver","deliver-during-change","fenced-message",1,r#"{"generation":"2"}"#);
    let denied_delivery=h::decode_receipt(&product.dispatch_user_request(&blocked_delivery).unwrap()).unwrap();
    assert_eq!(denied_delivery.status,V37Status::Conflict,"original receipt: {}",String::from_utf8_lossy(&denied_delivery.raw_bytes));
    assert!(Json::Object(denied_delivery.into_result()).canonical().contains("GENERATION_CHANGE_IN_PROGRESS"));
    let unchanged=Statement::prepare(product.connection.as_ptr(),
        "SELECT revision,state FROM main.gogoke_v37_inbox_messages WHERE message_id='fenced-message'").unwrap();
    assert!(unchanged.step_row().unwrap());assert_eq!(unchanged.column_text(0).unwrap(),"1");
    assert_eq!(unchanged.column_text(1).unwrap(),"PENDING");drop(unchanged);
    append(&mut product,&mut cursor,question(1000).as_bytes());
    product.native_sessions.get_mut(&key).unwrap().raw_capture.install_sql_fixture_cursor(cursor);
    product.process_native_pending_output(&key).unwrap();
    let captured=Statement::prepare(product.connection.as_ptr(),
        "SELECT card_id,revision,state FROM main.gogoke_v37_qcard_native WHERE domain_id='projectA' AND vendor_request_id='1000'").unwrap();
    assert!(captured.step_row().unwrap(),"pending generation change retains captured original question");
    let fenced_card=captured.column_text(0).unwrap();
    let fenced_revision=captured.column_text(1).unwrap().parse::<u64>().unwrap();
    assert_eq!(captured.column_text(2).unwrap(),"OPEN");drop(captured);
    let recover_fenced=operation("K-QCARD","recover","recover-generation-fenced",&fenced_card,fenced_revision,"{}");
    let recovery=h::decode_receipt(&product.dispatch_user_request(&recover_fenced).unwrap()).unwrap();
    assert_eq!(recovery.status,V37Status::Applied,"original receipt: {}",String::from_utf8_lossy(&recovery.raw_bytes));
    let result=Json::Object(recovery.into_result()).canonical();
    assert!(result.contains("\"availableForAnswer\":false"));
    assert!(result.contains("GENERATION_CHANGE_IN_PROGRESS"));
    let new_answer=operation("K-QCARD","answer","answer-generation-fenced",&fenced_card,fenced_revision,
        r#"{"generation":"2","answers":{"q":["must not write"]}}"#);
    assert!(product.dispatch_user_request(&new_answer).is_err(),"compound intent cannot grant a new native answer writer");
    let writer=Statement::prepare(product.connection.as_ptr(),
        "SELECT count(*) FROM main.gogoke_v37_qcard_native_operations WHERE request_id='answer-generation-fenced'").unwrap();
    assert!(writer.step_row().unwrap());assert_eq!(writer.column_text(0).unwrap(),"0");drop(writer);
    // Remove only the modeled test intent, not any production transition.
    product.connection.execute("DELETE FROM gogoke_v37_h_generation_change WHERE request_id='synthetic-c-fence'").unwrap();
    let recover_ready=operation("K-QCARD","recover","recover-generation-ready",&fenced_card,fenced_revision,"{}");
    let ready=h::decode_receipt(&product.dispatch_user_request(&recover_ready).unwrap()).unwrap();
    assert!(Json::Object(ready.into_result()).canonical().contains("\"availableForAnswer\":true"),
        "control: the same captured question is answer-eligible without the modeled compound intent");
    append(&mut product,&mut cursor,question(1001).as_bytes());
    append(&mut product,&mut cursor,resolved.as_bytes());
    product.native_sessions.get_mut(&key).unwrap().raw_capture.install_sql_fixture_cursor(cursor);
    product.process_native_pending_output(&key).unwrap();
    product.native_sessions.get_mut(&key).unwrap().turn_id=Some("syntheticTurn".into());
    product.process_native_pending_output(&key).unwrap();
    let closed=Statement::prepare(product.connection.as_ptr(),"SELECT card_id,state FROM main.gogoke_v37_qcard_native WHERE domain_id='projectA' AND vendor_request_id='1001'").unwrap();
    assert!(closed.step_row().unwrap());let closed_id=closed.column_text(0).unwrap();
    assert_eq!(closed.column_text(1).unwrap(),"EXPIRED","an earlier durable resolved cannot be lost by delayed projection");drop(closed);
    let answer_closed=operation("K-QCARD","answer","answer-closed",&closed_id,2,r#"{"generation":"2","answers":{"q":["ignored"]}}"#);
    assert!(product.dispatch_user_request(&answer_closed).is_err(),"known closure rejects a new answer before native writer");
    append(&mut product,&mut cursor,question(1002).as_bytes());append(&mut product,&mut cursor,tail.as_bytes());
    let run=product.native_sessions.get_mut(&key).unwrap();run.raw_capture.install_sql_fixture_cursor(cursor);
    run.raw_capture.model_fixture_source_error(Some("MODELED_EOF_CONTROL_NOT_A_PROVIDER_OBSERVATION".into()));
    product.process_native_pending_output(&key).unwrap();
    let tail_row=Statement::prepare(product.connection.as_ptr(),"SELECT state FROM main.v37_ledger_raw_source WHERE operation_id=?1 AND source_epoch=?2 AND source_cursor=?3").unwrap();
    tail_row.bind_text(1,&process_operation).unwrap();tail_row.bind_text(2,&custody.custodian_nonce).unwrap();tail_row.bind_text(3,&cursor.to_string()).unwrap();
    assert!(tail_row.step_row().unwrap());assert_eq!(tail_row.column_text(0).unwrap(),"RESOLVED","fenced input cannot block captured question plus normalized EOF tail");drop(tail_row);
    let run=product.native_sessions.get_mut(&key).unwrap();assert!(!run.allows_input());
    // Restore only the modeled control; the real CLI pipe never closed.
    run.raw_capture.model_fixture_source_error(None);
    // Leave a new synthetic A question/tail pending until the real native
    // stop below. It must be projected while stopped custody is retained.
    append(&mut product,&mut cursor,question(1003).as_bytes());append(&mut product,&mut cursor,tail.as_bytes());
    product.native_sessions.get_mut(&key).unwrap().raw_capture.install_sql_fixture_cursor(cursor);
    let stopped_tail_cursor=cursor;
    let reused_open = operation("K-SESSION", "send", "open-session", "sessionA", 2,
        r#"{"generation":"2","body":"must never reach the provider"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&reused_open).unwrap()).unwrap().status,
        V37Status::Conflict);
    let reserved_send = operation("K-SESSION", "send", "reserved-send", "sessionA", 3,
        r#"{"generation":"2","body":"intention only"}"#);
    let reserved_input = h::StdinRequest { domain_id: "projectA", session_id: "sessionA",
        ticket: custody.ticket.opaque(), generation: "2", request_bytes: &reserved_send.raw_bytes };
    h::prepare_codex_request(&mut product.connection, &reserved_input).unwrap();
    let reused_send = operation("K-SESSION", "stop", "reserved-send", "sessionA", 3,
        r#"{"seatId":"seatA","generation":"2"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&reused_send).unwrap()).unwrap().status,
        V37Status::Conflict);
    assert!(product.native_sessions.contains_key(&key), "conflict does not stop the actual child");
    let cwd=product.native_sessions.get(&key).unwrap().evidence.cwd().to_string_lossy().into_owned();
    product.connection.execute("CREATE TRIGGER inject_pending_capture_failure BEFORE INSERT ON v37_ledger_raw_source BEGIN SELECT RAISE(ABORT,'injected retained provider capture failure'); END").unwrap();
    assert!(product.native_rpc(&key,"capture-fault-config",Some(9),&Command::ConfigRead {cwd}).is_err());
    assert!(product.native_sessions.get(&key).unwrap().raw_capture.has_pending(),
        "the actual fourth CLI response remains in original native custody");
    let stop = operation("K-SESSION", "stop", "stop-session", "sessionA", 3, r#"{"seatId":"seatA","generation":"2"}"#);
    assert!(product.dispatch_user_request(&stop).is_err(),"original capture failure still reported after actual stop");
    let retained=product.native_sessions.get(&key).unwrap();
    let proof=retained.stop_proof.as_ref().expect("stop reached the actual Job despite capture SQL failure");
    assert!(proof.parent_exited && proof.active_job_processes==Some(0) && proof.writer_fence_verified);
    assert!(proof.errors.is_empty(),"original native stop evidence: {proof:?}");
    assert!(retained.raw_capture.has_pending(),"stop cannot drop the original uncaptured response");
    assert!(product.process_custodian.active(&custody.ticket).is_some(),"proof and guards retained until durable confirmation");
    product.connection.execute("DROP TRIGGER inject_pending_capture_failure").unwrap();
    // Use the production uncertain-custody transition while the same
    // original stopped object/frame remains held. This is not a new grant.
    authority::mark_process_unknown(&mut product.connection,&process_operation,&custody).unwrap();
    // Inject a same-store write failure after the real native stop and its
    // custody proof. The original request must finish without a second stop.
    product.connection.execute("CREATE TRIGGER inject_h_stop_failure BEFORE UPDATE ON gogoke_v37_h_claim WHEN NEW.state='STOPPED' BEGIN SELECT RAISE(ABORT,'injected H stop receipt failure'); END").unwrap();
    assert!(product.dispatch_user_request(&stop).is_err());
    assert!(product.native_sessions.get(&key).unwrap().stop_proof.is_some());
    assert!(!product.native_sessions.get(&key).unwrap().raw_capture.has_pending(),"same original response captured after SQL fault removed");
    let after_stop=Statement::prepare(product.connection.as_ptr(),"SELECT state FROM main.gogoke_v37_qcard_native WHERE domain_id='projectA' AND vendor_request_id='1003'").unwrap();
    assert!(after_stop.step_row().unwrap(),"captured question projected after actual OS stop, before custody release");drop(after_stop);
    let after_stop=Statement::prepare(product.connection.as_ptr(),"SELECT state FROM main.v37_ledger_raw_source WHERE operation_id=?1 AND source_epoch=?2 AND source_cursor=?3").unwrap();
    after_stop.bind_text(1,&process_operation).unwrap();after_stop.bind_text(2,&custody.custodian_nonce).unwrap();after_stop.bind_text(3,&stopped_tail_cursor.to_string()).unwrap();
    assert!(after_stop.step_row().unwrap());assert_eq!(after_stop.column_text(0).unwrap(),"RESOLVED","real stop must not orphan the modeled A tail");drop(after_stop);
    product.connection.execute("DROP TRIGGER inject_h_stop_failure").unwrap();
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Applied);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Replayed);
    let fact = runtime::observe_stop_fact(&product.connection, "projectA", "sessionA").unwrap();
    assert!(fact.is_some(), "actual native Job and same-store durable stop fact");
    // Model a lost H commit across restart, retaining the actually persisted
    // native STOPPED proof. Recovery must not require live OS handles.
    product.connection.execute("UPDATE gogoke_v37_h_operation SET status='UNKNOWN',revision=3 WHERE request_id='stop-session'").unwrap();
    product.connection.execute("UPDATE gogoke_v37_h_claim SET state='COMMITTED',revision=3,stop_fact_id=NULL WHERE session_id='sessionA'").unwrap();
    let old_episode=Statement::prepare(product.connection.as_ptr(),
        "UPDATE gogoke_v37_h_process_episode SET phase='ACTIVE',stop_fact_id=NULL
          WHERE process_operation_id=?1 AND phase='STOPPED' AND stop_request_id='stop-session'").unwrap();
    old_episode.bind_text(1,&process_operation).unwrap();old_episode.step_done().unwrap();drop(old_episode);
    product.close_checked().unwrap();
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&open).unwrap()).unwrap().status, V37Status::Replayed);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Replayed);
    // No model turn is sent: the real pinned app-server must reopen the same
    // durable thread in a new physical process from the held H admission.
    let resume=operation("K-SESSION","resume","resume-session","sessionA",4,
        r#"{"generation":"2"}"#);
    let continued=h::decode_receipt(&product.dispatch_user_request(&resume).expect("real native thread/resume")).unwrap();
    assert_eq!(continued.status,V37Status::Applied,"original receipt: {}",String::from_utf8_lossy(&continued.raw_bytes));
    assert_eq!(continued.previous_revision,4);
    assert_eq!(continued.revision,5);
    let continued_result=continued.into_result();
    assert_eq!(continued_result.get(&JsonString::from_str("oldGeneration")).map(Json::canonical),Some(Json::String(JsonString::from_str("2")).canonical()));
    assert_eq!(continued_result.get(&JsonString::from_str("newGeneration")).map(Json::canonical),Some(Json::String(JsonString::from_str("3")).canonical()));
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&resume).unwrap()).unwrap().status,
        V37Status::Replayed,"original request replay cannot start a second process");
    let admission=Statement::prepare(product.connection.as_ptr(),
        "SELECT count(*) FROM main.gogoke_v37_h_claim WHERE domain_id='projectA' AND session_id='sessionA' AND state='COMMITTED' AND generation='3'").unwrap();
    assert!(admission.step_row().unwrap());
    assert_eq!(admission.column_text(0).unwrap(),"1","resume keeps the same admission capacity claim");
    drop(admission);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status,
        V37Status::Replayed,"old stop proof stays reachable after claim advances");
    assert_eq!(h::rpc_journal::observed_thread_id(&product.connection,"projectA","sessionA",
        &process_operation,"2","open-session",custody.ticket.opaque(),
        &custody.custodian_nonce).unwrap(),thread,
        "old H RPC and A source still identify the original thread after resume");
    let old_send=operation("K-SESSION","send","old-generation-send","sessionA",5,
        r#"{"generation":"2","body":"must not reach either process"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&old_send).unwrap()).unwrap().status,
        V37Status::Conflict);
    let old_output=operation("K-SESSION","output-stream","old-generation-after-resume","sessionA",5,
        r#"{"generation":"2","afterCursor":"0"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&old_output).unwrap()).unwrap().status,
        V37Status::Conflict,"old generation cannot read the new native process");
    let new_output=operation("K-SESSION","output-stream","new-generation-after-resume","sessionA",5,
        r#"{"generation":"3","afterCursor":"0"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&new_output).unwrap()).unwrap().status,
        V37Status::Applied);
    let resumed_flags=operation("K-SESSION","capability-probe","loaded-features-resumed","sessionA",5,
        r#"{"generation":"3"}"#);
    let observed_flags=h::decode_receipt(&product.dispatch_user_request(&resumed_flags).unwrap()).unwrap();
    assert_eq!(observed_flags.status,V37Status::Applied,"original receipt: {}",String::from_utf8_lossy(&observed_flags.raw_bytes));
    let observed_flags=observed_flags.into_result();
    let Some(Json::Object(features))=observed_flags.get(&JsonString::from_str("loadedThreadFeatures")) else {
        panic!("resumed loaded thread feature response absent");
    };
    assert!(matches!(features.get(&JsonString::from_str("memories")),Some(Json::Bool(false))));
    assert!(matches!(features.get(&JsonString::from_str("multi_agent_v2")),Some(Json::Bool(false))));
    assert!(matches!(features.get(&JsonString::from_str("default_mode_request_user_input")),Some(Json::Bool(true))));
    let second_stop=operation("K-SESSION","stop","stop-resumed","sessionA",5,
        r#"{"seatId":"seatA","generation":"3"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&second_stop).unwrap()).unwrap().status,
        V37Status::Applied);
    // Lose only H's OBSERVED commit after A captured the actual second
    // thread/resume response. The original source remains PENDING and its
    // native RPC step WRITTEN; same-ID recovery must not write stdin again.
    product.connection.execute("CREATE TRIGGER inject_resume_rpc_commit_failure BEFORE UPDATE ON gogoke_v37_rpc_steps WHEN NEW.phase='OBSERVED' AND NEW.step_id LIKE 'h-resume-%-thread-resume' BEGIN SELECT RAISE(ABORT,'injected resume RPC observation failure'); END").unwrap();
    let resumed_again=operation("K-SESSION","resume","resume-after-stop","sessionA",6,
        r#"{"generation":"3"}"#);
    let uncertain=h::decode_receipt(&product.dispatch_user_request(&resumed_again).unwrap()).unwrap();
    assert_eq!(uncertain.status,V37Status::Unknown,"original receipt: {}",String::from_utf8_lossy(&uncertain.raw_bytes));
    assert_eq!(uncertain.revision,7);
    let still_stopped=Statement::prepare(product.connection.as_ptr(),
        "SELECT generation,state,revision FROM main.gogoke_v37_h_claim WHERE domain_id='projectA' AND session_id='sessionA'").unwrap();
    assert!(still_stopped.step_row().unwrap());
    assert_eq!(still_stopped.column_text(0).unwrap(),"3");
    assert_eq!(still_stopped.column_text(1).unwrap(),"STOPPED");
    assert_eq!(still_stopped.column_text(2).unwrap(),"7");
    drop(still_stopped);
    let competing=operation("K-SESSION","resume","competing-resume","sessionA",7,
        r#"{"generation":"3"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&competing).unwrap()).unwrap().status,
        V37Status::Conflict,"candidate fences every new ID before OS start");
    product.connection.execute("DROP TRIGGER inject_resume_rpc_commit_failure").unwrap();
    let reconciled=h::decode_receipt(&product.dispatch_user_request(&resumed_again).unwrap()).unwrap();
    assert_eq!(reconciled.status,V37Status::Replayed,"original receipt: {}",String::from_utf8_lossy(&reconciled.raw_bytes));
    assert_eq!(reconciled.previous_revision,7);
    assert_eq!(reconciled.revision,8);
    // Actual thread/inject_items through the User product entry; the native
    // empty ACK proves injection submission and does not start a model turn.
    let append=operation("K-SESSION","append-without-turn","append-reconciled","sessionA",8,
        r#"{"generation":"4","body":"cloud no-model append marker"}"#);
    product.connection.execute("CREATE TRIGGER inject_append_receipt_failure BEFORE UPDATE ON gogoke_v37_h_stdin_journal WHEN NEW.phase='RECEIPTED' AND NEW.operation='append-without-turn' BEGIN SELECT RAISE(ABORT,'injected append receipt failure'); END").unwrap();
    assert!(product.dispatch_user_request(&append).is_err(),"actual native ACK stays in original A/RPC when the final H receipt fails");
    product.connection.execute("DROP TRIGGER inject_append_receipt_failure").unwrap();
    let appended=h::decode_receipt(&product.dispatch_user_request(&append).unwrap()).unwrap();
    assert_eq!(appended.status,V37Status::Replayed,"original receipt: {}",String::from_utf8_lossy(&appended.raw_bytes));
    assert_eq!(appended.revision,9);
    let stale_append=operation("K-SESSION","append-without-turn","append-stale","sessionA",8,
        r#"{"generation":"4","body":"stale request must not be injected"}"#);
    let stale=h::decode_receipt(&product.dispatch_user_request(&stale_append).unwrap()).unwrap();
    assert_eq!(stale.status,V37Status::Stale,"original receipt: {}",String::from_utf8_lossy(&stale.raw_bytes));assert_eq!(stale.previous_revision,9);assert_eq!(stale.revision,9);
    let original_append_result=Json::Object(appended.into_result()).canonical();
    assert!(original_append_result.contains("\"createdTurn\":false"));
    // This local state assertion verifies the host's state machine only.
    // Native absence of a created turn is not proved by a generated receipt
    // or a cached field; authenticated model/history checks remain NOT_RUN.
    assert!(product.native_sessions.get(&key).unwrap().turn_id.is_none());
    let first_step_count={let row=Statement::prepare(product.connection.as_ptr(),"SELECT count(*) FROM gogoke_v37_rpc_steps WHERE step_id LIKE 'append-%'").unwrap();
        assert!(row.step_row().unwrap());row.column_text(0).unwrap()};
    let replay=h::decode_receipt(&product.dispatch_user_request(&append).unwrap()).unwrap();
    assert_eq!(replay.status,V37Status::Replayed,"original receipt: {}",String::from_utf8_lossy(&replay.raw_bytes));
    assert_eq!(Json::Object(replay.into_result()).canonical(),original_append_result);
    let step_count=Statement::prepare(product.connection.as_ptr(),"SELECT count(*) FROM gogoke_v37_rpc_steps WHERE step_id LIKE 'append-%'").unwrap();
    assert!(step_count.step_row().unwrap());assert_eq!(step_count.column_text(0).unwrap(),first_step_count);drop(step_count);
    let changed=operation("K-SESSION","append-without-turn","append-reconciled","sessionA",8,
        r#"{"generation":"4","body":"changed input must not be injected"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&changed).unwrap()).unwrap().status,V37Status::Conflict);
    // Actual no-model renewal: the original request internally stops the old
    // Job, retains admission, and resumes the same provider thread in a new
    // process. No model turn or manual compaction is claimed here.
    let renew=operation("K-SESSION","renew-session","renew-after-append","sessionA",9,
        r#"{"generation":"4"}"#);
    let renewed=h::decode_receipt(&product.dispatch_user_request(&renew).unwrap()).unwrap();
    assert_eq!(renewed.status,V37Status::Applied,"original receipt: {}",String::from_utf8_lossy(&renewed.raw_bytes));
    assert_eq!(renewed.previous_revision,9);assert_eq!(renewed.revision,10);
    let renewed_result=Json::Object(renewed.into_result()).canonical();
    assert!(renewed_result.contains("\"oldGeneration\":\"4\""));
    assert!(renewed_result.contains("\"newGeneration\":\"5\""));
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&renew).unwrap()).unwrap().status,
        V37Status::Replayed);
    let old_append=operation("K-SESSION","append-without-turn","append-old-gen","sessionA",10,
        r#"{"generation":"4","body":"must not reach old physical process"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&old_append).unwrap()).unwrap().status,
        V37Status::Conflict);
    let current_caps=operation("K-SESSION","capability-probe","cap-renewed","sessionA",10,
        r#"{"generation":"5"}"#);
    let caps=h::decode_receipt(&product.dispatch_user_request(&current_caps).unwrap()).unwrap();
    assert_eq!(caps.status,V37Status::Applied,"original receipt: {}",String::from_utf8_lossy(&caps.raw_bytes));
    let cap_result=Json::Object(caps.into_result()).canonical();
    assert!(cap_result.contains("\"generation\":\"5\""));
    // Lose the H internal-stop commit after the actual old Job proof. The
    // original renew ID alone settles it; no second OS stop or start occurs.
    product.connection.execute("CREATE TRIGGER fail_renew_old_h_stop BEFORE UPDATE ON gogoke_v37_h_claim WHEN NEW.state='STOPPED' BEGIN SELECT RAISE(ABORT,'injected compound H stop failure'); END").unwrap();
    let renew_uncertain=operation("K-SESSION","renew-session","renew-after-fault","sessionA",10,
        r#"{"generation":"5"}"#);
    let unknown=h::decode_receipt(&product.dispatch_user_request(&renew_uncertain).unwrap()).unwrap();
    assert_eq!(unknown.status,V37Status::Unknown,"original receipt: {}",String::from_utf8_lossy(&unknown.raw_bytes));
    assert_eq!(unknown.previous_revision,10);assert_eq!(unknown.revision,11);
    let competing=operation("K-SESSION","renew-session","renew-competing","sessionA",11,
        r#"{"generation":"5"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&competing).unwrap()).unwrap().status,
        V37Status::Conflict);
    let unresolved_reconnect=operation("K-SESSION","reconnect","reconnect-unknown","sessionA",11,
        r#"{"generation":"5"}"#);
    let read_only=h::decode_receipt(&product.dispatch_user_request(&unresolved_reconnect).unwrap()).unwrap();
    assert_eq!(read_only.status,V37Status::Unknown,"original receipt: {}",String::from_utf8_lossy(&read_only.raw_bytes));
    assert_eq!(read_only.previous_revision,11);assert_eq!(read_only.revision,11);
    product.connection.execute("DROP TRIGGER fail_renew_old_h_stop").unwrap();
    let recovered=h::decode_receipt(&product.dispatch_user_request(&renew_uncertain).unwrap()).unwrap();
    assert_eq!(recovered.status,V37Status::Replayed,"original receipt: {}",String::from_utf8_lossy(&recovered.raw_bytes));
    assert_eq!(recovered.previous_revision,11);assert_eq!(recovered.revision,12);
    let recovered_result=Json::Object(recovered.into_result()).canonical();
    assert!(recovered_result.contains("\"newGeneration\":\"6\""));
    let before_reconnect=Statement::prepare(product.connection.as_ptr(),
        "SELECT count(*) FROM gogoke_v37_h_process_episode WHERE domain_id='projectA' AND session_id='sessionA' AND process_operation_id IS NOT NULL").unwrap();
    assert!(before_reconnect.step_row().unwrap());let process_count=before_reconnect.column_text(0).unwrap();
    drop(before_reconnect);
    let reconnect=operation("K-SESSION","reconnect","reconnect-renewed","sessionA",12,
        r#"{"generation":"6"}"#);
    let reconnected=h::decode_receipt(&product.dispatch_user_request(&reconnect).unwrap()).unwrap();
    assert_eq!(reconnected.status,V37Status::Applied,"original receipt: {}",String::from_utf8_lossy(&reconnected.raw_bytes));
    assert_eq!(reconnected.previous_revision,12);assert_eq!(reconnected.revision,13);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&reconnect).unwrap()).unwrap().status,
        V37Status::Replayed);
    let after_reconnect=Statement::prepare(product.connection.as_ptr(),
        "SELECT count(*) FROM gogoke_v37_h_process_episode WHERE domain_id='projectA' AND session_id='sessionA' AND process_operation_id IS NOT NULL").unwrap();
    assert!(after_reconnect.step_row().unwrap());assert_eq!(after_reconnect.column_text(0).unwrap(),process_count);
    drop(after_reconnect);
    product.connection.execute("CREATE TRIGGER fail_owner_renew_stop BEFORE UPDATE ON gogoke_v37_h_claim WHEN NEW.state='STOPPED' BEGIN SELECT RAISE(ABORT,'injected owner-stop compound window'); END").unwrap();
    let owner_interrupted_renew=operation("K-SESSION","renew-session","renew-owner-interrupted","sessionA",13,
        r#"{"generation":"6"}"#);
    let owner_pending=h::decode_receipt(&product.dispatch_user_request(&owner_interrupted_renew).unwrap()).unwrap();
    assert_eq!(owner_pending.status,V37Status::Unknown,"original receipt: {}",String::from_utf8_lossy(&owner_pending.raw_bytes));
    assert_eq!(owner_pending.previous_revision,13);assert_eq!(owner_pending.revision,14);
    product.connection.execute("DROP TRIGGER fail_owner_renew_stop").unwrap();
    let third_stop=operation("K-SESSION","stop","stop-reconciled","sessionA",14,
        r#"{"seatId":"seatA","generation":"6"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&third_stop).unwrap()).unwrap().status,
        V37Status::Applied);
    let stopped_change=h::decode_receipt(&product.dispatch_user_request(&owner_interrupted_renew).unwrap()).unwrap();
    assert_eq!(stopped_change.status,V37Status::Unknown,"Owner stop fences the original compound writer");
    let release = operation("K-SESSION", "admission-release", "release-session", "sessionA", 15,
        r#"{"seatId":"seatA","generation":"6"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&release).unwrap()).unwrap().status, V37Status::Applied);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Replayed);
    product.close_checked().unwrap();
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    let replay=h::decode_receipt(&product.dispatch_user_request(&append).unwrap()).unwrap();
    assert_eq!(replay.status,V37Status::Replayed,"stopped/released historical original append needs no new process");
    assert_eq!(Json::Object(replay.into_result()).canonical(),original_append_result);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Replayed);
    product.close_checked().unwrap();
    drop(root);
    std::fs::remove_dir_all(path).unwrap();
}
