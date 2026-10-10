//! Cloud security composition, with one explicit synthetic fact:
//! `SYNTHETIC_LOGIN_PRESENCE_NOT_AUTHENTICATION` marks instance registry login metadata only.
//! No account, credential, User model turn or vendor session is invented.
//! E/F/H, fixed Claude initialize ACK, LPAC process identities and ACL effects
//! come from their production producers. This is not installed Owner recovery.
use super::*;
use crate::process::NativeProcessHoldersGone;
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
}
fn cold_two_claude(run: impl for<'a> FnOnce(ProductDatabase<'a>, &'a RootLock, &Cold)) {
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
        let generation = seat::get(&product.connection, domain, seat_id)
            .unwrap()
            .unwrap()
            .generation
            + 1;
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
    };
    assert_eq!(before.custody.len(), 2);
    assert_eq!(before.claims.len(), 2);
    assert!(before
        .claims
        .iter()
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
