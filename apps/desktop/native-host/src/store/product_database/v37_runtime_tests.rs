use super::*;
use crate::root::RootLock;
use crate::store::seat::{CreateSeat, Kind, StoreTemplate};
use crate::store::same_open::route_b_test_guard;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[path = "v37_stalled_health_tests.rs"]
mod stalled_health;

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
    health_control_work_turn_with_prefix(product,early_terminal,cause,|_| {})
}

fn health_control_work_turn_with_prefix(product:&mut ProductDatabase<'_>,early_terminal:bool,cause:&str,
    before_terminal:impl FnOnce(&mut ProductDatabase<'_>))->ledger::RawSourceKey {
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
    if !early_terminal {before_terminal(product);}
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
    operation_in_domain(domain, family, verb, id, target, revision, payload)
}

fn operation_in_domain(domain: &str, family: &str, verb: &str, id: &str,
    target: &str, revision: u64, payload: &str) -> V37Request {
    decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"{family}","operation":"{verb}","requestId":"{id}","targetId":"{target}","domainId":"{domain}","expectedRevision":"{revision}","payload":{payload}}}"#).as_bytes()).unwrap()
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

// This fixture uses the official pinned CLI's own File backend. The marker is
// deliberately invalid and public; neither helper nor ProductDatabase reads
// auth.json data. Custody/STOPPED facts come from the actual child and H.
fn qualify_synthetic_file_backend(product: &mut ProductDatabase<'_>, root: &RootLock) {
    use crate::process::{CredentialBinding, DurableStopConfirmation, StopBudgets};
    use crate::root::inspect_root;
    use std::os::windows::fs::MetadataExt;
    let home = instance::resolve_codex_instance_home(&product.connection, root, "instanceA").unwrap();
    let mut scope = product.prepare_owner_codex_login("instanceA")
        .expect("actual fixed CLI initial empty account observer");
    assert!(scope.credential_custody.is_none(), "initial auth object is absent");
    scope.login.launch.arguments = vec!["login".into(), "--with-api-key".into()];
    let prepared = product.process_custodian.prepare(&scope.login).unwrap();
    let operation_id = "two-scope-synthetic-cli-file-login";
    authority::record_prepared_process(&mut product.connection, operation_id, &prepared).unwrap();
    product.process_custodian.activate(&prepared).unwrap();
    authority::mark_process_active(&mut product.connection, operation_id, &prepared).unwrap();
    product.process_custodian.active(&prepared.ticket).unwrap()
        .write_persistent_frame(b"gogoke-synthetic-invalid-key-no-auth\n").unwrap();
    product.process_custodian.close_child_input(&prepared.ticket).unwrap();
    assert!(product.process_custodian.active(&prepared.ticket).unwrap()
        .wait(Duration::from_secs(15)).unwrap(), "real CLI credential-file action did not exit");
    let proof = product.process_custodian.stop(&prepared.ticket,
        StopBudgets::production(), || Ok(())).unwrap();
    assert_eq!(proof.exit_code, Some(0), "original CLI stderr: {}",
        product.process_custodian.active(&prepared.ticket).unwrap().stderr_tail());
    let revision = authority::mark_process_stopped(&mut product.connection,
        operation_id, &proof).unwrap();
    product.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
        ticket: prepared.ticket.clone(), custodian_nonce: prepared.custodian_nonce.clone(),
        identity: prepared.identity.clone(), proof_hash: proof.proof_hash(), durable_revision: revision,
    }).unwrap();
    assert_eq!(inspect_root(&scope.runtime_home).unwrap().identity, scope.runtime_identity);
    fn physical_runtime(path: &std::path::Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let child = entry.unwrap().path();
            let metadata = std::fs::symlink_metadata(&child).unwrap();
            assert_eq!(metadata.file_attributes() & 0x400, 0, "runtime reparse child: {child:?}");
            if metadata.is_dir() { physical_runtime(&child); }
        }
    }
    physical_runtime(&scope.runtime_home);
    std::fs::remove_dir_all(&scope.runtime_home).unwrap();
    let (observed, links) = CredentialBinding::observe_source_metadata(root,
        &home.path.join("auth.json"), &home.identity).expect("CLI-created physical credential object");
    assert_eq!(links, 1, "initial official CLI source has one physical name");
    let account = operation("K-INSTANCE", "login-state", "two-scope-original-account",
        "instanceA", 1, "{}");
    let account_reply = product.dispatch_owner_login_observation(&account)
        .expect("original fixed CLI account/read observation");
    assert!(String::from_utf8_lossy(&account_reply).contains("\"state\":\"LOGGED_IN\""),
        "synthetic marker establishes presence only: {}", String::from_utf8_lossy(&account_reply));
    let bound = operation("K-INSTANCE", "login-state", "two-scope-file-bound",
        "instanceA", 2, "{}");
    let bound_reply = product.dispatch_owner_file_backend_observation(&bound)
        .expect("separate actual FileBound startup observer");
    assert!(String::from_utf8_lossy(&bound_reply).contains("\"state\":\"LOGGED_IN\""));
    let backend = instance::read_usable_credential_backend(&product.connection, "instanceA").unwrap();
    assert_eq!(backend.backend, instance::CredentialBackend::File);
    assert_eq!(backend.startup_selector, instance::CredentialStartupSelector::FileBound);
    let (readback, count) = CredentialBinding::observe_source_metadata(root,
        &home.path.join("auth.json"), &home.identity).unwrap();
    assert_eq!((readback, count), (observed, 1), "observers preserved the original CLI file object");
}

fn actual_saved_thread_files(home: &std::path::Path, thread: &str) -> Vec<std::path::PathBuf> {
    use std::os::windows::fs::MetadataExt;
    let mut pending = vec![home.to_path_buf()];
    let mut matches = Vec::new();
    let suffix = format!("{thread}.jsonl");
    while let Some(parent) = pending.pop() {
        for entry in std::fs::read_dir(&parent).unwrap() {
            let path = entry.unwrap().path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            assert_eq!(metadata.file_attributes() & 0x400, 0, "native history reparse: {path:?}");
            if metadata.is_dir() { pending.push(path); }
            else if metadata.is_file() && path.file_name().unwrap().to_string_lossy().ends_with(&suffix) {
                matches.push(path);
            }
        }
    }
    matches
}

#[test]
fn actual_pinned_codex_two_scope_file_history_and_stopped_revocation_without_model() {
    use crate::process::{CredentialAliasScope, CredentialBinding};
    use crate::store::session_transport::codex_rpc::Reply;
    let _guard = route_b_test_guard();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-v37-real-two-scope-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
    let register = operation("K-INSTANCE", "register", "two-scope-register", "instanceA", 0,
        r#"{"driverId":"codex"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&register).unwrap()).unwrap().status,
        V37Status::Applied, "actual fixed Codex 0.160.0 registration");
    qualify_synthetic_file_backend(&mut product, &root);
    instance::set_instance_concurrency_cap(&mut product.connection, &product.owner,
        "instanceA", 2).unwrap();
    let source = crate::store::worktree::tests::make_source_fixture(&mut product.connection,
        &root, &product.owner, &mut product.process_custodian);
    let git = std::env::var_os("GOGOKE_CONTROLLED_GIT_PATH").expect("cloud Git pin");
    let configuration = Json::Object(BTreeMap::from([
        (JsonString::from_str("schema"), Json::String(JsonString::from_str("gogoke.37.owner-configuration.v1"))),
        (JsonString::from_str("command"), Json::String(JsonString::from_str("worktree-source"))),
        (JsonString::from_str("repositoryId"), Json::String(JsonString::from_str("fixtureRepo"))),
        (JsonString::from_str("sourcePath"), Json::String(JsonString::from_str(source.to_str().unwrap()))),
        (JsonString::from_str("gitPath"), Json::String(JsonString::from_str(git.to_str().unwrap()))),
    ])).canonical();
    product.configure_user_v37(configuration.as_bytes()).unwrap();
    for (domain, seat_id, tree_id, session_id) in [
        ("projectA", "seatA", "treeA", "sessionA"),
        ("projectB", "seatB", "treeB", "sessionB"),
    ] {
        seat::set_project_parallel_cap(&mut product.connection, &product.owner, domain, 1).unwrap();
        seat::store_template(&mut product.connection, NativeOrigin::user(&product.owner), StoreTemplate {
            domain_id: domain, template_id: "templateA",
            settings_json: br#"{"effort":"high","model":"gpt-6-sol","permissionTier":"NETWORKED_WRITE"}"#,
        }).unwrap();
        seat::create(&mut product.connection, NativeOrigin::user(&product.owner), CreateSeat {
            domain_id: domain, seat_id, template_id: "templateA", instance_id: Some("instanceA"),
            kind: Kind::Long, request_id: &format!("two-scope-create-{seat_id}"),
            request_bytes: format!("real two-scope seat {seat_id}").as_bytes(),
        }).unwrap();
        let create = operation_in_domain(domain, "K-WORKTREE", "create",
            &format!("two-scope-tree-{tree_id}"), tree_id, 0,
            &format!(r#"{{"repositoryId":"fixtureRepo","seatId":"{seat_id}"}}"#));
        let created = h::decode_receipt(&product.dispatch_user_request(&create).unwrap()).unwrap();
        assert_eq!(created.status, V37Status::Applied,
            "original worktree receipt: {}", String::from_utf8_lossy(&created.raw_bytes));
        for (verb, phase, revision) in [("admission-reserve", "reserve", 0),
            ("admission-commit", "commit", 1)] {
            let request = operation_in_domain(domain, "K-SESSION", verb,
                &format!("two-scope-{phase}-{session_id}"), session_id, revision,
                &format!(r#"{{"seatId":"{seat_id}","generation":"2"}}"#));
            let receipt = h::decode_receipt(&product.dispatch_user_request(&request).unwrap()).unwrap();
            assert_eq!(receipt.status, V37Status::Applied,
                "original admission receipt: {}", String::from_utf8_lossy(&receipt.raw_bytes));
        }
        let open = operation_in_domain(domain, "K-SESSION", "open",
            &format!("two-scope-open-{session_id}"), session_id, 2,
            &format!(r#"{{"seatId":"{seat_id}","generation":"2","repositoryId":"fixtureRepo","worktreeId":"{tree_id}"}}"#));
        let opened = h::decode_receipt(&product.dispatch_user_request(&open)
            .expect("real private CODEX_HOME app-server initialize/config/thread-start original error")).unwrap();
        assert_eq!(opened.status, V37Status::Applied,
            "original native open reply: {}", String::from_utf8_lossy(&opened.raw_bytes));
        assert_eq!(opened.revision, 3);
        let key = (domain.to_owned(), session_id.to_owned());
        assert!(product.native_sessions.get(&key).unwrap().evidence.file_credentials_bound(),
            "original native launch selected fixed File-bound credentials");
        let observed = Statement::prepare(product.connection.as_ptr(),
            "SELECT count(*) FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2 AND phase='OBSERVED'").unwrap();
        observed.bind_text(1, domain).unwrap(); observed.bind_text(2, session_id).unwrap();
        assert!(observed.step_row().unwrap());
        assert_eq!(observed.column_text(0).unwrap(), "3", "original initialize/config/thread-start replies");
    }
    let key_a = ("projectA".to_owned(), "sessionA".to_owned());
    let key_b = ("projectB".to_owned(), "sessionB".to_owned());
    let thread_a = product.native_sessions.get(&key_a).unwrap().thread_id.clone().unwrap();
    let thread_b = product.native_sessions.get(&key_b).unwrap().thread_id.clone().unwrap();
    assert_ne!(thread_a, thread_b, "two original thread/start replies");
    for (key, thread, step) in [(&key_a, &thread_a, "two-scope-save-a"),
        (&key_b, &thread_b, "two-scope-save-b")] {
        let reply = product.native_append_rpc(key, step, thread,
            format!("synthetic no-model durable history {step}")).unwrap();
        assert!(matches!(reply, Some(Reply::Ack { .. })), "original native append ACK: {reply:?}");
    }
    let history_id = |domain: &str, session: &str| {
        let query = Statement::prepare(product.connection.as_ptr(),
            "SELECT history_id FROM main.gogoke_v37_instance_histories WHERE instance_id='instanceA' AND domain_id=?1 AND session_id=?2 AND state='READY'").unwrap();
        query.bind_text(1, domain).unwrap(); query.bind_text(2, session).unwrap();
        assert!(query.step_row().unwrap(), "original F history row for {domain}/{session}");
        let id = query.column_text(0).unwrap();
        assert!(!query.step_row().unwrap(), "one F history per original scope");
        id
    };
    let history_a = history_id("projectA", "sessionA");
    let history_b = history_id("projectB", "sessionB");
    assert_ne!(history_a, history_b);
    let dir_a = instance::resolve_private_history_directory(&product.connection, &root, &history_a).unwrap();
    let dir_b = instance::resolve_private_history_directory(&product.connection, &root, &history_b).unwrap();
    assert_ne!(dir_a.identity, dir_b.identity, "independent physical F histories");
    assert_ne!(dir_a.path, dir_b.path);
    let home = instance::resolve_codex_instance_home(&product.connection, &root, "instanceA").unwrap();
    assert!(dir_a.path.starts_with(&home.path) && dir_b.path.starts_with(&home.path));
    for (key, expected) in [(&key_a, &dir_a.path), (&key_b, &dir_b.path)] {
        let request = product.native_sessions.get(key).unwrap().evidence.request().unwrap();
        let environment = request.launch.environment.unwrap();
        assert_eq!(environment.iter().find(|(name, _)| name == "CODEX_HOME")
            .map(|(_, value)| value.as_str()), expected.to_str(),
            "production launch maps the registered F private directory");
    }
    let saved_a = actual_saved_thread_files(&dir_a.path, &thread_a);
    let saved_b = actual_saved_thread_files(&dir_b.path, &thread_b);
    assert_eq!(saved_a.len(), 1, "original saved A thread file in A F history");
    assert_eq!(saved_b.len(), 1, "original saved B thread file in B F history");
    assert!(actual_saved_thread_files(&dir_a.path, &thread_b).is_empty()
        && actual_saved_thread_files(&dir_b.path, &thread_a).is_empty(),
        "peer native history UUID absent from other F directory");
    let object = instance::read_credential_object(&product.connection, "instanceA").unwrap().unwrap();
    let source_path = home.path.join("auth.json");
    let (source_id, links) = CredentialBinding::observe_source_metadata(&root,
        &source_path, &home.identity).unwrap();
    assert_eq!(object.file_identity, source_id);
    assert_eq!(links, 3, "one CLI source and two registered physical aliases");
    let aliases = instance::read_credential_aliases(&product.connection, "instanceA").unwrap();
    assert_eq!(aliases.len(), 2);
    assert!(aliases.iter().all(|row| row.state == "ACTIVE" && row.source_file_identity == source_id));
    let scopes = [CredentialAliasScope {root:dir_a.path.clone(),root_identity:dir_a.identity.clone()},
        CredentialAliasScope {root:dir_b.path.clone(),root_identity:dir_b.identity.clone()}];
    let binding = CredentialBinding::open_registered(&root, &source_path, &home.identity,
        &source_id, &scopes).unwrap();
    binding.verify_registered_aliases(&scopes).unwrap();
    drop(binding);
    let profiles = instance::read_credential_profiles(&product.connection, "instanceA").unwrap();
    assert_eq!(profiles.len(), 2);
    let profile_a = profiles.iter().find(|row| row.history_id == history_a).unwrap();
    let profile_b = profiles.iter().find(|row| row.history_id == history_b).unwrap();
    assert!(profile_a.state == "ACTIVE" && profile_b.state == "ACTIVE");
    assert_ne!(profile_a.profile_sid, profile_b.profile_sid, "distinct current model profile SIDs");
    assert_eq!(profile_a.source_file_identity, source_id);
    assert_eq!(profile_b.source_file_identity, source_id);
    let stop_a = operation_in_domain("projectA", "K-SESSION", "stop", "two-scope-stop-a",
        "sessionA", 3, r#"{"seatId":"seatA","generation":"2"}"#);
    let stopped_a = h::decode_receipt(&product.dispatch_user_request(&stop_a).unwrap()).unwrap();
    assert_eq!(stopped_a.status, V37Status::Applied,
        "original A STOPPED and ACL revoke: {}", String::from_utf8_lossy(&stopped_a.raw_bytes));
    assert!(product.native_sessions.contains_key(&key_b), "B actual native child retained");
    let profiles = instance::read_credential_profiles(&product.connection, "instanceA").unwrap();
    assert_eq!(profiles.iter().find(|row| row.history_id == history_a).unwrap().state, "REVOKED");
    assert_eq!(profiles.iter().find(|row| row.history_id == history_b).unwrap().state, "ACTIVE");
    let aliases = instance::read_credential_aliases(&product.connection, "instanceA").unwrap();
    assert_eq!(aliases.iter().find(|row| row.history_id == history_a).unwrap().state, "DORMANT");
    assert_eq!(aliases.iter().find(|row| row.history_id == history_b).unwrap().state, "ACTIVE");
    let reply_b = product.native_append_rpc(&key_b, "two-scope-b-after-a-stop", &thread_b,
        "synthetic B no-model save after A STOPPED".into()).unwrap();
    assert!(matches!(reply_b, Some(Reply::Ack { .. })), "B original child survives A revoke: {reply_b:?}");
    let resume_a = operation_in_domain("projectA", "K-SESSION", "resume", "two-scope-resume-a",
        "sessionA", 4, r#"{"generation":"2"}"#);
    let resumed = h::decode_receipt(&product.dispatch_user_request(&resume_a)
        .expect("original A thread/resume from its F history")).unwrap();
    assert_eq!(resumed.status, V37Status::Applied,
        "original same-UUID resume: {}", String::from_utf8_lossy(&resumed.raw_bytes));
    assert_eq!(product.native_sessions.get(&key_a).unwrap().thread_id.as_deref(), Some(thread_a.as_str()));
    let same_dir = instance::resolve_private_history_directory(&product.connection, &root, &history_a).unwrap();
    assert_eq!(same_dir.identity, dir_a.identity, "A new generation reuses F physical history");
    assert_eq!(same_dir.path, dir_a.path);
    let stop_resumed_a = operation_in_domain("projectA", "K-SESSION", "stop",
        "two-scope-stop-resumed-a", "sessionA", 5,
        r#"{"seatId":"seatA","generation":"3"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop_resumed_a).unwrap()).unwrap().status,
        V37Status::Applied);
    let stop_b = operation_in_domain("projectB", "K-SESSION", "stop", "two-scope-stop-b",
        "sessionB", 3, r#"{"seatId":"seatB","generation":"2"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop_b).unwrap()).unwrap().status,
        V37Status::Applied);
    product.close_checked().unwrap();
    drop(root);
    std::fs::remove_dir_all(path).unwrap();
}
