//! Composed native controls: actual pinned File backend, grants, H opens,
//! kill-on-close retirement, and the original cold recovery call. The only
//! key is an invalid public marker in this test-created root; no model runs
//! and no helper reads credential data. These controls are not Owner evidence.
use super::*;
use crate::process::{CredentialAliasScope, CredentialBinding, NativeCredentialAclRecoveryStep,
    NativeProcessHoldersGone, holder_gone_acl_write_count_for_test};
use crate::root::{RootIdentity, inspect_root};
use crate::store::instance::holder_disappearance::{self as gone, HolderDisappearancePhase};
use crate::store::same_open::route_b_test_guard;
use crate::store::seat::{self, CreateSeat, Kind, NativeOrigin, StoreTemplate};
use crate::store::session_transport as h;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const INSTANCE: &str = "instanceA";
const RIGHTS: u32 = 0x0012_019f;
const SCOPES: [(&str, &str, &str, &str); 2] = [
    ("projectA", "seatA", "treeA", "sessionA"),
    ("projectB", "seatB", "treeB", "sessionB"),
];

fn operation(domain: &str, family: &str, verb: &str, id: &str, target: &str,
    revision: u64, payload: &str) -> V37Request {
    decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"{family}","operation":"{verb}","requestId":"{id}","targetId":"{target}","domainId":"{domain}","expectedRevision":"{revision}","payload":{payload}}}"#).as_bytes()).unwrap()
}
fn applied(product: &mut ProductDatabase<'_>, request: &V37Request) {
    let raw = product.dispatch_user_request(request).expect("original native product dispatch");
    let receipt = h::decode_receipt(&raw).unwrap();
    assert_eq!(receipt.status, V37Status::Applied, "original refusal: {}", String::from_utf8_lossy(&raw));
}
fn rows(product: &ProductDatabase<'_>, sql: &str, params: &[&str], columns: usize) -> Vec<Vec<String>> {
    let q = Statement::prepare(product.connection.as_ptr(), sql).unwrap();
    for (n, value) in params.iter().enumerate() { q.bind_text((n + 1) as i32, value).unwrap(); }
    let mut result = Vec::new();
    while q.step_row().unwrap() {
        result.push((0..columns).map(|n| q.column_text(n as i32).unwrap()).collect());
    }
    result
}
fn custody_rows(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    rows(product, "SELECT operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,
        binary_digest_sha256,profile_id,domain_id,generation,state,COALESCE(stop_proof_hash,''),
        CAST(stop_proof_hash IS NULL AS TEXT) FROM main.gogoke_coordination_process_custody ORDER BY operation_id", &[], 13)
}
fn episode_rows(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    rows(product, "SELECT domain_id,request_id,session_id,generation,COALESCE(old_generation,''),raw_hex,
        CAST(previous_revision AS TEXT),COALESCE(CAST(result_revision AS TEXT),''),
        COALESCE(process_operation_id,''),instance_id,home_id,binding_id,COALESCE(seat_id,''),
        COALESCE(seat_incarnation,''),COALESCE(stop_request_id,''),phase,COALESCE(stop_fact_id,''),
        CAST(stop_fact_id IS NULL AS TEXT) FROM main.gogoke_v37_h_process_episode ORDER BY domain_id,request_id", &[], 18)
}
fn claim_rows(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    rows(product, "SELECT domain_id,session_id,instance_id,home_id,binding_id,generation,state,
        CAST(revision AS TEXT),COALESCE(process_operation_id,''),COALESCE(stop_fact_id,''),
        CAST(stop_fact_id IS NULL AS TEXT) FROM main.gogoke_v37_h_claim ORDER BY domain_id,session_id", &[], 11)
}
fn journal_rows(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    if rows(product, "SELECT 1 FROM main.sqlite_schema WHERE type='table' AND name='gogoke_v37_holder_disappearance'", &[], 1).is_empty() {
        return Vec::new();
    }
    rows(product, "SELECT binding_id,instance_id,process_operation_id,request_id,pid,creation_time_100ns,
        snapshot_hex,snapshot_digest,phase,CAST(revision AS TEXT)
        FROM main.gogoke_v37_holder_disappearance ORDER BY binding_id", &[], 10)
}
fn resource_snapshot(product: &ProductDatabase<'_>) -> Vec<Vec<Vec<String>>> {
    vec![custody_rows(product), episode_rows(product), claim_rows(product), journal_rows(product),
        rows(product, "SELECT binding_id,instance_id,history_id,generation,profile_sid,source_file_identity,
            intent_request,state,CAST(revision AS TEXT) FROM main.gogoke_v37_credential_profiles ORDER BY binding_id", &[], 9),
        rows(product, "SELECT history_id,instance_id,directory_identity,source_file_identity,intent_request,state,
            CAST(revision AS TEXT) FROM main.gogoke_v37_credential_aliases ORDER BY history_id", &[], 7),
        rows(product, "SELECT domain_id,seat_id,incarnation,state,CAST(generation AS TEXT),CAST(revision AS TEXT)
            FROM main.gogoke_v37_seats ORDER BY domain_id,seat_id", &[], 6),
        rows(product, "SELECT domain_id,request_id,raw_hex,operation,session_id,status,
            CAST(previous_revision AS TEXT),CAST(revision AS TEXT)
            FROM main.gogoke_v37_h_operation ORDER BY domain_id,request_id", &[], 8),
        rows(product, "SELECT instance_id,root_identity,home_identity,file_identity,source_parent_identity,
            phase,CAST(revision AS TEXT) FROM main.gogoke_v37_credential_objects ORDER BY instance_id", &[], 7),
        rows(product, "SELECT request_id,request_hex,target_id,phase,COALESCE(receipt_json,''),COALESCE(native_receipt_id,'')
            FROM main.gogoke_v37_instance_operations ORDER BY request_id", &[], 6),
        rows(product, "SELECT history_id,instance_id,root_identity,parent_identity,home_identity,domain_id,session_id,
            seat_id,seat_incarnation,initial_binding_id,initial_generation,initial_request_id,directory_ref,
            COALESCE(directory_identity,''),state,CAST(revision AS TEXT)
            FROM main.gogoke_v37_instance_histories ORDER BY history_id", &[], 16),
        rows(product, "SELECT binding_id,history_id,generation,request_id,COALESCE(predecessor_binding_id,''),
            COALESCE(process_operation_id,''),COALESCE(ticket,''),COALESCE(custodian_nonce,'')
            FROM main.gogoke_v37_instance_history_generations ORDER BY binding_id", &[], 8)]
}

// Same fixed CLI File qualification as v37_runtime_tests. It is kept local to
// this new test module so existing fixture visibility/production stay owned
// by Controller. The synthetic login child has its own genuine durable stop.
fn qualify_file_backend(product: &mut ProductDatabase<'_>, root: &RootLock) {
    use crate::process::{DurableStopConfirmation, StopBudgets};
    use std::os::windows::fs::MetadataExt;
    let home = instance::resolve_codex_instance_home(&product.connection, root, INSTANCE).unwrap();
    let mut scope = product.prepare_owner_codex_login(INSTANCE).unwrap();
    assert!(scope.credential_custody.is_none());
    scope.login.launch.arguments = vec!["login".into(), "--with-api-key".into()];
    let prepared = product.process_custodian.prepare(&scope.login).unwrap();
    let operation_id = "holder-gone-synthetic-cli-file-login";
    authority::record_prepared_process(&mut product.connection, operation_id, &prepared).unwrap();
    product.process_custodian.activate(&prepared).unwrap();
    authority::mark_process_active(&mut product.connection, operation_id, &prepared).unwrap();
    product.process_custodian.active(&prepared.ticket).unwrap()
        .write_persistent_frame(b"gogoke-synthetic-invalid-key-no-auth\n").unwrap();
    product.process_custodian.close_child_input(&prepared.ticket).unwrap();
    assert!(product.process_custodian.active(&prepared.ticket).unwrap().wait(Duration::from_secs(15)).unwrap());
    let proof = product.process_custodian.stop(&prepared.ticket, StopBudgets::production(), || Ok(())).unwrap();
    assert_eq!(proof.exit_code, Some(0), "original synthetic CLI stderr: {}",
        product.process_custodian.active(&prepared.ticket).unwrap().stderr_tail());
    let revision = authority::mark_process_stopped(&mut product.connection, operation_id, &proof).unwrap();
    product.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
        ticket: prepared.ticket.clone(), custodian_nonce: prepared.custodian_nonce.clone(),
        identity: prepared.identity.clone(), proof_hash: proof.proof_hash(), durable_revision: revision,
    }).unwrap();
    assert_eq!(inspect_root(&scope.runtime_home).unwrap().identity, scope.runtime_identity);
    fn physical_runtime(path: &Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let child = entry.unwrap().path(); let metadata = std::fs::symlink_metadata(&child).unwrap();
            assert_eq!(metadata.file_attributes() & 0x400, 0);
            if metadata.is_dir() { physical_runtime(&child); }
        }
    }
    physical_runtime(&scope.runtime_home);
    assert!(std::fs::canonicalize(&scope.runtime_home).unwrap().starts_with(root.canonical_root().canonical_path.clone()));
    std::fs::remove_dir_all(&scope.runtime_home).unwrap();
    let original = CredentialBinding::observe_source_metadata(root, &home.path.join("auth.json"), &home.identity).unwrap();
    assert_eq!(original.1, 1);
    for (id, revision, backend) in [("holder-account", 1, false), ("holder-file-bound", 2, true)] {
        let request = operation("global", "K-INSTANCE", "login-state", id, INSTANCE, revision, "{}");
        let raw = if backend { product.dispatch_owner_file_backend_observation(&request) }
            else { product.dispatch_owner_login_observation(&request) }.unwrap();
        assert!(String::from_utf8_lossy(&raw).contains("\"state\":\"LOGGED_IN\""),
            "presence of the synthetic marker, not valid authentication: {}", String::from_utf8_lossy(&raw));
    }
    let backend = instance::read_usable_credential_backend(&product.connection, INSTANCE).unwrap();
    assert_eq!(backend.backend, instance::CredentialBackend::File);
    assert_eq!(backend.startup_selector, instance::CredentialStartupSelector::FileBound);
    assert_eq!(CredentialBinding::observe_source_metadata(root, &home.path.join("auth.json"), &home.identity).unwrap(), original);
}

struct ColdState {
    database: PathBuf, home: PathBuf, home_identity: RootIdentity, source: RootIdentity,
    profiles: Vec<instance::CredentialProfileRecord>,
    custody: Vec<Vec<String>>, episodes: Vec<Vec<String>>, claims: Vec<Vec<String>>,
    aliases: Vec<Vec<String>>, pairs: Vec<(u32, u64)>,
}
fn source_acl_digest(product: &ProductDatabase<'_>, root: &RootLock, cold: &ColdState) -> String {
    let aliases = instance::read_credential_aliases(&product.connection, INSTANCE).unwrap();
    let scopes: Vec<_> = aliases.iter().filter(|a| a.state != "REMOVED").map(|a| {
        let directory = instance::resolve_private_history_directory(&product.connection, root, &a.history_id).unwrap();
        assert_eq!(directory.identity, a.directory_identity);
        CredentialAliasScope { root: directory.path, root_identity: directory.identity }
    }).collect();
    let binding = CredentialBinding::open_registered(root, &cold.home.join("auth.json"),
        &cold.home_identity, &cold.source, &scopes).unwrap();
    binding.verify_registered_aliases(&scopes).unwrap();
    let known: Vec<_> = cold.profiles.iter().map(|p| (p.profile_sid.clone(), RIGHTS)).collect();
    NativeCredentialAclRecoveryStep::capture(&binding, &cold.profiles[0].profile_sid, &known).unwrap().before_digest()
}

fn pin_parent_metadata(pid: u32, original_creation: u64) -> std::os::windows::io::OwnedHandle {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    #[repr(C)] struct FileTime { low: u32, high: u32 }
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn GetProcessTimes(process: *mut std::ffi::c_void, creation: *mut FileTime,
            exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    }
    let raw = unsafe { OpenProcess(0x0010_1000, 0, pid) }; // limited query + synchronize, no Job handle
    assert!(!raw.is_null(), "original parent metadata handle: {}", std::io::Error::last_os_error());
    let held = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut creation = FileTime { low: 0, high: 0 }; let mut exit = FileTime { low: 0, high: 0 };
    let mut kernel = FileTime { low: 0, high: 0 }; let mut user = FileTime { low: 0, high: 0 };
    assert_ne!(unsafe { GetProcessTimes(held.as_raw_handle(), &mut creation, &mut exit, &mut kernel, &mut user) }, 0);
    assert_eq!((u64::from(creation.high) << 32) | u64::from(creation.low), original_creation);
    held
}
fn wait_actual_parent_exit(parent: &std::os::windows::io::OwnedHandle, pair: (u32, u64)) {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" { fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32; }
    let at_close = unsafe { WaitForSingleObject(parent.as_raw_handle(), 0) };
    assert!(matches!(at_close, 0 | 258), "original Wait(0) error: {}", std::io::Error::last_os_error());
    let exit = if at_close == 0 { at_close } else { unsafe { WaitForSingleObject(parent.as_raw_handle(), 15_000) } };
    eprintln!("holder_fixture_exit pid={} creation={} after_close_wait0={} bounded_exit_wait={}", pair.0, pair.1, at_close, exit);
    assert_eq!(exit, 0,
        "actual parent exit after Job close, not a synthetic StopFact");
}

fn cold_two_grants(run: impl for<'a> FnOnce(ProductDatabase<'a>, &'a RootLock, &ColdState)) {
    let _guard = route_b_test_guard();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-holder-composed-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    applied(&mut product, &operation("global", "K-INSTANCE", "register", "holder-register", INSTANCE, 0,
        r#"{"driverId":"codex"}"#));
    qualify_file_backend(&mut product, &root);
    instance::set_instance_concurrency_cap(&mut product.connection, &product.owner, INSTANCE, 2).unwrap();
    let source = crate::store::worktree::tests::make_source_fixture(&mut product.connection, &root,
        &product.owner, &mut product.process_custodian);
    let git = std::env::var_os("GOGOKE_CONTROLLED_GIT_PATH").expect("original cloud Git pin");
    let configuration = Json::Object(BTreeMap::from([
        (JsonString::from_str("schema"), Json::String(JsonString::from_str("gogoke.37.owner-configuration.v1"))),
        (JsonString::from_str("command"), Json::String(JsonString::from_str("worktree-source"))),
        (JsonString::from_str("repositoryId"), Json::String(JsonString::from_str("fixtureRepo"))),
        (JsonString::from_str("sourcePath"), Json::String(JsonString::from_str(source.to_str().unwrap()))),
        (JsonString::from_str("gitPath"), Json::String(JsonString::from_str(git.to_str().unwrap()))),
    ])).canonical();
    product.configure_user_v37(configuration.as_bytes()).unwrap();
    let mut pairs = Vec::new(); let mut parent_metadata = Vec::new();
    for (domain, seat_id, tree_id, session_id) in SCOPES {
        seat::set_project_parallel_cap(&mut product.connection, &product.owner, domain, 1).unwrap();
        seat::store_template(&mut product.connection, NativeOrigin::user(&product.owner), StoreTemplate {
            domain_id: domain, template_id: "templateA",
            settings_json: br#"{"effort":"high","model":"gpt-6-sol","permissionTier":"NETWORKED_WRITE"}"#,
        }).unwrap();
        seat::create(&mut product.connection, NativeOrigin::user(&product.owner), CreateSeat {
            domain_id: domain, seat_id, template_id: "templateA", instance_id: Some(INSTANCE), kind: Kind::Long,
            request_id: &format!("holder-create-{seat_id}"), request_bytes: format!("holder seat {seat_id}").as_bytes(),
        }).unwrap();
        applied(&mut product, &operation(domain, "K-WORKTREE", "create", &format!("holder-tree-{tree_id}"), tree_id, 0,
            &format!(r#"{{"repositoryId":"fixtureRepo","seatId":"{seat_id}"}}"#)));
        let generation = seat::get(&product.connection, domain, seat_id).unwrap().unwrap().generation + 1;
        for (verb, revision) in [("admission-reserve", 0), ("admission-commit", 1)] {
            applied(&mut product, &operation(domain, "K-SESSION", verb, &format!("holder-{verb}-{session_id}"),
                session_id, revision, &format!(r#"{{"seatId":"{seat_id}","generation":"{generation}"}}"#)));
        }
        applied(&mut product, &operation(domain, "K-SESSION", "open", &format!("holder-open-{session_id}"), session_id, 2,
            &format!(r#"{{"seatId":"{seat_id}","generation":"{generation}","repositoryId":"fixtureRepo","worktreeId":"{tree_id}"}}"#)));
        let session = product.native_sessions.get(&(domain.into(), session_id.into())).unwrap();
        assert!(session.evidence.file_credentials_bound());
        assert!(session.thread_id.is_some(), "actual fixed CLI thread/start reply");
        pairs.push((session.custody.identity.pid, session.custody.identity.creation_time_100ns));
        parent_metadata.push(pin_parent_metadata(session.custody.identity.pid, session.custody.identity.creation_time_100ns));
    }
    assert_eq!(product.native_sessions.len(), 2);
    let home = instance::resolve_codex_instance_home(&product.connection, &root, INSTANCE).unwrap();
    let (source, links) = CredentialBinding::observe_source_metadata(&root, &home.path.join("auth.json"), &home.identity).unwrap();
    assert_eq!(links, 3, "one original source and two genuine registered aliases");
    let profiles = instance::read_credential_profiles(&product.connection, INSTANCE).unwrap();
    assert_eq!(profiles.len(), 2);
    assert!(profiles.iter().all(|p| p.state == "ACTIVE" && p.source_file_identity == source));
    assert_ne!(profiles[0].profile_sid, profiles[1].profile_sid);
    let live_custody = custody_rows(&product);
    for &(pid, creation) in &pairs {
        let row = live_custody.iter().find(|r| r[3] == pid.to_string() && r[4] == creation.to_string()).unwrap();
        assert_eq!(row[10], "ACTIVE"); assert_eq!(row[12], "1");
        assert!(NativeProcessHoldersGone::observe(&[(pid, creation)]).is_err(), "actual original child still alive");
    }
    let claims = claim_rows(&product);
    assert_eq!(claims.len(), 2);
    assert!(claims.iter().all(|r| r[6] == "COMMITTED" && !r[8].is_empty() && r[10] == "1"));
    assert!(journal_rows(&product).is_empty());
    // This is the genuine host-close path. It closes the old Jobs and drops
    // native handles without invoking User stop or fabricating any StopFact.
    product.close_checked().unwrap();
    for (parent, pair) in parent_metadata.iter().zip(&pairs) { wait_actual_parent_exit(parent, *pair); }
    let product = ProductDatabase::open(&root, &database).unwrap();
    assert!(product.native_sessions.is_empty());
    NativeProcessHoldersGone::observe(&pairs).expect("actual old exact identities after Job close").validate(&pairs).unwrap();
    let custody = custody_rows(&product);
    assert_eq!(custody.len(), live_custody.len());
    for (old, current) in live_custody.iter().zip(&custody) {
        let mut expected = old.clone();
        if matches!(expected[10].as_str(), "ACTIVE" | "PREPARED") { expected[10] = "UNKNOWN".into(); }
        assert_eq!(current, &expected, "only the existing cold initializer's conservative lifecycle change");
    }
    assert_eq!(claim_rows(&product), claims, "cold host did not release claims or synthesize STOPPED");
    assert_eq!(instance::read_credential_profiles(&product.connection, INSTANCE).unwrap(), profiles);
    let cold = ColdState { database, home: home.path, home_identity: home.identity, source, profiles,
        custody, episodes: episode_rows(&product), claims,
        aliases: rows(&product, "SELECT history_id,instance_id,directory_identity,source_file_identity,intent_request,state,
            CAST(revision AS TEXT) FROM main.gogoke_v37_credential_aliases ORDER BY history_id", &[], 7), pairs };
    run(product, &root, &cold);
    drop(parent_metadata);
    drop(root);
    let actual = std::fs::canonicalize(&path).unwrap();
    assert!(actual.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
    assert!(actual.file_name().unwrap().to_string_lossy().starts_with("gogoke-holder-composed-"));
    std::fs::remove_dir_all(actual).unwrap();
}

#[test]
fn actual_two_disappeared_holders_recover_in_one_call_replay_without_acl_effect_and_admit_cold_source() {
    cold_two_grants(|mut product, root, cold| {
        let before_acl = source_acl_digest(&product, root, cold);
        let writes = holder_gone_acl_write_count_for_test();
        // Do not retry an Err: this call must itself finish both original rows.
        product.recover_disappeared_credential_resources(INSTANCE, None)
            .expect("FIRST composed recovery must revoke both grants and release both H claims");
        assert_eq!(holder_gone_acl_write_count_for_test() - writes, 2, "two actual captured SID removals");
        assert_eq!(custody_rows(&product), cold.custody, "all original custody and STOPPED receipts preserved byte-for-byte");
        assert_eq!(episode_rows(&product), cold.episodes, "no H episode STOPPED or stop_request fabricated");
        assert_eq!(rows(&product, "SELECT history_id,instance_id,directory_identity,source_file_identity,intent_request,state,
            CAST(revision AS TEXT) FROM main.gogoke_v37_credential_aliases ORDER BY history_id", &[], 7), cold.aliases);
        let journals = journal_rows(&product);
        assert_eq!(journals.len(), 2);
        for original in &cold.profiles {
            let record = gone::read_holder_disappearance(&product.connection, &original.binding_id).unwrap().unwrap();
            assert_eq!(record.phase, HolderDisappearancePhase::Applied); assert_eq!(record.revision, 3);
            let generation = instance::read_private_history_generation(&product.connection,
                &original.binding_id, &original.generation).unwrap().unwrap();
            let source = generation.source.unwrap();
            assert_eq!(record.input.process_operation_id, source.process_operation_id);
            assert_eq!(record.input.instance_id, original.instance_id);
            let exact = cold.custody.iter().find(|r| r[0] == source.process_operation_id).unwrap();
            assert_eq!((record.input.pid.as_str(), record.input.creation_time_100ns.as_str()),
                (exact[3].as_str(), exact[4].as_str()));
            let revoked = instance::read_completed_profile_revoke(&product.connection, &instance::CredentialProfileIntent {
                request_id: format!("{}-revoke", record.input.request_id), instance_id: original.instance_id.clone(),
                history_id: original.history_id.clone(), binding_id: original.binding_id.clone(), generation: original.generation.clone(),
                profile_sid: original.profile_sid.clone(), source_file_identity: original.source_file_identity.clone(),
                expected_revision: original.revision, action: instance::CredentialProfileAction::Revoke,
            }).expect("actual completed F revoke receipt, not state-only evidence");
            assert_eq!(revoked.state, "REVOKED");
            let old = cold.claims.iter().find(|r| r[0] == generation.domain_id && r[1] == generation.session_id).unwrap();
            let mut expected = old.clone(); expected[6] = "RELEASED".into();
            expected[7] = (old[7].parse::<u64>().unwrap() + 1).to_string();
            assert_eq!(claim_rows(&product).into_iter().find(|r| r[0] == old[0] && r[1] == old[1]).unwrap(), expected);
            let seat = seat::get(&product.connection, &generation.domain_id, &generation.seat_id).unwrap().unwrap();
            assert_eq!(seat.state, seat::State::Idle);
            assert_eq!(seat.incarnation, generation.seat_incarnation);
            assert_eq!(seat.generation.to_string(), (generation.generation.parse::<u64>().unwrap() + 1).to_string());
            let release = rows(&product, "SELECT raw_hex,operation,session_id,status,CAST(previous_revision AS TEXT),CAST(revision AS TEXT)
                FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2",
                &[&generation.domain_id, &record.input.request_id], 6);
            assert_eq!(release, vec![vec![record.input.snapshot_hex.clone(), "holder-gone-release".into(),
                generation.session_id, "APPLIED".into(), old[7].clone(), expected[7].clone()]]);
        }
        assert_ne!(source_acl_digest(&product, root, cold), before_acl);
        assert_eq!(CredentialBinding::observe_source_metadata(root, &cold.home.join("auth.json"),
            &cold.home_identity).unwrap(), (cold.source.clone(), 3));
        let after = resource_snapshot(&product); let after_acl = source_acl_digest(&product, root, cold);
        let replay_writes = holder_gone_acl_write_count_for_test();
        product.recover_disappeared_credential_resources(INSTANCE, None).expect("same original completed replay");
        assert_eq!(holder_gone_acl_write_count_for_test(), replay_writes, "replay never calls actual ACL writer");
        assert_eq!(resource_snapshot(&product), after);
        assert_eq!(source_acl_digest(&product, root, cold), after_acl);
        product.close_checked().unwrap();
        let mut product = ProductDatabase::open(root, &cold.database).unwrap();
        product.recover_disappeared_credential_resources(INSTANCE, None).expect("persistent completed receipts in a new cold holder");
        assert_eq!(holder_gone_acl_write_count_for_test(), replay_writes, "cold replay never calls actual ACL writer");
        assert_eq!(resource_snapshot(&product), after);
        assert_eq!(source_acl_digest(&product, root, cold), after_acl);
        let generation = seat::get(&product.connection, "projectA", "seatA").unwrap().unwrap().generation + 1;
        for (verb, revision) in [("admission-reserve", 0), ("admission-commit", 1)] {
            applied(&mut product, &operation("projectA", "K-SESSION", verb, &format!("holder-cold-{verb}"),
                "sessionC", revision, &format!(r#"{{"seatId":"seatA","generation":"{generation}"}}"#)));
        }
        applied(&mut product, &operation("projectA", "K-SESSION", "open", "holder-cold-open", "sessionC", 2,
            &format!(r#"{{"seatId":"seatA","generation":"{generation}","repositoryId":"fixtureRepo","worktreeId":"treeA"}}"#)));
        assert!(product.native_sessions.get(&("projectA".into(), "sessionC".into())).unwrap().evidence.file_credentials_bound());
        assert!(instance::read_credential_profiles(&product.connection, INSTANCE).unwrap().iter()
            .any(|p| p.state == "ACTIVE" && p.source_file_identity == cold.source && p.generation == generation.to_string()));
        for old in &cold.custody {
            assert_eq!(custody_rows(&product).into_iter().find(|r| r[0] == old[0]).unwrap(), *old);
        }
        assert_eq!(journal_rows(&product), journals, "new admission does not rewrite old recovery capture");
        assert_eq!(CredentialBinding::observe_source_metadata(root, &cold.home.join("auth.json"), &cold.home_identity).unwrap().0, cold.source);
        applied(&mut product, &operation("projectA", "K-SESSION", "stop", "holder-cold-stop", "sessionC", 3,
            &format!(r#"{{"seatId":"seatA","generation":"{generation}"}}"#)));
        applied(&mut product, &operation("projectA", "K-SESSION", "admission-release", "holder-cold-release", "sessionC", 4,
            &format!(r#"{{"seatId":"seatA","generation":"{generation}"}}"#)));
        assert_eq!(journal_rows(&product), journals);
        product.close_checked().unwrap();
    });
}

#[test]
fn composed_disappearance_rejects_actual_live_identity_and_wrong_physical_source_without_resource_effects() {
    cold_two_grants(|mut product, root, cold| {
        let untouched = resource_snapshot(&product);
        #[repr(C)] struct FileTime { low: u32, high: u32 }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn GetProcessTimes(process: *mut std::ffi::c_void, creation: *mut FileTime,
                exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        }
        let mut creation = FileTime { low: 0, high: 0 }; let mut exit = FileTime { low: 0, high: 0 };
        let mut kernel = FileTime { low: 0, high: 0 }; let mut user = FileTime { low: 0, high: 0 };
        assert_ne!(unsafe { GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user) }, 0);
        let live_pid = std::process::id().to_string();
        let live_creation = ((u64::from(creation.high) << 32) | u64::from(creation.low)).to_string();
        let source = instance::read_private_history_generation(&product.connection,
            &cold.profiles[0].binding_id, &cold.profiles[0].generation).unwrap().unwrap().source.unwrap();
        let original = cold.custody.iter().find(|r| r[0] == source.process_operation_id).unwrap();
        // Negative instrument only: replace one metadata pair with this actual
        // still-live test host identity. The retired CLI itself remains gone;
        // this must not be reported as a natural surviving-child observation.
        let update_pair = |product: &ProductDatabase<'_>, pid: &str, created: &str| {
            let q = Statement::prepare(product.connection.as_ptr(),
                "UPDATE main.gogoke_coordination_process_custody SET pid=?1,creation_time_100ns=?2 WHERE operation_id=?3").unwrap();
            for (n, value) in [pid, created, &source.process_operation_id].iter().enumerate() { q.bind_text((n + 1) as i32, value).unwrap(); }
            q.step_done().unwrap();
        };
        update_pair(&product, &live_pid, &live_creation);
        let before = resource_snapshot(&product); let acl = source_acl_digest(&product, root, cold);
        let writes = holder_gone_acl_write_count_for_test();
        let error = product.recover_disappeared_credential_resources(INSTANCE, None).unwrap_err();
        assert!(format!("{error:?}").contains("ExactHolderAlive"), "original refusal: {error:?}");
        assert_eq!(holder_gone_acl_write_count_for_test(), writes);
        assert_eq!(resource_snapshot(&product), before); assert_eq!(source_acl_digest(&product, root, cold), acl);
        update_pair(&product, &original[3], &original[4]);
        assert_eq!(custody_rows(&product), cold.custody);
        NativeProcessHoldersGone::observe(&cold.pairs).unwrap();
        // Wrong-object metadata is a genuine different test-created FileID,
        // never copied/renamed auth data or a guessed opaque identity.
        let unrelated = cold.home.join("holder-wrong-object-public.txt");
        std::fs::write(&unrelated, b"public unrelated object, not credentials").unwrap();
        let wrong = {
            use std::os::windows::fs::OpenOptionsExt;
            use std::os::windows::io::AsRawHandle;
            #[repr(C)] struct FileIdInfo { volume_serial: u64, file_id: [u8; 16] }
            #[link(name = "kernel32")]
            extern "system" {
                fn GetFileInformationByHandleEx(handle: *mut std::ffi::c_void, class: i32,
                    information: *mut std::ffi::c_void, length: u32) -> i32;
            }
            // FileID query uses metadata access only, matching the production
            // class-18 ABI. inspect_root is deliberately directory-only.
            let file = std::fs::OpenOptions::new().access_mode(0x80).share_mode(7)
                .custom_flags(0x0020_0000).open(&unrelated).unwrap();
            let mut info = FileIdInfo { volume_serial: 0, file_id: [0; 16] };
            assert_ne!(unsafe { GetFileInformationByHandleEx(file.as_raw_handle().cast(), 18,
                (&mut info as *mut FileIdInfo).cast(), std::mem::size_of::<FileIdInfo>() as u32) }, 0);
            RootIdentity { volume_serial: info.volume_serial, file_id: info.file_id }
        };
        assert_ne!(wrong, cold.source);
        let update_source = |product: &ProductDatabase<'_>, identity: &RootIdentity| {
            // Keep the F metadata association internally consistent so the
            // negative reaches actual source FileID readback, rather than
            // merely tripping a mismatched alias/profile SQL relationship.
            for sql in [
                "UPDATE main.gogoke_v37_credential_objects SET file_identity=?1 WHERE instance_id=?2",
                "UPDATE main.gogoke_v37_credential_profiles SET source_file_identity=?1 WHERE instance_id=?2",
                "UPDATE main.gogoke_v37_credential_aliases SET source_file_identity=?1 WHERE instance_id=?2",
            ] {
                let q = Statement::prepare(product.connection.as_ptr(), sql).unwrap();
                q.bind_text(1, &identity.opaque()).unwrap(); q.bind_text(2, INSTANCE).unwrap(); q.step_done().unwrap();
            }
        };
        update_source(&product, &wrong);
        let before = resource_snapshot(&product);
        let error = product.recover_disappeared_credential_resources(INSTANCE, None)
            .expect_err("mismatched registered physical source must refuse");
        assert!(format!("{error:?}").contains("IdentityChanged"), "original physical refusal: {error:?}");
        assert_eq!(holder_gone_acl_write_count_for_test(), writes);
        assert_eq!(resource_snapshot(&product), before);
        update_source(&product, &cold.source);
        assert_eq!(source_acl_digest(&product, root, cold), acl);
        assert!(journal_rows(&product).is_empty());
        assert_eq!(custody_rows(&product), cold.custody); assert_eq!(episode_rows(&product), cold.episodes);
        assert_eq!(claim_rows(&product), cold.claims);
        assert_eq!(instance::read_credential_profiles(&product.connection, INSTANCE).unwrap(), cold.profiles);
        assert_eq!(resource_snapshot(&product), untouched, "negative instrumentation restored all original F/H resource rows");
        product.close_checked().unwrap();
    });
}
