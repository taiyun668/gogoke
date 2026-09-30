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
        settings_json: br#"{"permissionTier":"NETWORKED_WRITE","model":"gpt-6-sol","effort":"high"}"#,
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
    let row = Statement::prepare(product.connection.as_ptr(), "SELECT count(*) FROM main.gogoke_v37_rpc_steps WHERE phase='OBSERVED'").unwrap();
    assert!(row.step_row().unwrap());
    assert_eq!(row.column_text(0).unwrap(), "3", "initialize, effective config and real thread response; initialized has no ACK");
    drop(row);
    let stop = operation("K-SESSION", "stop", "stop-session", "sessionA", 2, r#"{"seatId":"seatA","generation":"2"}"#);
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&stop).unwrap()).unwrap().status, V37Status::Applied);
    let fact = runtime::observe_stop_fact(&product.connection, "projectA", "sessionA").unwrap();
    assert!(fact.is_some(), "actual native Job and same-store durable stop fact");
    product.close_checked().unwrap();
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&open).unwrap()).unwrap().status, V37Status::Replayed);
    product.close_checked().unwrap();
    drop(root);
    std::fs::remove_dir_all(path).unwrap();
}
