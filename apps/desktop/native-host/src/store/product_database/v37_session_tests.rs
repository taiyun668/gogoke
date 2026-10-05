use super::*;
use crate::root::RootLock;
use crate::store::same_open::route_b_test_guard;
use crate::store::seat::{CreateSeat, Kind, StoreTemplate};
use std::time::{SystemTime, UNIX_EPOCH};

fn request(operation: &str, id: &str, session: &str, seat: &str, revision: u64) -> V37Request {
    decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"{operation}","requestId":"{id}","targetId":"{session}","domainId":"projectA","expectedRevision":"{revision}","payload":{{"seatId":"{seat}","generation":"2"}}}}"#).as_bytes()).unwrap()
}

fn status(product: &mut ProductDatabase<'_>, r: &V37Request) -> V37Status {
    h::decode_receipt(&product.dispatch_user_request(r).unwrap()).unwrap().status
}

fn worktree_request(id: &str, target: &str, domain: &str, seat_id: &str) -> V37Request {
    decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-WORKTREE","operation":"create","requestId":"{id}","targetId":"{target}","domainId":"{domain}","expectedRevision":"0","payload":{{"repositoryId":"fixtureRepo","seatId":"{seat_id}"}}}}"#).as_bytes()).unwrap()
}

#[test]
fn product_merge_history_rechecks_current_grant_without_git_and_preserves_unknown_cause() {
    // This is the actual product dispatch and E permission reader. Trusted-turn
    // construction arranges ingress only; authenticated model ingress is NOT_RUN.
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-v37-merge-history-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
    product.connection.execute("INSERT INTO main.gogoke_v37_instances VALUES('instanceA','codex','homeA','identityA','sha256:fixture','fixture','INSTALLED','LOGGED_OUT',1)").unwrap();
    seat::store_template(&mut product.connection, NativeOrigin::user(&product.owner), StoreTemplate {
        domain_id: "projectA", template_id: "templateA", settings_json: b"{}",
    }).unwrap();
    for seat_id in ["writerSeat", "mergerSeat"] {
        seat::create(&mut product.connection, NativeOrigin::user(&product.owner), CreateSeat {
            domain_id: "projectA", seat_id, template_id: "templateA", instance_id: Some("instanceA"),
            kind: Kind::Long, request_id: seat_id, request_bytes: seat_id.as_bytes(),
        }).unwrap();
    }
    let merger = seat::get(&product.connection, "projectA", "mergerSeat").unwrap().unwrap();
    seat::set_dispatch_state(&mut product.connection, &merger, true).unwrap();
    let merger = seat::get(&product.connection, "projectA", "mergerSeat").unwrap().unwrap();
    let caller = seat::NativeSeatCall::from_verified_h_turn(&merger, "currentMergeTurn").unwrap();
    seat::initialize_policy(&mut product.connection, &product.owner, "projectA", "WORK").unwrap();
    seat::configure_call_grant(&mut product.connection, &product.owner, "projectA", "mergerSeat",
        "MAIN", seat::CallAction::Merge, None, 1).unwrap();
    product.connection.execute("INSERT INTO main.gogoke_v37_worktree_sources VALUES('repoA','absent-source','sourceIdentity','absent-common','commonIdentity','HTTPS','baseline','old digest','old version',1)").unwrap();
    product.connection.execute("INSERT INTO main.gogoke_v37_worktrees(worktree_id,path_id,repository_id,domain_id,seat_id,seat_incarnation,seat_generation,seat_revision,permission_tier,instance_id,source_revision,worktree_path,worktree_identity,git_pointer_hash,git_pointer_len,git_pointer_identity,common_identity,baseline_commit,state,revision) VALUES('treeA','wtA','repoA','projectA','writerSeat','incarnationA',1,1,'NetworkedWrite','instanceA',1,'absent-cleaned-tree','pathIdentity','pointerHash',10,'pointerIdentity','commonIdentity','baseline','REGISTERED',1)").unwrap();
    product.connection.execute("INSERT INTO main.gogoke_v37_worktree_lifecycle VALUES('treeA','CLEANED',4,'accepted merge',NULL,'stopA')").unwrap();
    let raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-WORKTREE","operation":"merge","requestId":"mergeA","targetId":"treeA","domainId":"projectA","expectedRevision":"2","payload":{"decision":"MERGE","reason":"accepted merge"}}"#;
    let request = decode_request(raw).unwrap();
    let commit = "a".repeat(40);
    let row = Statement::prepare(product.connection.as_ptr(),
        "INSERT INTO main.gogoke_v37_worktree_lifecycle_ops(request_id,request_hash,worktree_id,operation,phase,cause,result_commit) VALUES('mergeA',?1,'treeA','MERGE','APPLIED','',?2)").unwrap();
    row.bind_text(1, &crate::store::digest::sha256_hex(raw)).unwrap();
    row.bind_text(2, &commit).unwrap(); row.step_done().unwrap(); drop(row);
    let count_custody = |product: &ProductDatabase<'_>| {
        let row = Statement::prepare(product.connection.as_ptr(),
            "SELECT count(*) FROM main.gogoke_coordination_process_custody").unwrap();
        assert!(row.step_row().unwrap()); row.column_text(0).unwrap()
    };
    let before = count_custody(&product);
    for changed_pin in [false, true] {
        if changed_pin {
            product.connection.execute("INSERT INTO main.gogoke_v37_worktree_programs VALUES('repoA','absent-changed-git.exe')").unwrap();
        }
        let bytes = product.dispatch_native_worktree(&request, &caller).unwrap();
        let receipt = h::decode_receipt(&bytes).unwrap();
        assert_eq!(receipt.status, V37Status::Replayed);
        assert_eq!(receipt.revision, 3);
        let result = receipt.into_result();
        assert_eq!(result.get(&JsonString::from_str("targetCommit")), Some(&text(&commit)));
        assert!(!result.contains_key(&JsonString::from_str("childSealIntent")));
        assert!(!result.contains_key(&JsonString::from_str("childCommit")));
        assert_eq!(count_custody(&product), before, "product replay must not resolve or launch Git");
    }
    let partial_hash = "b".repeat(40);
    let original_cause = format!("original Windows error 5; childCommit={partial_hash}");
    let update = Statement::prepare(product.connection.as_ptr(),
        "UPDATE main.gogoke_v37_worktree_lifecycle_ops SET phase='UNKNOWN',cause=?1 WHERE request_id='mergeA'").unwrap();
    update.bind_text(1, &original_cause).unwrap(); update.step_done().unwrap(); drop(update);
    let bytes = product.dispatch_native_worktree(&request, &caller).unwrap();
    assert_eq!(h::decode_receipt(&bytes).unwrap().status, V37Status::Unknown);
    let wire = String::from_utf8(bytes).unwrap();
    assert!(wire.contains("original Windows error 5") && wire.contains(&partial_hash));
    let unchanged = Statement::prepare(product.connection.as_ptr(),
        "SELECT phase,cause,result_commit FROM main.gogoke_v37_worktree_lifecycle_ops WHERE request_id='mergeA'").unwrap();
    assert!(unchanged.step_row().unwrap());
    assert_eq!(unchanged.column_text(0).unwrap(), "UNKNOWN");
    assert_eq!(unchanged.column_text(1).unwrap(), original_cause);
    assert_eq!(unchanged.column_text(2).unwrap(), commit); drop(unchanged);
    product.connection.execute("UPDATE main.gogoke_v37_worktree_lifecycle_ops SET phase='APPLIED',cause='' WHERE request_id='mergeA'").unwrap();
    product.connection.execute("DELETE FROM main.gogoke_v37_seat_policy_grants WHERE domain_id='projectA'").unwrap();
    let denied = h::decode_receipt(&product.dispatch_native_worktree(&request, &caller).unwrap()).unwrap();
    assert!(!matches!(denied.status, V37Status::Applied | V37Status::Replayed),
        "historical success cannot bypass the current E.2 grant");
    assert_eq!(count_custody(&product), before);
    product.close_checked().unwrap(); drop(root);
    let actual = std::fs::canonicalize(&path).unwrap();
    assert!(actual.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
    assert!(actual.file_name().unwrap().to_string_lossy().starts_with("gogoke-v37-merge-history-"));
    std::fs::remove_dir_all(actual).unwrap();
}

#[test]
fn product_worktree_source_reopens_and_original_requests_never_reissue_unknown() {
    use crate::store::worktree;
    use crate::store::atomic::{Json, JsonString};
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-v37-product-worktree-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    product.connection.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:fixture','fixture','INSTALLED','LOGGED_OUT',1)").unwrap();
    seat::store_template(&mut product.connection, NativeOrigin::user(&product.owner), StoreTemplate {
        domain_id: "projectA", template_id: "templateA", settings_json: br#"{"permissionTier":"ISOLATED_WRITE"}"#,
    }).unwrap();
    seat::create(&mut product.connection, NativeOrigin::user(&product.owner), CreateSeat {
        domain_id: "projectA", seat_id: "seatA", template_id: "templateA", instance_id: Some("instanceA"),
        kind: Kind::Long, request_id: "create-seat", request_bytes: b"product worktree seat",
    }).unwrap();
    let source = worktree::tests::make_source_fixture(&mut product.connection, &root,
        &product.owner, &mut product.process_custodian);
    let git = std::env::var_os("GOGOKE_CONTROLLED_GIT_PATH").unwrap();
    let configuration = Json::Object(BTreeMap::from([
        (JsonString::from_str("schema"), text("gogoke.37.owner-configuration.v1")),
        (JsonString::from_str("command"), text("worktree-source")),
        (JsonString::from_str("repositoryId"), text("fixtureRepo")),
        (JsonString::from_str("sourcePath"), text(source.to_str().unwrap())),
        (JsonString::from_str("gitPath"), text(git.to_str().unwrap())),
    ])).canonical();
    product.configure_user_v37(configuration.as_bytes()).unwrap();
    product.close_checked().unwrap();
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    assert_eq!(status(&mut product, &worktree_request("wrong-domain", "badTree", "projectB", "seatA")), V37Status::Denied);
    assert_eq!(status(&mut product, &worktree_request("wrong-seat", "badTree", "projectA", "missingSeat")), V37Status::Denied);
    let original = worktree_request("create-tree", "treeA", "projectA", "seatA");
    assert_eq!(status(&mut product, &original), V37Status::Applied);
    assert_eq!(status(&mut product, &original), V37Status::Replayed);
    assert_eq!(status(&mut product, &worktree_request("create-tree", "treeB", "projectA", "seatA")), V37Status::Conflict);
    // Simulate an uncertain external completion in this test's actual store.
    // No production request can select or clear this phase.
    product.connection.execute("UPDATE main.gogoke_v37_worktree_operations SET phase='UNKNOWN' WHERE request_id='create-tree'").unwrap();
    let count = |product: &ProductDatabase<'_>| {
        let row = Statement::prepare(product.connection.as_ptr(), "SELECT count(*) FROM main.gogoke_coordination_process_custody").unwrap();
        assert!(row.step_row().unwrap()); row.column_text(0).unwrap()
    };
    let before = count(&product);
    assert_eq!(status(&mut product, &original), V37Status::Unknown);
    assert_eq!(count(&product), before, "UNKNOWN/replay must not launch even git --version");
    product.close_checked().unwrap();
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    assert_eq!(status(&mut product, &original), V37Status::Unknown);
    assert_eq!(count(&product), before);
    product.close_checked().unwrap();
    drop(root);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn product_reopens_exact_previous_worktree_schema_preserving_unpinned_sources() {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-v37-old-worktree-schema-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    product.connection.execute("DROP TABLE main.gogoke_v37_worktree_programs").unwrap();
    product.connection.execute("INSERT INTO main.gogoke_v37_worktree_sources VALUES('legacyRepo','sourceA','sourceId','commonA','commonId','HTTPS','baseline','old digest','old version',1)").unwrap();
    product.close_checked().unwrap();
    let mut product = ProductDatabase::open(&root, &database).expect("known earlier family remains readable");
    let row = Statement::prepare(product.connection.as_ptr(), "SELECT git_digest,git_version FROM main.gogoke_v37_worktree_sources WHERE repository_id='legacyRepo'").unwrap();
    assert!(row.step_row().unwrap());
    assert_eq!(row.column_text(0).unwrap(), "old digest");
    assert_eq!(row.column_text(1).unwrap(), "old version");
    drop(row);
    assert!(matches!(crate::store::worktree::resolve_registered_git(&mut product.connection,
        &root, &product.owner, "legacyRepo", &mut product.process_custodian),
        Err(crate::store::worktree::WorktreeError::Denied)));
    product.close_checked().unwrap();
    drop(root);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn product_admission_enforces_persisted_caps_and_rolls_back_busy_on_denial() {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-v37-product-admission-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    let register = decode_request(br#"{"schema":"gogoke.37.operations.v1","family":"K-INSTANCE","operation":"register","requestId":"regA","targetId":"instanceA","domainId":"global","expectedRevision":"0","payload":{"driverId":"codex"}}"#).unwrap();
    assert_eq!(h::decode_receipt(&product.dispatch_user_request(&register).unwrap()).unwrap().status,
        V37Status::Applied, "actual fixed CLI must be registered by User ingress");
    let install = decode_request(br#"{"schema":"gogoke.37.operations.v1","family":"K-INSTANCE","operation":"install-state","requestId":"installA","targetId":"instanceA","domainId":"global","expectedRevision":"1","payload":{}}"#).unwrap();
    let fact = h::decode_receipt(&product.dispatch_user_request(&install).unwrap()).unwrap();
    assert_eq!((fact.status, fact.previous_revision, fact.revision), (V37Status::Applied, 1, 1));
    // Capacity behavior arranges login presence only. Current installation is
    // proven by the real catalog; the F read leaves stored UNKNOWN untouched.
    instance::record_observation(&mut product.connection, &root, &instance::ObservationRequest {
        request_id: "loginA", request_bytes: b"loginA", instance_id: "instanceA",
        expected_revision: 1, observation: instance::InstanceObservation::LoggedIn,
    }).unwrap();
    seat::store_template(&mut product.connection, NativeOrigin::user(&product.owner), StoreTemplate {
        domain_id: "projectA", template_id: "templateA", settings_json: b"{}",
    }).unwrap();
    for name in ["seatA", "seatB", "seatC"] {
        seat::create(&mut product.connection, NativeOrigin::user(&product.owner), CreateSeat {
            domain_id: "projectA", seat_id: name, template_id: "templateA", instance_id: Some("instanceA"),
            kind: Kind::Long, request_id: name, request_bytes: name.as_bytes(),
        }).unwrap();
    }
    let reserve_a = request("admission-reserve", "reserveA", "sessionA", "seatA", 0);
    assert_eq!(status(&mut product, &reserve_a), V37Status::Denied);
    seat::set_project_parallel_cap(&mut product.connection, &product.owner, "projectA", 1).unwrap();
    assert_eq!(status(&mut product, &reserve_a), V37Status::Denied);
    instance::set_instance_concurrency_cap(&mut product.connection, &product.owner, "instanceA", 2).unwrap();
    let stored_pin = Statement::prepare(product.connection.as_ptr(),
        "SELECT program_digest FROM gogoke_v37_instances WHERE instance_id='instanceA'").unwrap();
    assert!(stored_pin.step_row().unwrap());
    let actual_digest = stored_pin.column_text(0).unwrap();
    drop(stored_pin);
    product.connection.execute("UPDATE gogoke_v37_instances SET driver_id='missing-provider' WHERE instance_id='instanceA'").unwrap();
    let missing = product.dispatch_user_request(&request("admission-reserve", "probeMissing", "sessionMissing", "seatA", 0)).unwrap();
    assert_eq!(h::decode_receipt(&missing).unwrap().status,
        V37Status::Unknown, "a missing native catalog driver cannot reserve");
    assert!(String::from_utf8_lossy(&missing).contains("UnknownDriver"));
    product.connection.execute("UPDATE gogoke_v37_instances SET driver_id='codex' WHERE instance_id='instanceA'").unwrap();
    product.connection.execute(&format!("UPDATE gogoke_v37_instances SET program_digest='sha256:{}' WHERE instance_id='instanceA'", "0".repeat(64))).unwrap();
    let changed = product.dispatch_user_request(&request("admission-reserve", "probeChanged", "sessionChanged", "seatA", 0)).unwrap();
    assert_eq!(h::decode_receipt(&changed).unwrap().status,
        V37Status::Unknown, "changed CLI pin cannot reserve");
    assert!(String::from_utf8_lossy(&changed).contains("IdentityChanged"));
    product.connection.execute(&format!("UPDATE gogoke_v37_instances SET program_digest='{actual_digest}' WHERE instance_id='instanceA'")).unwrap();
    assert_eq!(seat::get(&product.connection, "projectA", "seatA").unwrap().unwrap().state, State::Idle);
    assert_eq!(status(&mut product, &reserve_a), V37Status::Applied);
    assert_eq!(status(&mut product, &reserve_a), V37Status::Replayed);
    let reserve_b = request("admission-reserve", "reserveB", "sessionB", "seatB", 0);
    assert_eq!(status(&mut product, &reserve_b), V37Status::Denied);
    assert_eq!(seat::get(&product.connection, "projectA", "seatB").unwrap().unwrap().state, State::Idle);
    seat::set_project_parallel_cap(&mut product.connection, &product.owner, "projectA", 3).unwrap();
    assert_eq!(status(&mut product, &reserve_b), V37Status::Applied);
    let reserve_c = request("admission-reserve", "reserveC", "sessionC", "seatC", 0);
    assert_eq!(status(&mut product, &reserve_c), V37Status::Denied);
    assert_eq!(seat::get(&product.connection, "projectA", "seatC").unwrap().unwrap().state, State::Idle);
    assert_eq!(status(&mut product, &request("admission-release", "releaseA", "sessionA", "seatA", 1)), V37Status::Applied);
    assert_eq!(seat::get(&product.connection, "projectA", "seatA").unwrap().unwrap().state, State::Idle);
    assert_eq!(status(&mut product, &reserve_c), V37Status::Applied);
    // A peer or caller cannot select another same-instance, same-generation
    // seat to commit this session's reservation.
    assert_eq!(status(&mut product, &request("admission-commit", "wrongSeat", "sessionB", "seatC", 1)), V37Status::Conflict);
    product.close_checked().unwrap();
    let mut reopened = ProductDatabase::open(&root, &database).unwrap();
    assert_eq!(seat::read_project_parallel_cap(&reopened.connection, "projectA").unwrap(), 3);
    assert_eq!(instance::read_instance_concurrency_cap(&reopened.connection, "instanceA").unwrap(), 2);
    assert_eq!(status(&mut reopened, &request("admission-commit", "commitB", "sessionB", "seatB", 1)), V37Status::Applied);
    reopened.close_checked().unwrap();
    drop(root);
    // The paths below are all inside this test-created fixture root.
    std::fs::remove_dir_all(path).unwrap();
}
