use super::*;
use crate::process::{AppContainerProfile, NativeBinding, PrepareRequest,
    ProcessCustodian, ProcessLaunch};
use crate::root::{inspect_root, RootLock};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CHILD: &str = "process::session::credential_binding::tests::credential_child";

#[test]
fn credential_child() {
    let Some(path) = std::env::var_os("GOGOKE_TEST_CREDENTIAL_ALIAS") else { return; };
    let report = std::env::var_os("GOGOKE_TEST_CREDENTIAL_REPORT").expect("synthetic report path");
    let write = std::env::var_os("GOGOKE_TEST_CREDENTIAL_WRITE").is_some();
    // This is the CLI-equivalent child operating on a synthetic nonsecret
    // fixture. The Host's binding code never requests or reads data access.
    let read_result = fs::read(&path).map(|_| ());
    let write_result = if write {
        OpenOptions::new().write(true).truncate(true).open(&path)
            .and_then(|mut file| file.write_all(b"synthetic-native-in-place-save"))
    } else { Ok(()) };
    let describe = |result: std::io::Result<()>| match result {
        Ok(()) => "OK".to_owned(),
        Err(error) => format!("ERR:{:?};{error}", error.raw_os_error()),
    };
    fs::write(report, format!("read={} write={}", describe(read_result), describe(write_result)))
        .expect("child result report in granted runner");
}

fn test_root() -> (RootLock, std::path::PathBuf) {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-credential-binding-{}-{nonce}", std::process::id()));
    fs::create_dir(&path).expect("new isolated synthetic test root");
    let root = RootLock::acquire(&path).expect("exact synthetic RootLock");
    (root, path)
}

fn scope(path: &Path) -> CredentialAliasScope {
    CredentialAliasScope { root: path.to_path_buf(),
        root_identity: inspect_root(path).expect("physical scope directory").identity }
}

fn prepare_runner(profile: &AppContainerProfile, path: &Path) -> std::path::PathBuf {
    fs::create_dir(path).expect("fresh runner");
    profile.grant_fresh_session_directory(path).expect("exact LPAC runner ACE");
    let exe = path.join("credential-synthetic-child.exe");
    fs::copy(std::env::current_exe().expect("exact cloud test image"), &exe)
        .expect("copy cloud test image under profile runner");
    exe
}

fn run_child(profile_name: &str, exe: &Path, alias: &CredentialAlias,
    write: bool) -> String {
    let runner = exe.parent().expect("child runner");
    let report = runner.join("actual-result.txt");
    if report.exists() { fs::remove_file(&report).expect("remove old synthetic report"); }
    let mut launch = ProcessLaunch::new(exe);
    launch.current_directory = Some(runner.to_path_buf());
    launch.protocol_stdio = true;
    launch.app_container_profile = Some(profile_name.to_owned());
    launch.app_container_cli_identity_services = true;
    launch.app_container_internet_client = true;
    launch.environment = Some(vec![
        ("SystemRoot".into(), std::env::var("SystemRoot").expect("cloud SystemRoot")),
        ("USERPROFILE".into(), runner.to_string_lossy().into_owned()),
        ("LOCALAPPDATA".into(), runner.to_string_lossy().into_owned()),
        ("GOGOKE_TEST_CREDENTIAL_ALIAS".into(), alias.path().to_string_lossy().into_owned()),
        ("GOGOKE_TEST_CREDENTIAL_REPORT".into(), report.to_string_lossy().into_owned()),
    ]);
    if write { launch.environment.as_mut().unwrap().push((
        "GOGOKE_TEST_CREDENTIAL_WRITE".into(), "1".into())); }
    launch.arguments = vec!["--exact".into(), CHILD.into(), "--nocapture".into()];
    let digest = crate::store::digest::sha256_hex(&fs::read(exe).expect("cloud fixture image bytes"));
    let request = PrepareRequest { launch, binding: NativeBinding {
        binary_digest_sha256: format!("sha256:{digest}"),
        profile_id: "credential-fixture".into(), domain_id: "synthetic".into(),
        generation: "1".into(),
    } };
    let mut custody = ProcessCustodian::new().expect("exact process custodian");
    let prepared = custody.prepare(&request).expect("prepared exact LPAC child");
    custody.activate(&prepared).expect("activated exact LPAC child");
    let managed = custody.active(&prepared.ticket).expect("active exact child");
    assert!(managed.wait(Duration::from_secs(20)).expect("native child wait"),
        "LPAC child did not finish; stderr={:?}", managed.stderr_tail());
    assert_eq!(managed.exit_code().expect("native child exit"), Some(0),
        "LPAC child stderr={:?}", managed.stderr_tail());
    let result = fs::read_to_string(&report).expect("original child access result");
    drop(custody);
    result
}

#[test]
fn exact_credential_alias_custody_allows_two_profiles_and_precise_revocation() {
    let (root, requested) = test_root();
    let base = root.canonical_root().canonical_path.clone();
    let instance = base.join("instanceA");
    let scope_a_path = instance.join("private-history").join("scopeA");
    let scope_b_path = instance.join("private-history").join("scopeB");
    fs::create_dir_all(&scope_a_path).unwrap();
    fs::create_dir_all(&scope_b_path).unwrap();
    let source = instance.join(AUTH_NAME);
    fs::write(&source, b"synthetic-nonsecret-initial").unwrap();
    let source_id = AppContainerProfile::capture_program_identity(&source).unwrap();
    let instance_id = inspect_root(&instance).unwrap().identity;
    let a = scope(&scope_a_path);
    let b = scope(&scope_b_path);
    let binding = CredentialBinding::open_registered(&root, &source,
        &instance_id, &source_id, &[]).expect("metadata-only source holder");
    assert_eq!(binding.denied_data_read_for_test().unwrap_err().raw_os_error(), Some(5),
        "the actual held source handle must lack FILE_READ_DATA");
    let name_a = format!("Gogoke37.CredentialA.{}", std::process::id());
    let name_b = format!("Gogoke37.CredentialB.{}", std::process::id());
    let profile_a = AppContainerProfile::ensure_for_cli(&name_a, true).unwrap();
    let profile_b = AppContainerProfile::ensure_for_cli(&name_b, true).unwrap();
    let alias_a = binding.create_alias(&root, a.clone(), &[]).expect("first exact alias");
    assert!(matches!(profile_a.grant_bound_tree(&scope_a_path, &a.root_identity, true),
        Err(crate::process::IsolationError::DirectoryNotPhysical)),
        "ordinary tree still refuses the multi-link credential file");
    profile_a.grant_bound_credential_tree(&scope_a_path, &a.root_identity,
        &binding, &alias_a, true).expect("A exact scope and auth ACL");
    let runner_a = prepare_runner(&profile_a, &base.join("runnerA"));
    let result_a = run_child(&name_a, &runner_a, &alias_a, true);
    assert_eq!(result_a, "read=OK write=OK", "A native in-place access: {result_a}");
    let size_after_a = fs::metadata(&source).unwrap().len();
    assert_eq!(size_after_a, b"synthetic-native-in-place-save".len() as u64);

    // The first holder remains alive while F's second pending alias is made.
    let alias_b = binding.create_alias(&root, b.clone(), &[a.clone()])
        .expect("second alias under the one held file ID");
    let same = CredentialBinding::open_registered(&root, &source, &instance_id,
        &source_id, &[a.clone(), b.clone()]).expect("recheck complete registered aliases");
    assert!(Arc::ptr_eq(&binding, &same), "one object has one shared holder");
    profile_b.grant_bound_credential_tree(&scope_b_path, &b.root_identity,
        &same, &alias_b, true).expect("B exact scope and same auth object");
    profile_a.verify_bound_credential_tree_grant(&scope_a_path, &a.root_identity,
        &binding, &alias_a, true).expect("A grant remains after B enters");
    let runner_b = prepare_runner(&profile_b, &base.join("runnerB"));
    let result_b = run_child(&name_b, &runner_b, &alias_b, true);
    assert_eq!(result_b, "read=OK write=OK", "B native in-place access: {result_b}");
    binding.verify_registered_aliases(&[a.clone(), b.clone()])
        .expect("all three names still refer to the same object");
    profile_a.revoke_credential_alias(&binding, &alias_a)
        .expect("stop A removes only A's auth SID");
    let denied_a = run_child(&name_a, &runner_a, &alias_a, false);
    assert!(denied_a.starts_with("read=ERR:Some(5);") && denied_a.ends_with(" write=OK"),
        "A read after its SID is revoked: {denied_a}");
    let remaining_b = run_child(&name_b, &runner_b, &alias_b, true);
    assert_eq!(remaining_b, "read=OK write=OK", "B remains admitted: {remaining_b}");
    assert!(alias_a.path().is_file() && alias_b.path().is_file(),
        "normal stop must not unlink aliases while another scope is active");
    drop((profile_a, profile_b, same, binding));
    drop(root);
    fs::remove_dir_all(requested).expect("only isolated synthetic root cleanup");
    #[link(name = "userenv")]
    extern "system" { fn DeleteAppContainerProfile(name: *const u16) -> i32; }
    for name in [name_a, name_b] {
        let wide: Vec<u16> = OsStr::new(&name).encode_wide().chain(Some(0)).collect();
        assert!(unsafe { DeleteAppContainerProfile(wide.as_ptr()) } >= 0);
    }
}

#[test]
fn unknown_link_and_replaced_alias_fail_complete_metadata_registration() {
    let (root, requested) = test_root();
    let base = root.canonical_root().canonical_path.clone();
    let instance = base.join("instanceA");
    let alias_root = instance.join("private-history").join("scopeA");
    fs::create_dir_all(&alias_root).unwrap();
    let source = instance.join(AUTH_NAME);
    fs::write(&source, b"synthetic-nonsecret").unwrap();
    let source_id = AppContainerProfile::capture_program_identity(&source).unwrap();
    let parent_id = inspect_root(&instance).unwrap().identity;
    let a = scope(&alias_root);
    fs::hard_link(&source, alias_root.join(AUTH_NAME)).unwrap();
    fs::hard_link(&source, base.join("unregistered-link")).unwrap();
    assert!(matches!(CredentialBinding::open_registered(&root, &source,
        &parent_id, &source_id, &[a.clone()]), Err(CredentialError::LinkCount { expected: 2, observed: 3 })),
        "exact names do not excuse one unknown hardlink");
    fs::remove_file(base.join("unregistered-link")).unwrap();
    fs::remove_file(alias_root.join(AUTH_NAME)).unwrap();
    fs::write(alias_root.join(AUTH_NAME), b"different-synthetic-object").unwrap();
    assert!(matches!(CredentialBinding::open_registered(&root, &source,
        &parent_id, &source_id, &[a]), Err(CredentialError::IdentityChanged)),
        "same alias name with another file ID is refused");
    drop(root);
    fs::remove_dir_all(requested).unwrap();
}
