//! Cloud security composition, with explicit synthetic facts:
//! `SYNTHETIC_LOGIN_PRESENCE_NOT_AUTHENTICATION` marks instance registry login metadata only.
//! The UNKNOWN-open regression projects the observed persisted failure shape;
//! it does not claim that its successful initialize fixture reproduced a timeout.
//! No account, credential, User model turn or vendor session is invented.
//! E/F/H, fixed Claude initialize ACK, LPAC process identities and ACL effects
//! come from their production producers. This is not installed Owner recovery.
use super::*;
use crate::process::{AppContainerProfile, NativeProcessHoldersGone};
use crate::store::same_open::route_b_test_guard;
use crate::store::seat::{self, CreateSeat, Kind, NativeOrigin, StoreTemplate};
use crate::store::session_transport as h;
use crate::store::worktree;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{SystemTime, UNIX_EPOCH};

const INSTANCE: &str = "instanceA";
const SCOPES: [(&str, &str, &str, &str); 2] = [
    ("projectA", "seatA", "treeA", "sessionA"),
    ("projectB", "seatB", "treeB", "sessionB"),
];

fn operation(
    domain: &str,
    family: &str,
    verb: &str,
    id: &str,
    target: &str,
    revision: u64,
    payload: &str,
) -> V37Request {
    decode_request(
        format!(
            r#"{{"schema":"gogoke.37.operations.v1","family":"{family}",
        "operation":"{verb}","requestId":"{id}","targetId":"{target}","domainId":"{domain}",
        "expectedRevision":"{revision}","payload":{payload}}}"#
        )
        .as_bytes(),
    )
    .unwrap()
}

fn original_opens(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    rows(
        product,
        "SELECT domain_id,request_id,raw_hex,operation,session_id,status
         FROM main.gogoke_v37_h_operation WHERE operation='open'
         AND domain_id IN ('projectA','projectB') ORDER BY domain_id,request_id",
        &[],
        6,
    )
}

#[test]
fn claude_holder_unknown_open_keeps_originals_and_rejects_captured_status_drift() {
    cold_two_claude(|mut product, _root, cold| {
        // Synthetic persisted failure shape, not a fabricated authentication,
        // initialize failure or installed recovery result. Exact original H/E/F,
        // native process disappearance and ACLs still come from their producers.
        product.connection.execute(
            "UPDATE main.gogoke_v37_h_operation SET status='UNKNOWN'
             WHERE operation='open' AND domain_id IN ('projectA','projectB');
             UPDATE main.gogoke_v37_h_process_episode SET phase='PREPARED'
             WHERE instance_id='instanceA' AND domain_id IN ('projectA','projectB')",
        ).unwrap();
        let opens_before = original_opens(&product);
        let custody_before = custody(&product);
        let episodes_before = episodes(&product);
        let rpc_before = rpc(&product);
        assert_eq!(opens_before.len(), 2);
        assert!(opens_before.iter().all(|row| row[5] == "UNKNOWN"));
        assert!(custody_before.iter().all(|row| row[10] == "UNKNOWN" && row[12] == "1"));
        assert!(episodes_before.iter().all(|row| row[15] == "PREPARED" && row[16] == "NULL"));
        NativeProcessHoldersGone::observe(&cold.pairs).unwrap().validate(&cold.pairs).unwrap();
        assert_eq!(occupancy(&mut product), None);
        product.connection.execute(
            "CREATE TEMP TRIGGER claude_holder_unknown_second_release_cut
             BEFORE INSERT ON main.gogoke_v37_h_operation
             WHEN NEW.operation='claude-holder-gone-release' AND NEW.session_id='sessionB'
             BEGIN SELECT RAISE(FAIL,'controlled UNKNOWN second H release cut'); END",
        ).unwrap();
        let cut = product.recover_disappeared_claude_resources(INSTANCE, None).unwrap_err();
        assert!(format!("{cut:?}").contains("controlled UNKNOWN second H release cut"), "{cut:?}");
        let partial = journal(&product);
        assert_eq!(partial.len(), 2);
        assert_eq!(partial[0][7], "APPLIED");
        assert_eq!(partial[1][7], "PREPARED");
        let partial_claims = claims(&product);
        assert_eq!(partial_claims[0][6], "RELEASED");
        assert_eq!(partial_claims[1][6], "COMMITTED");
        assert!(partial_claims.iter().all(|row| row[10] == "1"));
        assert_eq!(occupancy(&mut product), None);
        assert_eq!(original_opens(&product), opens_before);
        assert_eq!(custody(&product), custody_before);
        assert_eq!(episodes(&product), episodes_before);
        assert_eq!(rpc(&product), rpc_before);

        product.connection.execute(
            "DROP TRIGGER temp.claude_holder_unknown_second_release_cut",
        ).unwrap();
        product.recover_disappeared_claude_resources(INSTANCE, None).unwrap();
        let completed = journal(&product);
        assert!(completed.iter().all(|row| row[7] == "APPLIED"));
        let mut expected = cold.claims.clone();
        for claim in &mut expected {
            claim[6] = "RELEASED".into();
            claim[7] = (claim[7].parse::<u64>().unwrap() + 1).to_string();
        }
        assert_eq!(claims(&product), expected, "independent release keeps StopFact NULL");
        for (domain, seat_id, _, _) in SCOPES {
            assert_eq!(seat::get(&product.connection, domain, seat_id).unwrap().unwrap().state, seat::State::Idle);
        }
        assert_eq!(occupancy(&mut product), Some(0));
        // The completed receipt still compares the captured original status.
        // Drift during a pending intent instead fences it UNKNOWN permanently;
        // do not invent a reset-and-resume path for that different case.
        product.connection.execute(
            "UPDATE main.gogoke_v37_h_operation SET status='APPLIED'
             WHERE domain_id='projectB' AND operation='open' AND session_id='sessionB'",
        ).unwrap();
        let drift = product.recover_disappeared_claude_resources(INSTANCE, None).unwrap_err();
        assert!(format!("{drift:?}").contains("Claude prior disappeared holder release unverified"), "{drift:?}");
        assert_eq!(claims(&product), expected);
        assert_eq!(journal(&product), completed);
        assert_eq!(custody(&product), custody_before);
        assert_eq!(episodes(&product), episodes_before);
        assert_eq!(rpc(&product), rpc_before);
        product.connection.execute(
            "UPDATE main.gogoke_v37_h_operation SET status='UNKNOWN'
             WHERE domain_id='projectB' AND operation='open' AND session_id='sessionB'",
        ).unwrap();
        product.recover_disappeared_claude_resources(INSTANCE, None).unwrap();
        assert_eq!(journal(&product), completed);
        assert_eq!(original_opens(&product), opens_before, "UNKNOWN is never rewritten to successful open");
        assert_eq!(custody(&product), custody_before, "no invented STOPPED or StopFact");
        assert_eq!(episodes(&product), episodes_before);
        assert_eq!(rpc(&product), rpc_before);
        product.close_checked().unwrap();
    });
}
fn applied(product: &mut ProductDatabase<'_>, request: &V37Request) {
    let raw = product
        .dispatch_user_request(request)
        .expect("original native User producer");
    let receipt = h::decode_receipt(&raw).unwrap();
    assert_eq!(
        receipt.status,
        V37Status::Applied,
        "original request: {}",
        String::from_utf8_lossy(&raw)
    );
}
fn rows(
    product: &ProductDatabase<'_>,
    sql: &str,
    params: &[&str],
    columns: usize,
) -> Vec<Vec<String>> {
    let q = Statement::prepare(product.connection.as_ptr(), sql).unwrap();
    for (i, value) in params.iter().enumerate() {
        q.bind_text(i as i32 + 1, value).unwrap();
    }
    let mut found = Vec::new();
    while q.step_row().unwrap() {
        found.push(
            (0..columns)
                .map(|n| q.column_text(n as i32).unwrap())
                .collect(),
        );
    }
    found
}
fn custody(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    rows(
        product,
        "SELECT operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,
        binary_digest_sha256,profile_id,domain_id,generation,state,COALESCE(stop_proof_hash,''),
        CAST(stop_proof_hash IS NULL AS TEXT) FROM main.gogoke_coordination_process_custody
        WHERE profile_id=?1 AND domain_id IN ('projectA','projectB') ORDER BY operation_id",
        &[INSTANCE],
        13,
    )
}
fn episodes(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    rows(
        product,
        "SELECT domain_id,request_id,session_id,generation,quote(old_generation),raw_hex,
        CAST(previous_revision AS TEXT),quote(result_revision),
        quote(process_operation_id),instance_id,home_id,binding_id,quote(seat_id),
        quote(seat_incarnation),quote(stop_request_id),phase,quote(stop_fact_id)
        FROM main.gogoke_v37_h_process_episode
        WHERE instance_id=?1 ORDER BY domain_id,request_id",
        &[INSTANCE],
        17,
    )
}
fn rpc(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    rows(
        product,
        "SELECT domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,
        custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,
        command_hex,CAST(requires_response AS TEXT),phase,quote(source_epoch),quote(source_cursor),
        quote(original_error),quote(permission_evidence) FROM main.gogoke_v37_rpc_steps
        WHERE profile_id=?1 ORDER BY domain_id,session_id,step_id",
        &[INSTANCE],
        20,
    )
}
fn claims(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    rows(
        product,
        "SELECT domain_id,session_id,instance_id,home_id,binding_id,generation,state,
        CAST(revision AS TEXT),COALESCE(process_operation_id,''),COALESCE(stop_fact_id,''),
        CAST(stop_fact_id IS NULL AS TEXT) FROM main.gogoke_v37_h_claim
        WHERE instance_id=?1 ORDER BY domain_id,session_id",
        &[INSTANCE],
        11,
    )
}
fn journal(product: &ProductDatabase<'_>) -> Vec<Vec<String>> {
    rows(product,"SELECT process_operation_id,instance_id,domain_id,session_id,request_id,
        snapshot_hex,snapshot_digest,phase,COALESCE(original_error,''),CAST(revision AS TEXT)
        FROM main.gogoke_v37_claude_holder_recovery WHERE instance_id=?1 ORDER BY domain_id,session_id",
        &[INSTANCE],10)
}
fn hex_bytes(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value.as_bytes().chunks_exact(2).map(|pair| {
        u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()
    }).collect()
}
fn captured_instance_after(row: &[String]) -> String {
    let snapshot = hex_bytes(&row[5]);
    let body = std::str::from_utf8(&snapshot).unwrap();
    let Json::Object(fields) = crate::store::atomic::Parser::parse(body).unwrap() else {
        panic!("captured journal body");
    };
    let Some(Json::Array(objects)) = fields.get(&JsonString::from_str("objects")) else {
        panic!("captured objects");
    };
    for object in objects {
        let Json::Object(values) = object else { panic!("captured ACL object"); };
        let value = |key: &str| -> String {
            let Some(Json::String(text)) = values.get(&JsonString::from_str(key)) else {
                panic!("captured ACL field {key}");
            };
            text.to_well_formed_string().unwrap()
        };
        if value("root") == "0" && value("relative").is_empty() {
            return value("after");
        }
    }
    panic!("captured instance root ACL absent");
}
fn acl_image_has_sid(image_hex: &str, sid_text: &str) -> bool {
    let parts: Vec<u64> = sid_text.strip_prefix("S-").unwrap().split('-')
        .map(|part| part.parse().unwrap()).collect();
    assert_eq!(parts[0], 1);
    let count = u8::try_from(parts.len() - 2).unwrap();
    let mut sid = vec![1, count];
    sid.extend_from_slice(&parts[1].to_be_bytes()[2..]);
    for part in parts.iter().skip(2) {
        sid.extend_from_slice(&u32::try_from(*part).unwrap().to_le_bytes());
    }
    let image = hex_bytes(image_hex);
    let mut cursor = 0;
    while cursor < image.len() {
        let length = u32::from_be_bytes(image[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        let ace = &image[cursor..cursor + length];
        if ace.len() >= 8 + sid.len() && ace[8..8 + sid.len()] == sid {
            return true;
        }
        cursor += length;
    }
    false
}
fn occupancy(product: &mut ProductDatabase<'_>) -> Option<usize> {
    let raw = product
        .configure_user_v37(
            br#"{"schema":"gogoke.37.owner-configuration.v1",
        "command":"instance-management-read"}"#,
        )
        .expect("original USER management read");
    let Json::Object(page) =
        crate::store::atomic::Parser::parse(std::str::from_utf8(&raw).unwrap()).unwrap()
    else {
        panic!("management page object");
    };
    let Some(Json::Array(profiles)) = page.get(&JsonString::from_str("profiles")) else {
        panic!("profiles");
    };
    profiles
        .iter()
        .find_map(|profile| {
            let Json::Object(fields) = profile else {
                return None;
            };
            let Some(Json::String(id)) = fields.get(&JsonString::from_str("instanceId")) else {
                return None;
            };
            if id.to_well_formed_string().as_deref() != Some(INSTANCE) {
                return None;
            }
            Some(match fields.get(&JsonString::from_str("runningSessions")) {
                Some(Json::Null) => None,
                Some(Json::Number(value)) => Some(value.parse::<usize>().unwrap()),
                _ => panic!("runningSessions shape"),
            })
        })
        .expect("registered instance")
}

#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}
#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    fn GetProcessTimes(
        handle: *mut std::ffi::c_void,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
}
fn held_parent(pair: (u32, u64)) -> OwnedHandle {
    let handle = unsafe { OpenProcess(0x0010_1000, 0, pair.0) };
    assert!(!handle.is_null(), "original Claude parent handle");
    let held = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut creation = FileTime { low: 0, high: 0 };
    let mut exit = FileTime { low: 0, high: 0 };
    let mut kernel = FileTime { low: 0, high: 0 };
    let mut user = FileTime { low: 0, high: 0 };
    assert_ne!(
        unsafe {
            GetProcessTimes(
                held.as_raw_handle().cast(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        },
        0
    );
    assert_eq!(
        (u64::from(creation.high) << 32) | u64::from(creation.low),
        pair.1
    );
    held
}
fn wait_gone(handle: &OwnedHandle, pair: (u32, u64)) {
    assert_eq!(
        unsafe { WaitForSingleObject(handle.as_raw_handle().cast(), 15_000) },
        0,
        "same original Claude PID/creation must exit after host Job close: {pair:?}"
    );
}

struct Cold {
    database: std::path::PathBuf,
    pairs: Vec<(u32, u64)>,
    custody: Vec<Vec<String>>,
    episodes: Vec<Vec<String>>,
    rpc: Vec<Vec<String>>,
    claims: Vec<Vec<String>>,
    stopped: Option<StoppedHistory>,
    target_profiles: Vec<String>,
}
struct StoppedHistory {
    home: std::path::PathBuf,
    home_identity: crate::root::RootIdentity,
    profile_name: String,
    sid: String,
    operation_id: String,
    stop_fact: String,
}
fn cold_two_claude(run: impl for<'a> FnOnce(ProductDatabase<'a>, &'a RootLock, &Cold)) {
    cold_two_claude_with_stopped(false, run)
}
fn cold_two_claude_with_stopped(
    include_stopped: bool,
    run: impl for<'a> FnOnce(ProductDatabase<'a>, &'a RootLock, &Cold),
) {
    let _guard = route_b_test_guard();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-claude-holder-composed-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    managed_cli_test_setup::ready(&mut product, &root, "claude");
    applied(
        &mut product,
        &operation(
            "global",
            "K-INSTANCE",
            "register",
            "claude-holder-register",
            INSTANCE,
            0,
            r#"{"driverId":"claude"}"#,
        ),
    );
    // Sole synthetic boundary: registry login presence. No credential bytes,
    // login UI, account claim, H ACK or model output is fabricated.
    instance::record_observation(
        &mut product.connection,
        &root,
        &instance::ObservationRequest {
            request_id: "claude-holder-synthetic-login-presence",
            request_bytes: b"SYNTHETIC_LOGIN_PRESENCE_NOT_AUTHENTICATION",
            instance_id: INSTANCE,
            expected_revision: 1,
            observation: instance::InstanceObservation::LoggedIn,
        },
    )
    .unwrap();
    assert_eq!(
        product
            .read_registered_instance(INSTANCE)
            .unwrap()
            .unwrap()
            .login_state,
        "LOGGED_IN"
    );
    instance::set_instance_concurrency_cap(&mut product.connection, &product.owner, INSTANCE, 2)
        .unwrap();
    let source = crate::store::worktree::tests::make_source_fixture(
        &mut product.connection,
        &root,
        &product.owner,
        &mut product.process_custodian,
    );
    let git = std::env::var_os("GOGOKE_CONTROLLED_GIT_PATH").expect("cloud controlled Git pin");
    let configuration = Json::Object(BTreeMap::from([
        (
            JsonString::from_str("schema"),
            Json::String(JsonString::from_str("gogoke.37.owner-configuration.v1")),
        ),
        (
            JsonString::from_str("command"),
            Json::String(JsonString::from_str("worktree-source")),
        ),
        (
            JsonString::from_str("repositoryId"),
            Json::String(JsonString::from_str("fixtureRepo")),
        ),
        (
            JsonString::from_str("sourcePath"),
            Json::String(JsonString::from_str(source.to_str().unwrap())),
        ),
        (
            JsonString::from_str("gitPath"),
            Json::String(JsonString::from_str(git.to_str().unwrap())),
        ),
    ]))
    .canonical();
    product
        .configure_user_v37(configuration.as_bytes())
        .unwrap();
    let mut pairs = Vec::new();
    let mut parents = Vec::new();
    let mut stopped = None;
    let mut target_profiles = Vec::new();
    for (domain, seat_id, tree, session) in SCOPES {
        seat::set_project_parallel_cap(&mut product.connection, &product.owner, domain, 1).unwrap();
        seat::store_template(&mut product.connection,NativeOrigin::user(&product.owner),StoreTemplate{
            domain_id:domain,template_id:"templateA",
            settings_json:br#"{"effort":"high","model":"claude-sonnet-4-6","permissionTier":"NETWORKED_WRITE"}"#,
        }).unwrap();
        seat::create(
            &mut product.connection,
            NativeOrigin::user(&product.owner),
            CreateSeat {
                domain_id: domain,
                seat_id,
                template_id: "templateA",
                instance_id: Some(INSTANCE),
                kind: Kind::Long,
                request_id: &format!("claude-holder-seat-{seat_id}"),
                request_bytes: format!("seat {seat_id}").as_bytes(),
            },
        )
        .unwrap();
        applied(
            &mut product,
            &operation(
                domain,
                "K-WORKTREE",
                "create",
                &format!("claude-tree-{tree}"),
                tree,
                0,
                &format!(r#"{{"repositoryId":"fixtureRepo","seatId":"{seat_id}"}}"#),
            ),
        );
        let graph = worktree::graph_query(&product.connection, tree)
            .unwrap()
            .unwrap();
        assert_eq!(graph.classification, "SINGLE");
        assert_eq!(graph.members.len(), 1);
        if include_stopped && domain == "projectA" {
            let historical_session = "stoppedSessionA";
            let historical_seat = seat::get(&product.connection, domain, seat_id)
                .unwrap().unwrap();
            let historical_generation = historical_seat.generation + 1;
            let historical_f = worktree::resolve_for_launch(&product.connection, &root,
                tree, "fixtureRepo", domain, seat_id, &historical_seat.incarnation,
                historical_seat.generation).unwrap();
            let profile_name = h::launch::original_session_profile_name(
                &root.canonical_root().identity.opaque(), domain, historical_session,
                &historical_seat.incarnation, &historical_generation.to_string());
            let profile = AppContainerProfile::derive_for_revocation(&profile_name).unwrap();
            let sid = profile.sid_identity().unwrap();
            for (verb, revision) in [("admission-reserve", 0), ("admission-commit", 1)] {
                applied(&mut product, &operation(domain, "K-SESSION", verb,
                    &format!("claude-history-{verb}"), historical_session, revision,
                    &format!(r#"{{"seatId":"{seat_id}","generation":"{historical_generation}"}}"#)));
            }
            applied(&mut product, &operation(domain, "K-SESSION", "open",
                "claude-history-open", historical_session, 2,
                &format!(r#"{{"seatId":"{seat_id}","generation":"{historical_generation}",
                    "repositoryId":"fixtureRepo","worktreeId":"{tree}"}}"#)));
            let operation_id = product.native_sessions.get(&(domain.into(), historical_session.into()))
                .unwrap().operation_id.clone();
            let stopped_raw = product.dispatch_user_request(&operation(domain, "K-SESSION",
                "stop", "claude-history-stop", historical_session, 3,
                &format!(r#"{{"seatId":"{seat_id}","generation":"{historical_generation}"}}"#)))
                .expect("actual Claude child normal stop");
            let stopped_receipt = h::decode_receipt(&stopped_raw).unwrap();
            assert_eq!(stopped_receipt.status, V37Status::Applied,
                "normal stop: {}", String::from_utf8_lossy(&stopped_raw));
            let proof = rows(&product,
                "SELECT c.stop_proof_hash,e.stop_fact_id,a.stop_fact_id,c.state,e.phase,a.state
                 FROM main.gogoke_coordination_process_custody c
                 JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=c.operation_id
                 JOIN main.gogoke_v37_h_claim a ON a.process_operation_id=c.operation_id
                 WHERE c.operation_id=?1", &[&operation_id], 6);
            assert_eq!(proof.len(), 1);
            assert!(proof[0][0].starts_with("sha256:") && proof[0][0] == proof[0][1]
                && proof[0][0] == proof[0][2]);
            assert_eq!((&proof[0][3][..], &proof[0][4][..], &proof[0][5][..]),
                ("STOPPED", "STOPPED", "STOPPED"));
            let stop_fact = proof[0][0].clone();
            applied(&mut product, &operation(domain, "K-SESSION", "admission-release",
                "claude-history-release", historical_session, stopped_receipt.revision,
                &format!(r#"{{"seatId":"{seat_id}","generation":"{historical_generation}"}}"#)));
            assert_eq!(seat::get(&product.connection, domain, seat_id).unwrap().unwrap().state,
                seat::State::Idle);
            let next_seat = seat::get(&product.connection, domain, seat_id)
                .unwrap().unwrap();
            let rebound_f = worktree::resolve_for_launch(&product.connection, &root,
                tree, "fixtureRepo", domain, seat_id, &next_seat.incarnation,
                next_seat.generation).unwrap();
            assert_eq!((&rebound_f.path, &rebound_f.identity),
                (&historical_f.path, &historical_f.identity),
                "F's existing physical root is reused at the producer-approved E generation");
            let home = root.canonical_root().canonical_path
                .join("v37-instances").join(INSTANCE);
            let home_identity = crate::root::inspect_root(&home).unwrap().identity;
            profile.verify_bound_directory_grant(&home, &home_identity, true)
                .expect("normal STOPPED leaves its exact package SID ACE on reused HOME");
            stopped = Some(StoppedHistory { home, home_identity, profile_name, sid,
                operation_id, stop_fact });
        }
        let current_seat = seat::get(&product.connection, domain, seat_id)
            .unwrap().unwrap();
        let generation = current_seat.generation + 1;
        target_profiles.push(h::launch::original_session_profile_name(
            &root.canonical_root().identity.opaque(), domain, session,
            &current_seat.incarnation, &generation.to_string()));
        for (verb, revision) in [("admission-reserve", 0), ("admission-commit", 1)] {
            applied(
                &mut product,
                &operation(
                    domain,
                    "K-SESSION",
                    verb,
                    &format!("claude-{verb}-{session}"),
                    session,
                    revision,
                    &format!(r#"{{"seatId":"{seat_id}","generation":"{generation}"}}"#),
                ),
            );
        }
        let open = operation(
            domain,
            "K-SESSION",
            "open",
            &format!("claude-open-{session}"),
            session,
            2,
            &format!(
                r#"{{"seatId":"{seat_id}","generation":"{generation}",
                "repositoryId":"fixtureRepo","worktreeId":"{tree}"}}"#
            ),
        );
        applied(&mut product, &open);
        let run = product
            .native_sessions
            .get(&(domain.into(), session.into()))
            .unwrap();
        assert_eq!(run.evidence.driver_id(), "claude");
        assert!(
            run.thread_id.is_none(),
            "initialize ACK alone invents no vendor session"
        );
        let pair = (
            run.custody.identity.pid,
            run.custody.identity.creation_time_100ns,
        );
        assert!(
            NativeProcessHoldersGone::observe(&[pair]).is_err(),
            "actual old Claude child remains live"
        );
        parents.push(held_parent(pair));
        pairs.push(pair);
    }
    assert_eq!(product.native_sessions.len(), 2);
    let before = Cold {
        database,
        pairs,
        custody: custody(&product),
        episodes: episodes(&product),
        rpc: rpc(&product),
        claims: claims(&product),
        stopped,
        target_profiles,
    };
    assert_eq!(before.custody.len(), if include_stopped { 3 } else { 2 });
    assert_eq!(before.claims.len(), if include_stopped { 3 } else { 2 });
    assert!(before.claims.iter().filter(|row| row[1] != "stoppedSessionA")
        .all(|row| row[6] == "COMMITTED" && row[10] == "1"));
    assert!(
        before.rpc.iter().any(|row| row[15] == "OBSERVED"),
        "real Claude initialize ACK journal absent"
    );
    assert!(journal(&product).is_empty());
    product.close_checked().unwrap();
    for (held, pair) in parents.iter().zip(&before.pairs) {
        wait_gone(held, *pair);
    }
    let product = ProductDatabase::open(&root, &before.database).unwrap();
    assert!(product.native_sessions.is_empty());
    NativeProcessHoldersGone::observe(&before.pairs)
        .unwrap()
        .validate(&before.pairs)
        .unwrap();
    let current = custody(&product);
    for (old, now) in before.custody.iter().zip(&current) {
        let mut expected = old.clone();
        if matches!(expected[10].as_str(), "ACTIVE" | "PREPARED") {
            expected[10] = "UNKNOWN".into();
        }
        assert_eq!(
            now, &expected,
            "only native cold custodian state may change"
        );
    }
    assert_eq!(episodes(&product), before.episodes);
    assert_eq!(rpc(&product), before.rpc);
    assert_eq!(claims(&product), before.claims);
    run(product, &root, &before);
    drop(parents);
    drop(root);
    let actual = std::fs::canonicalize(&path).unwrap();
    assert!(actual.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
    assert!(actual
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("gogoke-claude-holder-composed-"));
    std::fs::remove_dir_all(actual).unwrap();
}

#[test]
fn claude_holder_two_real_h_releases_only_after_exact_acl_retirement_and_keeps_history() {
    cold_two_claude(|mut product, _root, cold| {
        assert_eq!(
            occupancy(&mut product),
            None,
            "cold UNKNOWN must not imply capacity"
        );
        let custody_before = custody(&product);
        let episodes_before = episodes(&product);
        let rpc_before = rpc(&product);
        // The original producer performs two separate H releases. Fault only
        // the second operation receipt, after the first exact ACL retirement
        // and its original H/E transaction have committed.
        product
            .connection
            .execute(
                "CREATE TEMP TRIGGER claude_holder_second_release_cut
            BEFORE INSERT ON main.gogoke_v37_h_operation
            WHEN NEW.operation='claude-holder-gone-release' AND NEW.session_id='sessionB'
            BEGIN SELECT RAISE(FAIL,'controlled second Claude H release cut'); END",
            )
            .unwrap();
        let interrupted = product
            .recover_disappeared_claude_resources(INSTANCE, None)
            .unwrap_err();
        assert!(
            format!("{interrupted:?}").contains("controlled second Claude H release cut"),
            "original second release fault must remain visible: {interrupted:?}"
        );
        let partial = journal(&product);
        assert_eq!(
            partial.len(),
            2,
            "both original intents persist across the cut"
        );
        assert_eq!(
            (partial[0][2].as_str(), partial[0][7].as_str()),
            ("projectA", "APPLIED")
        );
        assert_eq!(
            (partial[1][2].as_str(), partial[1][7].as_str()),
            ("projectB", "PREPARED")
        );
        let partial_claims = claims(&product);
        assert_eq!(
            (partial_claims[0][0].as_str(), partial_claims[0][6].as_str()),
            ("projectA", "RELEASED")
        );
        assert_eq!(
            (partial_claims[1][0].as_str(), partial_claims[1][6].as_str()),
            ("projectB", "COMMITTED")
        );
        assert_eq!(
            seat::get(&product.connection, "projectA", "seatA")
                .unwrap()
                .unwrap()
                .state,
            seat::State::Idle
        );
        assert_eq!(
            seat::get(&product.connection, "projectB", "seatB")
                .unwrap()
                .unwrap()
                .state,
            seat::State::Busy
        );
        assert_eq!(
            occupancy(&mut product),
            None,
            "one completed H cannot qualify the whole instance"
        );
        assert_eq!(custody(&product), custody_before);
        assert_eq!(episodes(&product), episodes_before);
        assert_eq!(rpc(&product), rpc_before);
        product
            .connection
            .execute("DROP TRIGGER temp.claude_holder_second_release_cut")
            .unwrap();
        product
            .recover_disappeared_claude_resources(INSTANCE, None)
            .expect("same captured intent must settle the remaining old H resource");
        assert_eq!(
            custody(&product),
            custody_before,
            "no STOPPED/StopFact or PID rewrite"
        );
        assert_eq!(
            episodes(&product),
            episodes_before,
            "old episode/UNKNOWN preserved"
        );
        assert_eq!(
            rpc(&product),
            rpc_before,
            "original Claude ACK/RPC kept byte-for-byte"
        );
        let captured_journal = journal(&product);
        assert_eq!(captured_journal.len(), 2);
        assert!(captured_journal
            .iter()
            .all(|row| row[7] == "APPLIED" && row[8].is_empty() && row[9] == "2"));
        let mut expected = cold.claims.clone();
        for claim in &mut expected {
            claim[6] = "RELEASED".into();
            claim[7] = (claim[7].parse::<u64>().unwrap() + 1).to_string();
        }
        assert_eq!(
            claims(&product),
            expected,
            "exact original H claims released with NULL StopFact"
        );
        for (domain, seat_id, _, _) in SCOPES {
            assert_eq!(
                seat::get(&product.connection, domain, seat_id)
                    .unwrap()
                    .unwrap()
                    .state,
                seat::State::Idle
            );
        }
        assert_eq!(
            occupancy(&mut product),
            Some(0),
            "only both completed H receipts qualify E capacity"
        );
        product
            .recover_disappeared_claude_resources(INSTANCE, None)
            .unwrap();
        assert_eq!(
            journal(&product),
            captured_journal,
            "cold replay cannot recapture or rewrite old intent"
        );
        assert_eq!(custody(&product), custody_before);
        assert_eq!(episodes(&product), episodes_before);
        assert_eq!(rpc(&product), rpc_before);
        product.close_checked().unwrap();
    });
}

#[test]
fn claude_holder_keeps_real_stopped_sid_when_later_unknown_holders_disappear() {
    cold_two_claude_with_stopped(true, |mut product, _root, cold| {
        let history = cold.stopped.as_ref().expect("real earlier normal stop");
        let stopped_profile = AppContainerProfile::derive_for_revocation(&history.profile_name)
            .unwrap();
        assert_eq!(stopped_profile.sid_identity().unwrap(), history.sid);
        let historical_grant = stopped_profile.verify_bound_directory_grant(
            &history.home, &history.home_identity, true).unwrap();
        assert_eq!(cold.custody.iter().find(|row| row[0] == history.operation_id)
            .unwrap()[11], history.stop_fact);
        // Only the two later open outcomes are projected to the persisted
        // UNKNOWN shape. Their real initialize ACKs remain unchanged.
        product.connection.execute(
            "UPDATE main.gogoke_v37_h_operation SET status='UNKNOWN'
             WHERE operation='open' AND session_id IN ('sessionA','sessionB');
             UPDATE main.gogoke_v37_h_process_episode SET phase='PREPARED'
             WHERE instance_id='instanceA' AND session_id IN ('sessionA','sessionB')",
        ).unwrap();
        let opens_before = original_opens(&product);
        let custody_before = custody(&product);
        let episodes_before = episodes(&product);
        let rpc_before = rpc(&product);
        let claims_before = claims(&product);
        assert_eq!(opens_before.len(), 3);
        assert_eq!(opens_before.iter().filter(|row| row[5] == "UNKNOWN").count(), 2);
        assert_eq!(journal(&product).len(), 0);
        product.recover_disappeared_claude_resources(INSTANCE, None)
            .expect("normal STOPPED SID has validated provenance, not an unknown peer");
        let captured = journal(&product);
        assert_eq!(captured.len(), 2);
        assert!(captured.iter().all(|row| row[7] == "APPLIED"));
        for (row, target_name) in captured.iter().zip(&cold.target_profiles) {
            let after = captured_instance_after(row);
            let target = AppContainerProfile::derive_for_revocation(target_name).unwrap();
            assert!(acl_image_has_sid(&after, &history.sid),
                "sealed after image must retain the real STOPPED peer SID");
            assert!(!acl_image_has_sid(&after, &target.sid_identity().unwrap()),
                "sealed after image retires only its exact unconfirmed target SID");
            assert!(target.verify_bound_directory_grant(
                &history.home, &history.home_identity, true).is_err(),
                "unconfirmed H root grant must be retired");
        }
        assert_eq!(stopped_profile.verify_bound_directory_grant(
            &history.home, &history.home_identity, true).unwrap(), historical_grant,
            "the normal STOPPED root grant is not the recovery target");
        assert_eq!(original_opens(&product), opens_before);
        assert_eq!(custody(&product), custody_before, "real STOPPED and old UNKNOWN stay exact");
        assert_eq!(episodes(&product), episodes_before);
        assert_eq!(rpc(&product), rpc_before);
        let after_claims = claims(&product);
        assert_eq!(after_claims.len(), claims_before.len());
        for (before, after) in claims_before.iter().zip(&after_claims) {
            if before[1] == "stoppedSessionA" {
                assert_eq!(after, before, "real StopFact cannot be rewritten");
            } else {
                assert_eq!(after[6], "RELEASED");
                assert_eq!(after[9], "", "disappearance never invents StopFact");
            }
        }
        product.close_checked().unwrap();
    });
}

#[test]
fn claude_holder_real_stopped_provenance_does_not_admit_foreign_package_sid() {
    cold_two_claude_with_stopped(true, |mut product, _root, cold| {
        let history = cold.stopped.as_ref().unwrap();
        let foreign = AppContainerProfile::derive_for_revocation(
            "Gogoke37.ForeignClaudeHolderFixture").unwrap();
        let foreign_sid = foreign.sid_identity().unwrap();
        assert_ne!(foreign_sid, history.sid);
        for name in &cold.target_profiles {
            assert_ne!(foreign_sid,
                AppContainerProfile::derive_for_revocation(name).unwrap().sid_identity().unwrap());
        }
        foreign.grant_bound_tree(&history.home, &history.home_identity, true)
            .expect("test-only foreign package ACE on private fixture HOME");
        let before = (original_opens(&product), custody(&product), episodes(&product),
            rpc(&product), claims(&product));
        let refused = product.recover_disappeared_claude_resources(INSTANCE, None).unwrap_err();
        assert!(format!("{refused:?}").contains("AclWitnessMismatch"), "{refused:?}");
        assert!(journal(&product).is_empty(), "foreign SID rejected before capture intent");
        assert_eq!((original_opens(&product), custody(&product), episodes(&product),
            rpc(&product), claims(&product)), before);
        foreign.verify_bound_directory_grant(&history.home, &history.home_identity, true)
            .expect("refusal did not edit the foreign test ACE");
        product.close_checked().unwrap();
    });
}

#[test]
fn claude_holder_rejects_stopped_peer_stop_bytes_changed_after_capture() {
    cold_two_claude_with_stopped(true, |mut product, _root, cold| {
        let history = cold.stopped.as_ref().unwrap();
        let stop_row = rows(&product,
            "SELECT raw_hex FROM main.gogoke_v37_h_operation
             WHERE domain_id='projectA' AND request_id='claude-history-stop'
               AND operation='stop'", &[], 1);
        assert_eq!(stop_row.len(), 1);
        assert!(!stop_row[0][0].is_empty());
        product.connection.execute(
            "CREATE TEMP TRIGGER claude_stopped_peer_second_release_cut
             BEFORE INSERT ON main.gogoke_v37_h_operation
             WHEN NEW.operation='claude-holder-gone-release' AND NEW.session_id='sessionB'
             BEGIN SELECT RAISE(FAIL,'controlled stopped peer second release cut'); END",
        ).unwrap();
        let cut = product.recover_disappeared_claude_resources(INSTANCE, None).unwrap_err();
        assert!(format!("{cut:?}").contains("controlled stopped peer second release cut"),
            "{cut:?}");
        product.connection.execute(
            "DROP TRIGGER temp.claude_stopped_peer_second_release_cut",
        ).unwrap();
        let sealed = journal(&product);
        assert_eq!(sealed.len(), 2);
        assert_eq!((sealed[0][7].as_str(), sealed[1][7].as_str()),
            ("APPLIED", "PREPARED"));
        let second = AppContainerProfile::derive_for_revocation(&cold.target_profiles[1])
            .unwrap();
        let stopped_profile = AppContainerProfile::derive_for_revocation(&history.profile_name)
            .unwrap();
        let stopped_grant = stopped_profile.verify_bound_directory_grant(
            &history.home, &history.home_identity, true).unwrap();
        let second_after = captured_instance_after(&sealed[1]);
        assert!(acl_image_has_sid(&second_after, &history.sid));
        assert!(!acl_image_has_sid(&second_after, &second.sid_identity().unwrap()));
        assert!(second.verify_bound_directory_grant(
            &history.home, &history.home_identity, true).is_err(),
            "second target ACL was already retired before the release cut");
        let claims_before_drift = claims(&product);
        assert_eq!(claims_before_drift.iter().find(|row| row[1] == "stoppedSessionA")
            .unwrap()[9], history.stop_fact);
        // The real normal STOPPED fact stays; alter only its original stop
        // request bytes after both later H ACL captures were sealed.
        product.connection.execute(
            "UPDATE main.gogoke_v37_h_operation SET raw_hex=raw_hex||'00'
             WHERE domain_id='projectA' AND request_id='claude-history-stop'
               AND operation='stop'",
        ).unwrap();
        let changed = rows(&product,
            "SELECT raw_hex FROM main.gogoke_v37_h_operation
             WHERE domain_id='projectA' AND request_id='claude-history-stop'
               AND operation='stop'", &[], 1);
        assert_eq!(changed[0][0], format!("{}00", stop_row[0][0]));
        let refused = product.recover_disappeared_claude_resources(INSTANCE, None)
            .unwrap_err();
        let reason = format!("{refused:?}");
        assert!(reason.contains("unverified") || reason.contains("changed")
            || reason.contains("peer") || reason.contains("provenance"), "{refused:?}");
        assert_eq!(journal(&product), sealed, "sealed ACL intent cannot be refreshed");
        assert_eq!(claims(&product), claims_before_drift,
            "historical StopFact and pending later H claim remain exact");
        assert_eq!(captured_instance_after(&journal(&product)[1]), second_after);
        assert_eq!(stopped_profile.verify_bound_directory_grant(
            &history.home, &history.home_identity, true).unwrap(), stopped_grant);
        assert!(second.verify_bound_directory_grant(
            &history.home, &history.home_identity, true).is_err(),
            "drift cannot restore or further edit the already retired target SID");
        product.close_checked().unwrap();
    });
}
