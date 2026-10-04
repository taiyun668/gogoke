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
    if std::env::var_os("GOGOKE_TEST_CREDENTIAL_CREATE").is_some() {
        let home = std::path::PathBuf::from(std::env::var_os("CODEX_HOME")
            .expect("actual account initialization HOME"));
        let target = std::path::PathBuf::from(&path);
        assert_eq!(target.parent().and_then(Path::parent), Some(home.as_path()),
            "initialization target is in the fresh HOME runtime");
        let create = OpenOptions::new().write(true).create_new(true).open(&target)
            .and_then(|mut file| file.write_all(b"synthetic-account-init"));
        let reopen = fs::read(&target).map(|bytes| bytes == b"synthetic-account-init");
        let describe = |result: std::io::Result<()>| match result {
            Ok(()) => "OK".to_owned(),
            Err(error) => format!("ERR:{:?};{error}", error.raw_os_error()),
        };
        fs::write(report, format!("create={} reopen={:?}", describe(create), reopen))
            .expect("child account initialization result report");
        return;
    }
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
    run_child_path(profile_name, exe, &alias.path(), write)
}

fn run_child_path(profile_name: &str, exe: &Path, target: &Path,
    write: bool) -> String {
    run_child_path_with_home(profile_name, exe, target, write, None)
}

fn run_child_path_with_home(profile_name: &str, exe: &Path, target: &Path,
    write: bool, home: Option<&Path>) -> String {
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
        ("GOGOKE_TEST_CREDENTIAL_ALIAS".into(), target.to_string_lossy().into_owned()),
        ("GOGOKE_TEST_CREDENTIAL_REPORT".into(), report.to_string_lossy().into_owned()),
    ]);
    if write { launch.environment.as_mut().unwrap().push((
        "GOGOKE_TEST_CREDENTIAL_WRITE".into(), "1".into())); }
    if let Some(home) = home {
        launch.environment.as_mut().unwrap().extend([
            ("CODEX_HOME".into(), home.to_string_lossy().into_owned()),
            ("GOGOKE_TEST_CREDENTIAL_CREATE".into(), "1".into()),
        ]);
    }
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

#[test]
fn quiescent_alias_removal_preserves_source_and_other_registered_alias() {
    let (root, requested) = test_root();
    let base = root.canonical_root().canonical_path.clone();
    let home = base.join("instanceA");
    let scope_a = home.join("private-history").join("scopeA");
    let scope_b = home.join("private-history").join("scopeB");
    fs::create_dir_all(&scope_a).unwrap();
    fs::create_dir_all(&scope_b).unwrap();
    let source = home.join(AUTH_NAME);
    fs::write(&source, b"synthetic-nonsecret-auth").unwrap();
    let source_id = AppContainerProfile::capture_program_identity(&source).unwrap();
    let home_id = inspect_root(&home).unwrap().identity;
    let a = scope(&scope_a);
    let b = scope(&scope_b);
    let binding = CredentialBinding::open_registered(&root, &source, &home_id,
        &source_id, &[]).unwrap();
    binding.create_alias(&root, a.clone(), &[]).unwrap();
    let alias_b = binding.create_alias(&root, b.clone(), &[a.clone()]).unwrap();
    let complete = [a.clone(), b.clone()];
    assert!(matches!(CredentialBinding::remove_quiescent_alias(binding.clone(),
        &root, &alias_b, &complete), Err(CredentialError::Invalid(
            "credential holder is still shared"))), "live Arc must prevent unlink");
    let other = home.join("synthetic-other-object");
    fs::write(&other, b"other-synthetic-file").unwrap();
    let wrong = CredentialAlias { scope: b.clone(),
        file_identity: AppContainerProfile::capture_program_identity(&other).unwrap() };
    assert!(matches!(CredentialBinding::remove_quiescent_alias(binding,
        &root, &wrong, &complete), Err(CredentialError::IdentityChanged)),
        "wrong file ID witness must not unlink the registered name");
    assert!(alias_b.path().is_file(), "failed removal leaves exact alias intact");
    let binding = CredentialBinding::open_registered(&root, &source, &home_id,
        &source_id, &complete).unwrap();
    let alias_b = binding.alias(&b, &complete).unwrap();
    let receipt = CredentialBinding::remove_quiescent_alias(binding,
        &root, &alias_b, &complete).expect("one quiescent alias disposition");
    assert_eq!(receipt.source_identity, source_id);
    assert_eq!(receipt.removed, b);
    assert_eq!(receipt.remaining_aliases, vec![a.clone()]);
    assert_eq!(receipt.remaining_links, 2);
    assert!(!alias_b.path().exists() && a.root.join(AUTH_NAME).is_file());
    let observed = CredentialBinding::verify_removed_alias(&root, &source,
        &home_id, &source_id, &b, &[a.clone()]).expect("read-only recovery receipt");
    assert_eq!(observed.remaining_links, 2);
    fs::write(alias_b.path(), b"different-synthetic-object").unwrap();
    assert!(matches!(CredentialBinding::verify_removed_alias(&root, &source,
        &home_id, &source_id, &b, &[a]), Err(CredentialError::Invalid(
            "removed alias name still exists"))),
        "replacement cannot be mistaken for a completed remove intent");
    drop(root);
    fs::remove_dir_all(requested).unwrap();
}

#[test]
fn owner_account_observer_reads_only_registered_auth_object_and_fresh_runtime() {
    let (root, requested) = test_root();
    let base = root.canonical_root().canonical_path.clone();
    let instance = base.join("instanceA");
    let scope_path = instance.join("private-history").join("scopeA");
    let runtime = instance.join("gogoke-login-runtime");
    fs::create_dir_all(&scope_path).unwrap();
    fs::create_dir(&runtime).unwrap();
    let source = instance.join(AUTH_NAME);
    fs::write(&source, b"synthetic-nonsecret-auth").unwrap();
    let source_id = AppContainerProfile::capture_program_identity(&source).unwrap();
    let instance_id = inspect_root(&instance).unwrap().identity;
    let runtime_id = inspect_root(&runtime).unwrap().identity;
    let scope = scope(&scope_path);
    let binding = CredentialBinding::open_registered(&root, &source,
        &instance_id, &source_id, &[]).expect("registered metadata holder");
    let name = format!("Gogoke37.OwnerAccountObserver.{}", std::process::id());
    let observer = AppContainerProfile::ensure_for_cli(&name, true).unwrap();
    assert!(observer.grant_bound_owner_account_observer(&root, &instance,
        &instance_id, &runtime, &runtime_id, &binding, &[]).is_err(),
        "unknown live state must not silently reconstruct the source DACL");
    AppContainerProfile::prepare_quiescent_owner_account_source(&root,
        &instance, &instance_id, &binding).expect("explicit quiescent source preparation");
    let alias = binding.create_alias(&root, scope.clone(), &[])
        .expect("registered existing credential object alias");
    let history = scope_path.join("private-history.jsonl");
    fs::write(&history, b"synthetic-history-not-for-observer").unwrap();
    observer.grant_bound_owner_account_observer(&root, &instance,
        &instance_id, &runtime, &runtime_id, &binding, &[scope.clone()])
        .expect("narrow account/read observer ACL");
    observer.verify_bound_owner_account_observer(&root, &instance,
        &instance_id, &runtime, &runtime_id, &binding, &[scope.clone()])
        .expect("exact source/runtime and no history ACE");
    assert!(binding.verify_registered_aliases(&[]).is_err(),
        "cached binding cannot replace F's complete alias set");
    let runner = prepare_runner(&observer, &base.join("observer-runner"));
    let source_read = run_child_path(&name, &runner, &source, false);
    assert_eq!(source_read, "read=OK write=OK", "actual LPAC source read: {source_read}");
    let source_write = run_child_path(&name, &runner, &source, true);
    assert!(source_write.starts_with("read=OK write=ERR:Some(5);"),
        "account/read observer cannot write registered auth: {source_write}");
    let private_read = run_child_path(&name, &runner, &history, false);
    assert!(private_read.starts_with("read=ERR:Some(5);") && private_read.ends_with(" write=OK"),
        "actual LPAC history denial: {private_read}");
    assert!(alias.path().is_file(), "the registered hardlink remains present");
    let history_id = AppContainerProfile::capture_program_identity(&history).unwrap();
    observer.grant_bound_program(&history, &history_id)
        .expect("controlled residual history ACE");
    assert!(observer.verify_bound_owner_account_observer(&root, &instance,
        &instance_id, &runtime, &runtime_id, &binding, &[scope]).is_err(),
        "a residual observer ACE on history must invalidate the exact witness");
    drop((observer, binding));
    drop(root);
    fs::remove_dir_all(requested).unwrap();
    #[link(name = "userenv")]
    extern "system" { fn DeleteAppContainerProfile(name: *const u16) -> i32; }
    let wide: Vec<u16> = OsStr::new(&name).encode_wide().chain(Some(0)).collect();
    assert!(unsafe { DeleteAppContainerProfile(wide.as_ptr()) } >= 0);
}

#[test]
fn empty_owner_account_observer_initializes_only_fresh_runtime() {
    let (root, requested) = test_root();
    let base = root.canonical_root().canonical_path.clone();
    let home = base.join("empty-instance");
    let history_dir = home.join("sessions").join("2026");
    let runtime = home.join("gogoke-login-runtime");
    fs::create_dir_all(&history_dir).unwrap();
    fs::create_dir(&runtime).unwrap();
    let history = history_dir.join("synthetic-history.jsonl");
    fs::write(&history, b"synthetic-history-not-for-account-observer").unwrap();
    let home_id = inspect_root(&home).unwrap().identity;
    let runtime_id = inspect_root(&runtime).unwrap().identity;
    let name = format!("Gogoke37.EmptyOwnerAccountObserver.{}", std::process::id());
    let observer = AppContainerProfile::ensure_for_cli(&name, true).unwrap();
    observer.grant_bound_owner_account_empty(&root, &home, &home_id, &runtime, &runtime_id)
        .expect("empty HOME admits only traverse and fresh runtime");
    let runner = prepare_runner(&observer, &base.join("empty-observer-runner"));
    let initialized = runtime.join("synthetic-account-init.json");
    let result = run_child_path_with_home(&name, &runner, &initialized, false, Some(&home));
    assert_eq!(result, "create=OK reopen=Ok(true)",
        "actual LPAC creates and reopens only in its HOME runtime: {result}");
    let private_read = run_child_path(&name, &runner, &history, false);
    assert!(private_read.starts_with("read=ERR:Some(5);") && private_read.ends_with(" write=OK"),
        "actual LPAC history access denied: {private_read}");
    observer.verify_bound_owner_account_empty(&root, &home, &home_id, &runtime, &runtime_id)
        .expect("no auth source and no observer SID elsewhere");
    fs::write(home.join(AUTH_NAME), b"synthetic-new-source").unwrap();
    assert!(matches!(observer.verify_bound_owner_account_empty(&root, &home,
        &home_id, &runtime, &runtime_id), Err(CredentialError::Invalid(
            "empty account auth object exists"))), "present source requires binding path");
    drop(observer);
    drop(root);
    fs::remove_dir_all(requested).unwrap();
    #[link(name = "userenv")]
    extern "system" { fn DeleteAppContainerProfile(name: *const u16) -> i32; }
    let wide: Vec<u16> = OsStr::new(&name).encode_wide().chain(Some(0)).collect();
    assert!(unsafe { DeleteAppContainerProfile(wide.as_ptr()) } >= 0);
}

#[test]
fn legacy_owner_login_whole_home_grant_migrates_to_exact_account_observer() {
    let (root, requested) = test_root();
    let base = root.canonical_root().canonical_path.clone();
    let home = base.join("legacy-instance");
    let history_dir = home.join("sessions").join("2026");
    fs::create_dir_all(&history_dir).unwrap();
    let history = history_dir.join("synthetic-history.jsonl");
    fs::write(&history, b"synthetic-history-not-for-new-observer").unwrap();
    let source = home.join(AUTH_NAME);
    fs::write(&source, b"synthetic-nonsecret-auth").unwrap();
    let home_id = inspect_root(&home).unwrap().identity;
    let source_id = AppContainerProfile::capture_program_identity(&source).unwrap();
    let name = format!("Gogoke37.LegacyOwnerLoginMigration.{}", std::process::id());
    let observer = AppContainerProfile::ensure_for_cli(&name, true).unwrap();
    let other_name = format!("Gogoke37.LegacyOtherSid.{}", std::process::id());
    let other_profile = AppContainerProfile::ensure_for_cli(&other_name, true).unwrap();
    observer.grant_bound_tree(&home, &home_id, true)
        .expect("actual legacy whole-HOME inherited RW grant");
    observer.verify_bound_tree_grant(&home, &home_id, true)
        .expect("legacy child ACEs came from old grant primitive");
    let history_id = AppContainerProfile::capture_program_identity(&history).unwrap();
    other_profile.grant_bound_program(&history, &history_id)
        .expect("independent SID on legacy history");
    let binding = CredentialBinding::open_registered(&root, &source,
        &home_id, &source_id, &[]).expect("registered metadata-only source");
    observer.migrate_legacy_owner_login_grant(&root, &home, &home_id,
        Some((&binding, &[]))).expect("revoke only known old observer SID grants");
    assert!(!binding.acl_prepared_in_this_holder().unwrap(),
        "legacy revoke cannot claim protected credential baseline preparation");
    other_profile.verify_bound_program_grant(&history, &history_id)
        .expect("migration preserves another SID's exact ACL");
    AppContainerProfile::prepare_quiescent_owner_account_source(&root,
        &home, &home_id, &binding).expect("quiescent source protection after migration");
    let runtime = home.join("gogoke-login-runtime");
    fs::create_dir(&runtime).unwrap();
    let runtime_id = inspect_root(&runtime).unwrap().identity;
    observer.grant_bound_owner_account_observer(&root, &home, &home_id,
        &runtime, &runtime_id, &binding, &[])
        .expect("exact source-read and fresh-runtime grant after legacy cleanup");
    let runner = prepare_runner(&observer, &base.join("migrated-observer-runner"));
    let source_read = run_child_path(&name, &runner, &source, false);
    assert_eq!(source_read, "read=OK write=OK", "actual LPAC auth read: {source_read}");
    let initialized = runtime.join("synthetic-account-init.json");
    let created = run_child_path_with_home(&name, &runner, &initialized, false, Some(&home));
    assert_eq!(created, "create=OK reopen=Ok(true)",
        "actual LPAC runtime creation after migration: {created}");
    let private_read = run_child_path(&name, &runner, &history, false);
    assert!(private_read.starts_with("read=ERR:Some(5);") && private_read.ends_with(" write=OK"),
        "legacy history access must be revoked: {private_read}");
    observer.verify_bound_owner_account_observer(&root, &home, &home_id,
        &runtime, &runtime_id, &binding, &[])
        .expect("no broad observer SID remains on history");
    drop((observer, other_profile, binding));
    drop(root);
    fs::remove_dir_all(requested).unwrap();
    #[link(name = "userenv")]
    extern "system" { fn DeleteAppContainerProfile(name: *const u16) -> i32; }
    for profile_name in [name, other_name] {
        let wide: Vec<u16> = OsStr::new(&profile_name).encode_wide().chain(Some(0)).collect();
        assert!(unsafe { DeleteAppContainerProfile(wide.as_ptr()) } >= 0);
    }
}
