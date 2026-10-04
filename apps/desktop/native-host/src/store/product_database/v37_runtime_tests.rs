use super::*;
use crate::root::RootLock;
use crate::store::seat::{CreateSeat, Kind, StoreTemplate};
use crate::store::same_open::route_b_test_guard;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::process::AppContainerProfile;

// Reuses the actual fixed-catalog E/F/H launch from the session control below.
// Login presence alone is synthetic. No credentials or model are loaded.
fn health_control_product(driver:&str, run:impl FnOnce(&mut ProductDatabase<'_>)) {
    let _guard=route_b_test_guard();
    let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path=std::env::temp_dir().join(format!("gogoke-v37-health-control-{}-{stamp}",std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root=RootLock::acquire(&path).unwrap();
    let mut product=ProductDatabase::open(&root,&path.join("state.sqlite")).unwrap();
    let register=operation("K-INSTANCE","register","health-register","instanceA",0,
        &format!(r#"{{"driverId":"{driver}"}}"#));
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&register).unwrap()).unwrap().status,V37Status::Applied);
    instance::record_observation(&mut product.connection,&root,&instance::ObservationRequest {
        request_id:"health-login-presence",request_bytes:b"SYNTHETIC_LOGIN_PRESENCE_NOT_AUTHENTICATION",
        instance_id:"instanceA",expected_revision:1,observation:instance::InstanceObservation::LoggedIn,
    }).unwrap();
    instance::set_instance_concurrency_cap(&mut product.connection,&product.owner,"instanceA",1).unwrap();
    seat::set_project_parallel_cap(&mut product.connection,&product.owner,"projectA",1).unwrap();
    let model=if driver=="claude" {"claude-sonnet-4-6"} else {"gpt-6-sol"};
    seat::store_template(&mut product.connection,NativeOrigin::user(&product.owner),StoreTemplate {
        domain_id:"projectA",template_id:"templateA",settings_json:format!(
            r#"{{"effort":"high","model":"{model}","permissionTier":"NETWORKED_WRITE"}}"#).as_bytes(),
    }).unwrap();
    seat::create(&mut product.connection,NativeOrigin::user(&product.owner),CreateSeat {
        domain_id:"projectA",seat_id:"seatA",template_id:"templateA",instance_id:Some("instanceA"),
        kind:Kind::Long,request_id:"health-create-seat",request_bytes:b"actual health control seat",
    }).unwrap();
    let source=crate::store::worktree::tests::make_source_fixture(&mut product.connection,&root,
        &product.owner,&mut product.process_custodian);
    let git=std::env::var_os("GOGOKE_CONTROLLED_GIT_PATH").unwrap();
    let configuration=Json::Object(BTreeMap::from([
        (JsonString::from_str("schema"),Json::String(JsonString::from_str("gogoke.37.owner-configuration.v1"))),
        (JsonString::from_str("command"),Json::String(JsonString::from_str("worktree-source"))),
        (JsonString::from_str("repositoryId"),Json::String(JsonString::from_str("fixtureRepo"))),
        (JsonString::from_str("sourcePath"),Json::String(JsonString::from_str(source.to_str().unwrap()))),
        (JsonString::from_str("gitPath"),Json::String(JsonString::from_str(git.to_str().unwrap()))),
    ])).canonical();
    product.configure_user_v37(configuration.as_bytes()).unwrap();
    let create=operation("K-WORKTREE","create","health-create-tree","treeA",0,
        r#"{"repositoryId":"fixtureRepo","seatId":"seatA"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&create).unwrap()).unwrap().status,V37Status::Applied);
    for (verb,id,rev) in [("admission-reserve","health-reserve",0),("admission-commit","health-commit",1)] {
        let request=operation("K-SESSION",verb,id,"sessionA",rev,r#"{"seatId":"seatA","generation":"2"}"#);
        assert_eq!(h::decode_receipt(&product.dispatch_user_request(&request).unwrap()).unwrap().status,V37Status::Applied);
    }
    let open=operation("K-SESSION","open","health-open","sessionA",2,
        r#"{"seatId":"seatA","generation":"2","repositoryId":"fixtureRepo","worktreeId":"treeA"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&open).unwrap()).unwrap().status,V37Status::Applied);
    run(&mut product);
    let claim=Statement::prepare(product.connection.as_ptr(),
        "SELECT generation,revision FROM main.gogoke_v37_h_claim WHERE domain_id='projectA' AND session_id='sessionA'").unwrap();
    assert!(claim.step_row().unwrap());let generation=claim.column_text(0).unwrap();
    let revision=claim.column_text(1).unwrap().parse::<u64>().unwrap();drop(claim);
    let stop=operation("K-SESSION","stop","health-final-stop","sessionA",revision,
        &format!(r#"{{"seatId":"seatA","generation":"{generation}"}}"#));
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status,V37Status::Applied);
    product.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
}

fn health_control_rows(product:&ProductDatabase<'_>,sql:&str)->Vec<Vec<String>> {
    let row=Statement::prepare(product.connection.as_ptr(),sql).unwrap();let mut result=Vec::new();
    while row.step_row().unwrap() {result.push(vec![row.column_text(0).unwrap(),row.column_text(1).unwrap()]);}
    result
}

// Existing synthetic A ordering seam, under the actual opened H/C identity.
// This is not an OriginBoundFrame/pipe observation or a hand-made health proof.
fn health_control_source(product:&mut ProductDatabase<'_>,raw:&[u8])->ledger::RawSourceKey {
    let key=("projectA".to_owned(),"sessionA".to_owned());let live=product.native_sessions.get(&key).unwrap();
    let custody=live.custody.clone();let process=live.operation_id.clone();
    let maximum=Statement::prepare(product.connection.as_ptr(),
        "SELECT COALESCE(MAX(CAST(source_cursor AS INTEGER)),0) FROM main.v37_ledger_raw_source WHERE operation_id=?1").unwrap();
    maximum.bind_text(1,&process).unwrap();assert!(maximum.step_row().unwrap());
    let cursor=maximum.column_text(0).unwrap().parse::<u64>().unwrap()+1;drop(maximum);
    let insert=Statement::prepare(product.connection.as_ptr(),
        "INSERT INTO main.v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state) VALUES(?1,?2,?3,'projectA','sessionA',?4,?3,?5,?6,'PENDING')").unwrap();
    insert.bind_text(1,&process).unwrap();insert.bind_text(2,custody.ticket.opaque()).unwrap();
    insert.bind_text(3,&custody.custodian_nonce).unwrap();insert.bind_text(4,&custody.binding.generation).unwrap();
    insert.bind_text(5,&cursor.to_string()).unwrap();insert.bind_blob(6,raw).unwrap();insert.step_done().unwrap();drop(insert);
    product.native_sessions.get_mut(&key).unwrap().raw_capture.install_sql_fixture_cursor(cursor);
    ledger::RawSourceKey {operation_id:process,source_epoch:custody.custodian_nonce,source_cursor:cursor.to_string()}
}

// Modelled ordinary-send writer/ACK ordering; original receipt is generated
// by H's production recovery. No model prompt is written to the actual pipe.
fn health_control_work_turn(product:&mut ProductDatabase<'_>,early_terminal:bool,cause:&str)->ledger::RawSourceKey {
    use crate::store::session_transport::{codex_rpc::{Command,RpcId},rpc_journal as rpc};
    let key=("projectA".to_owned(),"sessionA".to_owned());let live=product.native_sessions.get(&key).unwrap();
    let custody=live.custody.clone();let open_id=live.open_request_id.clone();let open_bytes=live.open_request_bytes.clone();
    let original_rpc_ordinal=live.next_rpc_id;
    let thread=live.thread_id.clone().unwrap();let body="synthetic work boundary; never written to provider";
    let command=Command::TurnStart {thread_id:thread.clone(),cwd:live.evidence.cwd().to_string_lossy().into_owned(),
        model:"gpt-6-sol".into(),effort:"high".into(),text:body.into(),network_access:Some(live.evidence.network_access())};
    let request=operation("K-SESSION","send","health-work-send","sessionA",3,
        &format!(r#"{{"generation":"2","body":"{body}"}}"#));
    let input=h::StdinRequest {domain_id:"projectA",session_id:"sessionA",ticket:custody.ticket.opaque(),
        generation:"2",request_bytes:&request.raw_bytes};
    assert_eq!(h::prepare_codex_request(&mut product.connection,&input).unwrap().disposition,h::PrepareDisposition::Prepared);
    let step_id=format!("send-{}",&crate::store::digest::sha256_hex(&request.raw_bytes)[..40]);
    let rpc_id=RpcId::Number(88001);
    let step=rpc::Step {domain_id:"projectA",session_id:"sessionA",open_request_id:&open_id,
        open_request_bytes:&open_bytes,step_id:&step_id,custody:&custody,rpc_id:Some(&rpc_id),command:&command};
    assert_eq!(rpc::prepare(&mut product.connection,&product.owner,&step).unwrap().disposition,rpc::Disposition::NewWrite);
    rpc::mark_written(&mut product.connection,&product.owner,&step).unwrap();
    let terminal=format!("{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":{},\"turn\":{{\"id\":\"healthWorkTurn\",\"status\":\"failed\",\"error\":{{\"message\":\"synthetic typed failure control\",\"codexErrorInfo\":{cause}}}}}}}}}\n",
        Json::String(JsonString::from_str(&thread)).canonical());
    let mut first=None;
    if early_terminal {
        first=Some(health_control_source(product,terminal.as_bytes()));
        product.process_native_pending_output(&key).unwrap();
        assert!(health_control_rows(product,"SELECT event_id,state FROM main.gogoke_v37_seat_health").is_empty(),
            "terminal without original ordinary receipt cannot yet mint");
    }
    let ack=health_control_source(product,b"{\"id\":88001,\"result\":{\"turn\":{\"id\":\"healthWorkTurn\",\"status\":\"inProgress\"}}}\n");
    // Only the synthetic A/H association is modelled. Normal recover below
    // decodes/rechecks command, ACK, current H seat and produces original bytes.
    let observed=Statement::prepare(product.connection.as_ptr(),
        "UPDATE main.gogoke_v37_rpc_steps SET phase='OBSERVED',source_epoch=?1,source_cursor=?2 WHERE step_id=?3 AND phase='WRITTEN'").unwrap();
    observed.bind_text(1,&ack.source_epoch).unwrap();observed.bind_text(2,&ack.source_cursor).unwrap();
    observed.bind_text(3,&step_id).unwrap();observed.step_done().unwrap();drop(observed);
    ledger::resolve_raw_source_no_event(&mut product.connection,&ack.operation_id,&ack.source_epoch,&ack.source_cursor,"CODEX_RPC_RESPONSE").unwrap();
    // Mirror the existing real Reply::Turn retention point, including late ACK.
    product.native_sessions.get_mut(&key).unwrap().turn_id=Some("healthWorkTurn".into());
    if early_terminal {
        product.process_native_pending_output(&key).unwrap();
        assert!(product.native_sessions.get(&key).unwrap().turn_id.is_none(),
            "original resolved terminal must clear the turn resurrected by its late ACK");
    }
    let receipt=h::recover_codex_turn_request(&mut product.connection,&input).unwrap().unwrap();
    assert_eq!(h::decode_receipt(&receipt.record.receipt_bytes.unwrap()).unwrap().status,V37Status::Applied);
    if !early_terminal {first=Some(health_control_source(product,terminal.as_bytes()));product.process_native_pending_output(&key).unwrap();}
    assert!(product.native_sessions.get(&key).unwrap().turn_id.is_none());
    product.revisit_host_health_sources().unwrap();
    let original=first.unwrap();
    let observation=health_control_rows(product,"SELECT source_event_id,state FROM main.gogoke_v37_seat_health");
    assert_eq!(observation.len(),1,"exactly one production sealed health observation");assert_eq!(observation[0][1],"OBSERVED");
    let raw=ledger::read_captured_raw_source(&product.connection,&original.operation_id,&original.source_epoch,&original.source_cursor).unwrap().unwrap();
    assert_eq!(raw.state,ledger::RawSourceState::Resolved);assert_eq!(Some(&observation[0][0]),raw.resolved_event_id.as_ref());
    let before=health_control_rows(product,"SELECT command_hex,phase FROM main.gogoke_v37_rpc_steps WHERE step_id LIKE 'send-%'");
    health_control_source(product,terminal.as_bytes());product.process_native_pending_output(&key).unwrap();
    product.revisit_host_health_sources().unwrap();product.revisit_host_health_sources().unwrap();
    assert_eq!(health_control_rows(product,"SELECT source_event_id,state FROM main.gogoke_v37_seat_health"),observation,
        "duplicate source and safe-point revisit must not mint a second observation or overwrite original source");
    assert_eq!(health_control_rows(product,"SELECT command_hex,phase FROM main.gogoke_v37_rpc_steps WHERE step_id LIKE 'send-%'"),before,
        "health observation cannot allocate or repeat work input");
    assert_eq!(product.native_sessions.get(&key).unwrap().next_rpc_id,original_rpc_ordinal,
        "sealed observation/revisit cannot allocate a native writer ID");
    original
}

#[test]
fn health_terminal_and_ordinary_ack_orders_keep_one_original_seal_and_no_work_resend() {
    for early in [false,true] {health_control_product("codex",|product| {
        health_control_work_turn(product,early,r#"{"responseTooManyFailedAttempts":{"httpStatusCode":null}}"#);
        assert!(health_control_rows(product,"SELECT request_id,stage FROM main.gogoke_v37_h_generation_change").is_empty(),
            "observation/revisit alone cannot acquire a generation action permit");
    });}
}

#[test]
fn health_compact_late_original_ack_continues_without_second_write_or_new_request() {
    health_control_product("codex",|product| {
        let key=("projectA".to_owned(),"sessionA".to_owned());
        let thread=product.native_sessions.get(&key).unwrap().thread_id.clone().unwrap();
        assert!(matches!(product.native_append_rpc(&key,"health-real-history",&thread,"no-model native history marker".into()).unwrap(),
            Some(crate::store::session_transport::codex_rpc::Reply::Ack {..})));
        health_control_work_turn(product,false,r#""contextWindowExceeded""#);
        // Audit the actual production mark_written immediately after its OS
        // write. The only fault is the later OBSERVED transaction, not a writer.
        product.connection.execute("CREATE TEMP TABLE health_control_writes(value INTEGER NOT NULL); INSERT INTO health_control_writes VALUES(0)").unwrap();
        product.connection.execute("CREATE TEMP TRIGGER health_count_compact_write AFTER UPDATE OF phase ON main.gogoke_v37_rpc_steps WHEN NEW.phase='WRITTEN' AND OLD.phase='INTENT' AND NEW.step_id LIKE 'compact-%' BEGIN UPDATE health_control_writes SET value=value+1; END").unwrap();
        product.connection.execute("CREATE TEMP TRIGGER health_delay_compact_ack BEFORE UPDATE OF phase ON main.gogoke_v37_rpc_steps WHEN NEW.phase='OBSERVED' AND NEW.step_id LIKE 'compact-%' BEGIN SELECT RAISE(ABORT,'health control delayed original compact observation'); END").unwrap();
        let original_error=product.pump_host_health().expect_err("original OBSERVED fault cannot be swallowed as PASS");
        assert!(format!("{original_error:?}").contains("health control delayed original compact observation"),
            "the retained error must identify the actual injected observation failure: {original_error:?}");
        product.connection.execute("DROP TRIGGER health_delay_compact_ack").unwrap();
        let change=health_control_rows(product,"SELECT raw_hex,request_id FROM main.gogoke_v37_h_generation_change");
        assert_eq!(change.len(),1);
        let raw=change[0][0].as_bytes().chunks_exact(2).map(|pair|
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(),16).unwrap()).collect::<Vec<_>>();
        let request=decode_request(&raw).unwrap();
        let step=format!("compact-{}",&crate::store::digest::sha256_hex(&raw)[..40]);
        let command_sql=format!("SELECT command_hex,phase FROM main.gogoke_v37_rpc_steps WHERE step_id='{step}'");
        let written=health_control_rows(product,&command_sql);
        assert_eq!(written.len(),1);assert_eq!(written[0][1],"WRITTEN");
        let original_ordinal=product.native_sessions.get(&key).unwrap().next_rpc_id;
        assert_eq!(health_control_rows(product,"SELECT CAST(value AS TEXT),'writes' FROM temp.health_control_writes")[0][0],"1");
        assert!(product.health_generation_may_continue(&request).unwrap(),
            "production gate must first reconcile the exact original WRITTEN/A ACK");
        let observed=health_control_rows(product,&command_sql);assert_eq!(observed[0][0],written[0][0]);assert_eq!(observed[0][1],"OBSERVED");
        assert_eq!(product.native_sessions.get(&key).unwrap().next_rpc_id,original_ordinal,
            "late ACK gate consumes original A without allocating another RPC ID");
        product.pump_host_health().unwrap();
        assert_eq!(product.native_sessions.get(&key).unwrap().next_rpc_id,original_ordinal);
        assert_eq!(health_control_rows(product,"SELECT stage,request_id FROM main.gogoke_v37_h_generation_change")[0],vec!["ACKED".to_owned(),request.request_id.clone()],
            "an ACK alone does not settle compact or stop/restart the original process");
        let retained_compact_turn=product.native_sessions.get(&key).unwrap().turn_id.clone();
        let compact_turn=retained_compact_turn.as_deref().unwrap_or("healthCompactTurn");
        // These explicit synthetic A controls must describe the retained
        // actual compact turn. An item alone cannot terminate that turn.
        let complete=format!("{{\"method\":\"item/completed\",\"params\":{{\"threadId\":{},\"turnId\":{},\"completedAtMs\":0,\"item\":{{\"type\":\"contextCompaction\",\"id\":\"healthCompactItem\"}}}}}}\n",
            Json::String(JsonString::from_str(&thread)).canonical(),
            Json::String(JsonString::from_str(compact_turn)).canonical());
        health_control_source(product,complete.as_bytes());product.process_native_pending_output(&key).unwrap();
        if let Some(turn)=retained_compact_turn {
            product.pump_host_health().unwrap();
            assert_eq!(health_control_rows(product,"SELECT stage,request_id FROM main.gogoke_v37_h_generation_change")[0],
                vec!["ITEM_OBSERVED".to_owned(),request.request_id.clone()],
                "the original active compact turn still fences generation stop");
            assert_eq!(product.native_sessions.get(&key).unwrap().turn_id.as_deref(),Some(turn.as_str()));
            let terminal=format!("{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":{},\"turn\":{{\"id\":{},\"status\":\"completed\"}}}}}}\n",
                Json::String(JsonString::from_str(&thread)).canonical(),
                Json::String(JsonString::from_str(&turn)).canonical());
            health_control_source(product,terminal.as_bytes());product.process_native_pending_output(&key).unwrap();
        }
        product.pump_host_health().unwrap();product.pump_host_health().unwrap();
        assert_eq!(health_control_rows(product,"SELECT raw_hex,request_id FROM main.gogoke_v37_h_generation_change"),change,
            "automatic continuation keeps original request identity and exact bytes");
        let final_change=health_control_rows(product,"SELECT stage,COALESCE(original_error,'') FROM main.gogoke_v37_h_generation_change");
        assert_eq!(final_change[0][0],"APPLIED",
            "original generation progress and stored native error: {final_change:?}");
        assert_eq!(health_control_rows(product,"SELECT state,session_request_id FROM main.gogoke_v37_seat_health")[0],
            vec!["RECEIPTED".to_owned(),request.request_id.clone()]);
        assert_eq!(health_control_rows(product,&command_sql),observed);
        assert_eq!(health_control_rows(product,"SELECT CAST(value AS TEXT),'writes' FROM temp.health_control_writes")[0][0],"1",
            "continuation/replay cannot repeat the actual compact writer");
        assert_eq!(health_control_rows(product,"SELECT command_hex,phase FROM main.gogoke_v37_rpc_steps WHERE step_id LIKE 'compact-%'").len(),1,
            "no new command ID is allocated");
        product.connection.execute("DROP TRIGGER health_count_compact_write; DROP TABLE temp.health_control_writes").unwrap();
    });
}

fn operation(family: &str, verb: &str, id: &str, target: &str, revision: u64, payload: &str) -> V37Request {
    let domain = if family == "K-INSTANCE" { "global" } else { "projectA" };
    decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"{family}","operation":"{verb}","requestId":"{id}","targetId":"{target}","domainId":"{domain}","expectedRevision":"{revision}","payload":{payload}}}"#).as_bytes()).unwrap()
}

#[test]
fn actual_pinned_codex_product_open_records_rpc_and_durable_stop_without_model_call() {
    actual_pinned_codex_product_open_baseline_without_model_call();
    actual_pinned_codex_history_acl_vendor_qualification_without_model_call();
}

fn actual_pinned_codex_product_open_baseline_without_model_call() {
    let _guard = route_b_test_guard();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-v37-real-session-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    let register = operation("K-INSTANCE", "register", "register-cli", "instanceA", 0,
        r#"{"driverId":"codex"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&register).unwrap()).unwrap().status,
        V37Status::Applied, "actual cloud-pinned CLI User registration");
    let install = operation("K-INSTANCE", "install-state", "read-real-install", "instanceA", 1, "{}");
    let install_receipt = h::decode_receipt(&product.dispatch_user_request(&install).unwrap()).unwrap();
    assert_eq!(install_receipt.status, V37Status::Applied);
    assert_eq!((install_receipt.previous_revision, install_receipt.revision), (1, 1),
        "real catalog install-state is observational");
    assert!(String::from_utf8_lossy(&install_receipt.raw_bytes).contains("\"installed\":true"));
    let stored = Statement::prepare(product.connection.as_ptr(),
        "SELECT install_state,revision FROM gogoke_v37_instances WHERE instance_id='instanceA'").unwrap();
    assert!(stored.step_row().unwrap());
    assert_eq!((stored.column_text(0).unwrap(), stored.column_text(1).unwrap()),
        ("UNKNOWN".into(), "1".into()), "read must not manufacture INSTALLED");
    drop(stored);
    // Only login presence is synthetic. The installed CLI fact above and the
    // H launch below are the actual fixed catalog, not a database substitute.
    instance::record_observation(&mut product.connection, &root, &instance::ObservationRequest {
        request_id: "fixture-login-admission", request_bytes: b"fixture-login-admission",
        instance_id: "instanceA", expected_revision: 1,
        observation: instance::InstanceObservation::LoggedIn,
    }).unwrap();
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
    let final_instance = Statement::prepare(product.connection.as_ptr(),
        "SELECT install_state,revision FROM gogoke_v37_instances WHERE instance_id='instanceA'").unwrap();
    assert!(final_instance.step_row().unwrap());
    assert_eq!((final_instance.column_text(0).unwrap(), final_instance.column_text(1).unwrap()),
        ("UNKNOWN".into(), "2".into()), "H open, RPC and recovery must not rewrite F's read-only install fact");
    drop(final_instance);
    product.close_checked().unwrap();
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    let replay=h::decode_receipt(&product.dispatch_user_request(&append).unwrap()).unwrap();
    assert_eq!(replay.status,V37Status::Replayed,"stopped/released historical original append needs no new process");
    assert_eq!(Json::Object(replay.into_result()).canonical(),original_append_result);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Replayed);
    // The same real fixed CLI starts a separate, zero-lineage review thread.
    // Only login presence/setup is synthetic; no model or credentials are used.
    let seat_generation=Statement::prepare(product.connection.as_ptr(),
        "SELECT generation FROM main.gogoke_v37_seats WHERE domain_id='projectA' AND seat_id='seatA'").unwrap();
    assert!(seat_generation.step_row().unwrap());
    let review_generation=seat_generation.column_text(0).unwrap().parse::<u64>().unwrap()+1;
    drop(seat_generation);
    for (verb,id,revision) in [("admission-reserve","reserve-review",0),("admission-commit","commit-review",1)] {
        let request=operation("K-SESSION",verb,id,"reviewA",revision,
            &format!(r#"{{"seatId":"seatA","generation":"{review_generation}"}}"#));
        assert_eq!(h::decode_receipt(&product.dispatch_user_request(&request).unwrap()).unwrap().status,V37Status::Applied);
    }
    let review_open=operation("K-SESSION","open","open-review","reviewA",2,
        &format!(r#"{{"seatId":"seatA","generation":"{review_generation}","repositoryId":"fixtureRepo","worktreeId":"treeA","purpose":"FORMAL_REVIEW"}}"#));
    let opened_review=h::decode_receipt(&product.dispatch_user_request(&review_open).unwrap()).unwrap();
    assert_eq!(opened_review.status,V37Status::Applied,"original review receipt: {}",String::from_utf8_lossy(&opened_review.raw_bytes));
    let review_result=opened_review.into_result();
    let Some(Json::String(review_thread))=review_result.get(&JsonString::from_str("threadId")) else {
        panic!("original native review thread absent");
    };
    assert_ne!(review_thread.to_well_formed_string().unwrap(),thread,"actual fresh review thread");
    assert_eq!(ledger::read_registered_session(&product.connection,"reviewA").unwrap().unwrap().purpose,SessionPurpose::FormalReview);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&review_open).unwrap()).unwrap().status,V37Status::Replayed);
    let before_review_custody=product.native_sessions.len();
    let review_process_count=|product:&ProductDatabase<'_>| {
        let rows=Statement::prepare(product.connection.as_ptr(),
            "SELECT count(*) FROM main.gogoke_v37_h_process_episode WHERE domain_id='projectA' AND session_id='reviewA'").unwrap();
        assert!(rows.step_row().unwrap());rows.column_text(0).unwrap()
    };
    let before_review_process_count=review_process_count(&product);
    for verb in ["resume","reconnect","compact","renew-session"] {
        let refused=operation("K-SESSION",verb,&format!("review-{verb}"),"reviewA",3,
            &format!(r#"{{"generation":"{review_generation}"}}"#));
        let receipt=h::decode_receipt(&product.dispatch_user_request(&refused).unwrap()).unwrap();
        assert_eq!(receipt.status,V37Status::Denied,"formal review continuation: {verb}");
        assert_eq!((receipt.previous_revision,receipt.revision),(3,3));
        assert_eq!(product.native_sessions.len(),before_review_custody);
        assert_eq!(review_process_count(&product),before_review_process_count);
    }
    let inherited_open=operation("K-SESSION","open","fork-into-review","reviewA",3,
        &format!(r#"{{"seatId":"seatA","generation":"{review_generation}","repositoryId":"fixtureRepo","worktreeId":"treeA","purpose":"FORMAL_REVIEW","sourceSessionId":"sessionA"}}"#));
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&inherited_open).unwrap()).unwrap().status,V37Status::Denied);
    assert_eq!(review_process_count(&product),before_review_process_count);
    let review_stop=operation("K-SESSION","stop","stop-review","reviewA",3,
        &format!(r#"{{"seatId":"seatA","generation":"{review_generation}"}}"#));
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&review_stop).unwrap()).unwrap().status,V37Status::Applied);
    let review_release=operation("K-SESSION","admission-release","release-review","reviewA",4,
        &format!(r#"{{"seatId":"seatA","generation":"{review_generation}"}}"#));
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&review_release).unwrap()).unwrap().status,V37Status::Applied);
    product.close_checked().unwrap();
    drop(root);
    std::fs::remove_dir_all(path).unwrap();
}

/// Select a physical history path only from the exact H-journaled original
/// thread/start response. The production RPC decoder first validates the same
/// command/response identity; this test reads the field it intentionally omits.
fn original_codex_thread_path_for_acl_test(product: &ProductDatabase<'_>,
    session: &str) -> (String, Option<std::path::PathBuf>) {
    let key = ("projectA".to_owned(), session.to_owned());
    let run = product.native_sessions.get(&key).expect("exact active H session");
    let thread = h::rpc_journal::observed_thread_id(&product.connection, "projectA", session,
        &run.operation_id, &run.custody.binding.generation, &run.open_request_id,
        run.custody.ticket.opaque(), &run.custody.custodian_nonce)
        .expect("production original RPC/source binding");
    assert_eq!(run.thread_id.as_deref(), Some(thread.as_str()));
    let q = Statement::prepare(product.connection.as_ptr(),
        "SELECT hex(r.raw_bytes)
           FROM main.gogoke_v37_rpc_steps s
           JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
             AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
             AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
             AND r.domain_id=s.domain_id AND r.session_id=s.session_id
             AND r.generation=s.generation
          WHERE s.domain_id='projectA' AND s.session_id=?1
            AND s.process_operation_id=?2 AND s.open_request_id=?3
            AND s.ticket=?4 AND s.custodian_nonce=?5
            AND s.step_id='thread-start' AND s.phase='OBSERVED'
            AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'")
        .expect("original thread source query");
    for (index, value) in [session, run.operation_id.as_str(), run.open_request_id.as_str(),
        run.custody.ticket.opaque(), run.custody.custodian_nonce.as_str()].iter().enumerate() {
        q.bind_text((index + 1) as i32, value).expect("bind original source identity");
    }
    assert!(q.step_row().expect("read original source"), "original thread/start response absent");
    let raw = unhex(&q.column_text(0).expect("original response bytes")).expect("decode original hex");
    assert!(!q.step_row().expect("unique original source"), "duplicate original thread response");
    let body = raw.strip_suffix(b"\n").expect("original JSONL LF");
    let Json::Object(top) = Parser::parse(std::str::from_utf8(body).expect("original UTF-8"))
        .expect("original response JSON") else { panic!("original response object"); };
    let Json::Object(result) = top.get(&JsonString::from_str("result")).expect("original result")
        else { panic!("original result object"); };
    let Json::Object(native_thread) = result.get(&JsonString::from_str("thread")).expect("original thread")
        else { panic!("original thread object"); };
    let Some(Json::String(found_id)) = native_thread.get(&JsonString::from_str("id")) else {
        panic!("original thread id absent");
    };
    assert_eq!(found_id.to_well_formed_string().as_deref(), Some(thread.as_str()));
    let path = match native_thread.get(&JsonString::from_str("path")) {
        None | Some(Json::Null) => None,
        Some(Json::String(value)) => Some(std::path::PathBuf::from(
            value.to_well_formed_string().expect("original path Unicode"))),
        _ => panic!("original thread path shape"),
    };
    (thread, path)
}

fn actual_pinned_codex_history_acl_vendor_qualification_without_model_call() {
    let _route_guard = route_b_test_guard();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-v37-vendor-acl-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
    let register = operation("K-INSTANCE", "register", "acl-register", "instanceA", 0,
        r#"{"driverId":"codex"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&register).unwrap()).unwrap().status,
        V37Status::Applied, "exact installed CLI registration");
    instance::record_observation(&mut product.connection, &root, &instance::ObservationRequest {
        request_id: "acl-synthetic-login-presence",
        request_bytes: b"SYNTHETIC_LOGIN_PRESENCE_NOT_AUTHENTICATION",
        instance_id: "instanceA", expected_revision: 1,
        observation: instance::InstanceObservation::LoggedIn,
    }).unwrap();
    instance::set_instance_concurrency_cap(&mut product.connection, &product.owner, "instanceA", 2).unwrap();
    seat::set_project_parallel_cap(&mut product.connection, &product.owner, "projectA", 2).unwrap();
    seat::store_template(&mut product.connection, NativeOrigin::user(&product.owner), StoreTemplate {
        domain_id: "projectA", template_id: "aclTemplate",
        settings_json: br#"{"effort":"high","model":"gpt-6-sol","permissionTier":"NETWORKED_WRITE"}"#,
    }).unwrap();
    for (seat_id, request_id) in [("seatA", "acl-seat-a"), ("seatB", "acl-seat-b")] {
        seat::create(&mut product.connection, NativeOrigin::user(&product.owner), CreateSeat {
            domain_id: "projectA", seat_id, template_id: "aclTemplate", instance_id: Some("instanceA"),
            kind: Kind::Long, request_id, request_bytes: request_id.as_bytes(),
        }).unwrap();
    }
    let source = crate::store::worktree::tests::make_source_fixture(&mut product.connection,
        &root, &product.owner, &mut product.process_custodian);
    let git = std::env::var_os("GOGOKE_CONTROLLED_GIT_PATH").expect("cloud pinned Git backend");
    let configuration = Json::Object(BTreeMap::from([
        (JsonString::from_str("schema"), Json::String(JsonString::from_str("gogoke.37.owner-configuration.v1"))),
        (JsonString::from_str("command"), Json::String(JsonString::from_str("worktree-source"))),
        (JsonString::from_str("repositoryId"), Json::String(JsonString::from_str("fixtureRepo"))),
        (JsonString::from_str("sourcePath"), Json::String(JsonString::from_str(source.to_str().unwrap()))),
        (JsonString::from_str("gitPath"), Json::String(JsonString::from_str(git.to_str().unwrap()))),
    ])).canonical();
    product.configure_user_v37(configuration.as_bytes()).unwrap();
    for (seat_id, tree_id) in [("seatA", "treeA"), ("seatB", "treeB")] {
        let create = operation("K-WORKTREE", "create", &format!("acl-create-{tree_id}"), tree_id, 0,
            &format!(r#"{{"repositoryId":"fixtureRepo","seatId":"{seat_id}"}}"#));
        assert_eq!(h::decode_receipt(&product.dispatch_user_request(&create).unwrap()).unwrap().status,
            V37Status::Applied, "exact F worktree");
    }
    for (session, seat_id) in [("sessionA", "seatA"), ("sessionB", "seatB")] {
        for (verb, revision) in [("admission-reserve", 0), ("admission-commit", 1)] {
            let request = operation("K-SESSION", verb, &format!("acl-{verb}-{session}"),
                session, revision, &format!(r#"{{"seatId":"{seat_id}","generation":"2"}}"#));
            assert_eq!(h::decode_receipt(&product.dispatch_user_request(&request).unwrap()).unwrap().status,
                V37Status::Applied, "exact H admission");
        }
    }
    let home = instance::resolve_codex_instance_home(&product.connection, &root, "instanceA")
        .expect("registered physical Codex HOME");
    let seat_a = seat::get(&product.connection, "projectA", "seatA").unwrap().unwrap();
    let seat_b = seat::get(&product.connection, "projectA", "seatB").unwrap().unwrap();
    let profile_name = |session: &str, incarnation: &str, generation: &str|
        h::launch::history_candidate_profile_name_for_test(&root.canonical_root().identity,
            "projectA", session, incarnation, generation);
    let name_a = profile_name("sessionA", &seat_a.incarnation, "2");
    let name_b = profile_name("sessionB", &seat_b.incarnation, "2");
    let name_a_resume = profile_name("sessionA", &seat_a.incarnation, "3");
    let profile_a = AppContainerProfile::ensure_for_cli(&name_a, true).unwrap();
    let profile_b = AppContainerProfile::ensure_for_cli(&name_b, true).unwrap();
    let profile_a_resume = AppContainerProfile::ensure_for_cli(&name_a_resume, true).unwrap();
    AppContainerProfile::set_history_candidate_home_for_test(&home.path,
        &[&profile_a, &profile_b, &profile_a_resume]).expect("initial CI-only HOME candidate ACL");
    // Pin down the first original Registry(Io(5)) source before H observes this
    // exact HOME. All probes are ordinary Host reads of the registered fixture.
    let marker = home.path.join("gogoke-instance.marker");
    println!("HISTORY_VENDOR_ACL_PRECHECK home_metadata={:?} home_entries={:?} marker_metadata={:?} marker_read={:?} absent_temp_metadata={:?} resolver={:?}",
        std::fs::symlink_metadata(&home.path).map(|v| v.is_dir()),
        std::fs::read_dir(&home.path).map(|entries| entries.count()),
        std::fs::symlink_metadata(&marker).map(|v| (v.is_file(), v.len())),
        std::fs::read(&marker).map(|bytes| bytes.len()),
        std::fs::symlink_metadata(home.path.join("temporary-homes")).map(|v| v.is_dir()),
        instance::resolve_codex_instance_home(&product.connection, &root, "instanceA")
            .map(|v| v.identity));
    let _candidate_guard = h::launch::install_history_acl_test_mode(home.path.clone(),
        home.identity.clone(), vec![name_a, name_b.clone(), name_a_resume]);
    let open = |session: &str, seat_id: &str, tree_id: &str| operation("K-SESSION", "open",
        &format!("acl-open-{session}"), session, 2,
        &format!(r#"{{"seatId":"{seat_id}","generation":"2","repositoryId":"fixtureRepo","worktreeId":"{tree_id}"}}"#));
    let run = (|| -> std::result::Result<String, String> {
        let open_a = open("sessionA", "seatA", "treeA");
        let opened_a = product.dispatch_user_request(&open_a)
            .map_err(|error| format!("CANDIDATE_REJECTED stage=A thread/start original={error:?}"))?;
        let opened_a = h::decode_receipt(&opened_a).expect("A original receipt shape");
        if opened_a.status != V37Status::Applied {
            return Err(format!("CANDIDATE_REJECTED stage=A open status={:?}", opened_a.status));
        }
        let (thread_a, Some(path_a)) = original_codex_thread_path_for_acl_test(&product, "sessionA")
            else { return Err("NOT_RUN original thread/start returned no path".into()); };
        if !path_a.is_absolute() || !path_a.starts_with(&home.path) {
            return Err("CANDIDATE_REJECTED original A path outside bound HOME".into());
        }
        let key_a = ("projectA".to_owned(), "sessionA".to_owned());
        let appended_a = product.native_append_rpc(&key_a, "acl-save-a", &thread_a,
            "cloud no-model nonsecret A history".into())
            .map_err(|error| format!("CANDIDATE_REJECTED stage=A native save original={error:?}"))?;
        if !matches!(appended_a, Some(Reply::Ack { .. })) {
            return Err(format!("CANDIDATE_REJECTED stage=A native save reply={appended_a:?}"));
        }
        let (id_a, a_own, a_peer, a_user) = AppContainerProfile::observed_vendor_history_leaf_acl_for_test(
            &path_a, &profile_a, &profile_b)
            .map_err(|error| format!("CANDIDATE_REJECTED stage=A original leaf={error}"))?;
        let open_b = open("sessionB", "seatB", "treeB");
        let opened_b = product.dispatch_user_request(&open_b)
            .map_err(|error| format!("CANDIDATE_REJECTED stage=B thread/start original={error:?}"))?;
        let opened_b = h::decode_receipt(&opened_b).expect("B original receipt shape");
        if opened_b.status != V37Status::Applied {
            return Err(format!("CANDIDATE_REJECTED stage=B open status={:?}", opened_b.status));
        }
        let (thread_b, Some(path_b)) = original_codex_thread_path_for_acl_test(&product, "sessionB")
            else { return Err("NOT_RUN original B thread/start returned no path".into()); };
        if !path_b.is_absolute() || !path_b.starts_with(&home.path) || path_a == path_b {
            return Err("CANDIDATE_REJECTED B path outside HOME or same history object".into());
        }
        let key_b = ("projectA".to_owned(), "sessionB".to_owned());
        let appended_b = product.native_append_rpc(&key_b, "acl-save-b", &thread_b,
            "cloud no-model nonsecret B history".into())
            .map_err(|error| format!("CANDIDATE_REJECTED stage=B native save original={error:?}"))?;
        if !matches!(appended_b, Some(Reply::Ack { .. })) {
            return Err(format!("CANDIDATE_REJECTED stage=B native save reply={appended_b:?}"));
        }
        let (id_b, b_own, b_peer, b_user) = AppContainerProfile::observed_vendor_history_leaf_acl_for_test(
            &path_b, &profile_b, &profile_a)
            .map_err(|error| format!("CANDIDATE_REJECTED stage=B original leaf={error}"))?;
        let parent_a = path_a.parent().expect("original A history parent");
        let parent_b = path_b.parent().expect("original B history parent");
        let (_directory_a_identity, dir_a_own, dir_a_peer) =
            AppContainerProfile::observed_vendor_history_directory_acl_for_test(
                parent_a, &profile_a, &profile_b)
                .map_err(|error| format!("CANDIDATE_REJECTED original date directory ACL={error}"))?;
        let (_directory_b_identity, dir_b_own, dir_b_peer) =
            AppContainerProfile::observed_vendor_history_directory_acl_for_test(
                parent_b, &profile_b, &profile_a)
                .map_err(|error| format!("CANDIDATE_REJECTED B date directory ACL={error}"))?;
        if dir_a_own.is_empty() || dir_a_peer.is_empty() || dir_b_own.is_empty()
            || dir_b_peer.is_empty() {
            return Err(format!("CANDIDATE_REJECTED shared date directory ACEs A=({dir_a_own:?},{dir_a_peer:?}) B=({dir_b_own:?},{dir_b_peer:?})"));
        }
        if id_a == id_b || a_own.is_empty() || b_own.is_empty() || !a_peer.is_empty()
            || !b_peer.is_empty() || a_user.is_empty() || b_user.is_empty() {
            return Err(format!("CANDIDATE_REJECTED actual history DACL A=({a_own:?},{a_peer:?},{a_user:?}) B=({b_own:?},{b_peer:?},{b_user:?})"));
        }
        std::fs::read(&path_a).map_err(|error| format!("CANDIDATE_REJECTED Host A read Win32={:?}", error.raw_os_error()))?;
        std::fs::read(&path_b).map_err(|error| format!("CANDIDATE_REJECTED Host B read Win32={:?}", error.raw_os_error()))?;
        let (foreign, own) = profile_b.probe_vendor_history_paths_for_test(&name_b,
            &path.join("peer-read-runner"), &path_a, &path_b).expect("exact profile B native read probe");
        if !foreign.starts_with("ERR:Some(5);") || own != "OK" {
            return Err(format!("CANDIDATE_REJECTED actual peer read foreign={foreign} own={own}"));
        }
        let index_path = home.path.join("session_index.jsonl");
        let index_state = if index_path.is_file() {
            let (_index_id, index_a, index_b, index_user) =
                AppContainerProfile::observed_vendor_history_leaf_acl_for_test(
                    &index_path, &profile_a, &profile_b)
                    .map_err(|error| format!("CANDIDATE_REJECTED actual session index ACL={error}"))?;
            let index_bytes = std::fs::read(&index_path)
                .map_err(|error| format!("CANDIDATE_REJECTED Host index read Win32={:?}",
                    error.raw_os_error()))?;
            let (peer_index, peer_own) = profile_b.probe_vendor_history_paths_for_test(&name_b,
                &path.join("peer-index-runner"), &index_path, &path_b)
                .expect("exact B LPAC native index read probe");
            if peer_own != "OK" {
                return Err(format!("CANDIDATE_REJECTED B own history during index probe={peer_own}"));
            }
            let contains_a = String::from_utf8_lossy(&index_bytes).contains(&thread_a);
            if contains_a && peer_index == "OK" {
                return Err("CANDIDATE_REJECTED shared index exposes A native thread to peer".into());
            }
            format!("PRESENT A={index_a:?} B={index_b:?} User={index_user:?} contains_A={contains_a} peer={peer_index}")
        } else { "NOT_RUN_INDEX_NOT_CREATED_BY_NO_MODEL_SEQUENCE".into() };
        let cwd_b = product.native_sessions.get(&key_b).unwrap().evidence.cwd()
            .to_string_lossy().into_owned();
        let cross = product.native_rpc(&key_b, "acl-cross-resume-a", Some(50),
            &Command::ThreadResume { thread_id: thread_a.clone(), cwd: cwd_b,
                model: "gpt-6-sol".into() });
        let cross_state = match cross {
            Ok(Some(Reply::RemoteError { raw_frame, .. })) =>
                format!("REMOTE_ERROR:{}", String::from_utf8_lossy(&raw_frame)),
            Ok(Some(Reply::Thread { .. })) => "PEER_RESUME_SUCCEEDED".into(),
            Ok(other) => format!("UNEXPECTED:{other:?}"),
            Err(error) => format!("ORIGINAL_ERROR:{error:?}"),
        };
        if cross_state == "PEER_RESUME_SUCCEEDED" {
            return Err("CANDIDATE_REJECTED peer native thread/resume succeeded".into());
        }
        let stop_a = operation("K-SESSION", "stop", "acl-stop-a", "sessionA", 3,
            r#"{"seatId":"seatA","generation":"2"}"#);
        let stopped_a = product.dispatch_user_request(&stop_a)
            .map_err(|error| format!("CANDIDATE_REJECTED stage=A normal stop original={error:?}"))?;
        if h::decode_receipt(&stopped_a).expect("A stop receipt").status != V37Status::Applied {
            return Err("CANDIDATE_REJECTED A normal stop did not apply".into());
        }
        let resume_a = operation("K-SESSION", "resume", "acl-resume-a", "sessionA", 4,
            r#"{"generation":"2"}"#);
        let resumed = product.dispatch_user_request(&resume_a);
        let resume_state = match resumed {
            Ok(bytes) => {
                let receipt = h::decode_receipt(&bytes).expect("same UUID resume receipt");
                format!("{:?}", receipt.status)
            }
            Err(error) => format!("ORIGINAL_ERROR:{error:?}"),
        };
        println!("HISTORY_VENDOR_ACL_FACT primitive=CI_ONLY same_home=true fixed_codex=true date_dir_A={dir_a_own:?}/{dir_a_peer:?} date_dir_B={dir_b_own:?}/{dir_b_peer:?} A_leaf_dacl={a_own:?}/{a_peer:?}/{a_user:?} B_leaf_dacl={b_own:?}/{b_peer:?}/{b_user:?} host_reads=OK/OK peer_read={foreign}/{own} session_index={index_state} cross_resume={cross_state} normal_resume={resume_state}");
        if resume_state != "Applied" {
            return Err(format!("CANDIDATE_REJECTED normal same-UUID new-generation resume={resume_state}"));
        }
        if !cross_state.starts_with("REMOTE_ERROR:") {
            return Err(format!("NOT_RUN peer native resume was not an original vendor rejection: {cross_state}"));
        }
        Ok("CANDIDATE_QUALIFIED fixed Codex no-model current CI-only HOME sequence".into())
    })();
    match run {
        Ok(result) => println!("HISTORY_VENDOR_ACL_QUALIFICATION verdict={result}"),
        Err(reason) => println!("HISTORY_VENDOR_ACL_QUALIFICATION verdict={reason}"),
    }
    assert!(h::launch::history_candidate_home_checks_for_test() > 0,
        "the actual H launch never used the exact-root candidate ACL test seam");
    drop(product);
    drop(_candidate_guard);
    drop((profile_a, profile_b, profile_a_resume));
    drop(root);
    if let Err(error) = std::fs::remove_dir_all(&path) {
        println!("HISTORY_VENDOR_ACL_CLEANUP_UNVERIFIED original={error}");
    }
}
