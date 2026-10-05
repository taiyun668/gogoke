//! Catalog executable exceptions must not relax any single-link data boundary.
use super::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn fixture(name: &str) -> PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-catalog-{name}-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}

#[test]
fn catalog_program_hardlinks_bind_one_file_with_exact_rx_and_keep_data_strict() {
    let base = fixture("acl");
    let program = base.join("catalog.exe");
    let alias = base.join("installed.exe");
    let neighbor = base.join("neighbor.exe");
    let reparse = base.join("reparse.exe");
    std::fs::write(&program, b"fixed catalog program bytes").unwrap();
    std::fs::hard_link(&program, &alias).unwrap();
    std::fs::write(&neighbor, b"different physical object").unwrap();
    std::os::windows::fs::symlink_file(&program, &reparse).unwrap();
    let profile = AppContainerProfile::derived_for_test("Gogoke37.CatalogProgramAcl").unwrap();
    let identity = AppContainerProfile::capture_catalog_program_identity(&program).unwrap();
    assert_eq!(AppContainerProfile::capture_catalog_program_identity(&alias).unwrap(), identity);
    let object = open_catalog_program(&program, READ_CONTROL).unwrap();
    assert_eq!(file_information(object.0).unwrap().links, 2, "fixture must be a real hardlink");
    let other_identity = AppContainerProfile::capture_program_identity(&neighbor).unwrap();
    assert!(matches!(profile.grant_bound_catalog_program(&program, &other_identity),
        Err(IsolationError::AclWitnessMismatch)));
    assert!(package_aces(object.0, profile.sid).unwrap().is_empty(), "wrong FileID must not grant");
    assert!(matches!(AppContainerProfile::capture_catalog_program_identity(&reparse),
        Err(IsolationError::DirectoryNotPhysical)));
    assert!(profile.grant_bound_catalog_program(&reparse, &identity).is_err());
    assert!(profile.verify_bound_catalog_program_grant(&reparse, &identity).is_err());
    assert!(package_aces(object.0, profile.sid).unwrap().is_empty(), "reparse must not grant target");
    assert!(AppContainerProfile::capture_program_identity(&program).is_err());
    assert!(profile.grant_bound_program(&program, &identity).is_err());
    assert!(profile.verify_bound_program_grant(&program, &identity).is_err());
    assert!(open_physical_object(&program, false, READ_CONTROL).is_err(),
        "the generic physical-file opener still rejects a data hardlink");
    std::fs::remove_file(&reparse).unwrap();
    let base_identity = crate::root::inspect_root(&base).unwrap().identity;
    assert!(matches!(profile.grant_bound_tree(&base, &base_identity, true),
        Err(IsolationError::DirectoryNotPhysical)), "data tree must still reject hardlinks");
    let witness = profile.grant_bound_catalog_program(&program, &identity).unwrap();
    assert_eq!(witness.identity, identity);
    assert_eq!(witness.rights, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE);
    assert_eq!(witness.inheritance, NO_INHERITANCE);
    assert_eq!(profile.verify_bound_catalog_program_grant(&alias, &identity).unwrap(), witness);
    // Inspect the actual ACE, independently of executable image-section locks.
    let aces = package_aces(object.0, profile.sid).unwrap();
    assert_eq!(aces, vec![(GRANT_ACCESS, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE, NO_INHERITANCE)]);
    for (_, rights, _) in aces {
        assert_eq!(rights & (0x0000_0116 | DELETE_ACCESS | WRITE_DAC | 0x0008_0000), 0,
            "no WRITE_DATA/APPEND/WRITE_EA/WRITE_ATTRIBUTES/DELETE/WRITE_DAC/WRITE_OWNER");
    }
    let other = open_physical_object(&neighbor, false, READ_CONTROL).unwrap();
    assert!(package_aces(other.0, profile.sid).unwrap().is_empty(), "neighbor receives no grant");
    assert!(profile.verify_bound_catalog_program_grant(&neighbor, &identity).is_err());
    assert!(profile.verify_bound_program_grant(&alias, &identity).is_err(),
        "catalog grant cannot relax old single-link verification");
    drop(other);
    drop(object);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn actual_catalog_hardlink_program_prepares_suspended_lpac_activates_and_stops() {
    use crate::process::{NativeBinding, PrepareRequest, ProcessCustodian, ProcessLaunch, StopBudgets};
    const DIGEST: &str = "sha256:180d7b279455e8b89d4353a5146447be2f80b80fb0db14bdc6dd9cb98c0aef09";
    let program = crate::store::instance::locate_pinned_program("claude", DIGEST, "2.1.196")
        .expect("required real cloud catalog CLI, not a copied test executable");
    let original = program.parent().unwrap().parent().unwrap()
        .join("node_modules/@anthropic-ai/claude-code-win32-x64/claude.exe");
    let identity = AppContainerProfile::capture_catalog_program_identity(&program).unwrap();
    assert_eq!(AppContainerProfile::capture_catalog_program_identity(&original).unwrap(), identity);
    let object = open_catalog_program(&program, READ_CONTROL).unwrap();
    assert_eq!(file_information(object.0).unwrap().links, 2, "catalog fixture must preserve links=2");
    assert!(crate::store::instance::locate_pinned_program("claude", &format!("sha256:{}", "0".repeat(64)),
        "2.1.196").is_err(), "wrong pin fails before program grant");
    let home = fixture("process");
    let name = format!("Gogoke37.CatalogProcess{}{}", std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos());
    let profile = AppContainerProfile::ensure_for_cli(&name, false).unwrap();
    assert!(package_aces(object.0, profile.sid).unwrap().is_empty(), "wrong pin granted no new package ACE");
    profile.grant_fresh_session_directory(&home).unwrap();
    profile.grant_bound_catalog_program(&program, &identity).unwrap();
    assert_eq!(package_aces(object.0, profile.sid).unwrap(),
        vec![(GRANT_ACCESS, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE, NO_INHERITANCE)]);
    let mut launch = ProcessLaunch::new(&program);
    launch.arguments = vec!["--version".into()];
    launch.current_directory = Some(home.clone());
    launch.protocol_stdio = true;
    launch.app_container_profile = Some(name.clone());
    launch.app_container_cli_identity_services = true;
    launch.environment = Some(vec![
        ("SystemRoot".into(), std::env::var("SystemRoot").unwrap()),
        ("USERPROFILE".into(), home.to_string_lossy().into_owned()),
        ("LOCALAPPDATA".into(), home.to_string_lossy().into_owned()),
        ("CLAUDE_CONFIG_DIR".into(), home.to_string_lossy().into_owned()),
    ]);
    let request = PrepareRequest { launch, binding: NativeBinding {
        binary_digest_sha256: DIGEST.into(), profile_id: name.clone(),
        domain_id: "catalog-test".into(), generation: "1".into() } };
    let mut custodian = ProcessCustodian::new().unwrap();
    let prepared = custodian.prepare(&request).expect("actual image SHA, exact LPAC token and Job checked suspended");
    assert!(prepared.identity.pid > 0 && prepared.identity.creation_time_100ns > 0);
    assert!(custodian.active(&prepared.ticket).is_none(), "still suspended before activation");
    profile.verify_bound_catalog_program_grant(&program, &identity).unwrap();
    custodian.activate(&prepared).unwrap();
    let active = custodian.active(&prepared.ticket).unwrap();
    assert!(active.handles_are_non_inheritable().unwrap());
    let output = active.read_protocol_frame(Duration::from_secs(30)).unwrap_or_else(|error|
        panic!("real catalog CLI version: {error}; exit={:?}; stderr={}", active.exit_code(), active.stderr_tail()));
    assert_eq!(String::from_utf8_lossy(&output).trim(), "2.1.196 (Claude Code)");
    let proof = custodian.stop(&prepared.ticket, StopBudgets::production(), || Ok(())).unwrap();
    assert!(proof.parent_exited && proof.writer_fence_verified, "exact Job stop: {proof:?}");
    assert_eq!(proof.active_job_processes, Some(0));
    assert!(proof.errors.is_empty(), "stop has no unconfirmed errors: {proof:?}");
    profile.verify_bound_catalog_program_grant(&original, &identity).unwrap();
    drop(custodian);
    drop(object);
    drop(profile);
    std::fs::remove_dir_all(home).unwrap();
}
