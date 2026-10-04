//! Synthetic Windows-only qualification of creation-time history ACLs.
//! A passing test means the experiment ran; its printed candidate verdict is
//! the result. It is not a vendor CLI, credential, or product acceptance test.

use super::*;
use crate::process::{prepare_and_activate_with_suspended_test, ProcessLaunch};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::os::windows::ffi::OsStrExt;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TOKEN_ADJUST_DEFAULT: u32 = 0x0080;
const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_DEFAULT_DACL_CLASS: u32 = 6;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
const FILE_ALL_ACCESS: u32 = 0x001f_01ff;
const CONTAINER_INHERIT: u32 = 2;
const TEST_HELPER: &str = "process::session::isolation::history_acl_qualification::synthetic_history_acl_child";

#[repr(C)]
struct TokenDefaultDacl { default_dacl: *mut c_void }

#[link(name = "kernel32")]
extern "system" { fn GetCurrentProcess() -> Handle; }

#[link(name = "advapi32")]
extern "system" {
    fn SetTokenInformation(token: Handle, class: u32, input: *const c_void, length: u32) -> i32;
}

fn token_user(token: Handle) -> Result<(Vec<usize>, *mut c_void), String> {
    let mut size = 0;
    unsafe { GetTokenInformation(token, TOKEN_USER_CLASS, ptr::null_mut(), 0, &mut size); }
    if size < size_of::<SidAndAttributes>() as u32 || size > 4096 {
        return Err(format!("TokenUser size={size}, Win32={}", io::Error::last_os_error()));
    }
    let mut words = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    let mut returned = 0;
    if unsafe { GetTokenInformation(token, TOKEN_USER_CLASS, words.as_mut_ptr().cast(),
        size, &mut returned) } == 0 {
        return Err(format!("TokenUser read: {}", io::Error::last_os_error()));
    }
    if returned < size_of::<SidAndAttributes>() as u32 {
        return Err(format!("TokenUser returned only {returned} bytes"));
    }
    let sid = unsafe { (*(words.as_ptr().cast::<SidAndAttributes>())).sid };
    if sid.is_null() { return Err("TokenUser SID was null".into()); }
    Ok((words, sid))
}

fn build_acl(entries: &[(*mut c_void, u32, u32)]) -> Result<LocalAllocation, String> {
    let mut grants: Vec<ExplicitAccessW> = entries.iter().map(|(sid, rights, inheritance)|
        ExplicitAccessW { permissions: *rights, access_mode: GRANT_ACCESS,
            inheritance: *inheritance, trustee: TrusteeW { multiple: ptr::null_mut(),
                multiple_operation: 0, form: TRUSTEE_IS_SID, kind: TRUSTEE_IS_UNKNOWN,
                name: sid.cast() } }).collect();
    let mut raw = ptr::null_mut();
    let status = unsafe { SetEntriesInAclW(grants.len() as u32, grants.as_mut_ptr(),
        ptr::null_mut(), &mut raw) };
    if status != 0 { return Err(format!("SetEntriesInAclW Win32={status}")); }
    if raw.is_null() { return Err("SetEntriesInAclW returned a null ACL".into()); }
    Ok(LocalAllocation(raw))
}

fn acl_has_sid(acl: *mut c_void, sid: *mut c_void) -> Result<bool, String> {
    if acl.is_null() { return Err("null DACL".into()); }
    let mut info = AclSizeInformation { ace_count: 0, acl_bytes_in_use: 0, acl_bytes_free: 0 };
    if unsafe { GetAclInformation(acl, (&mut info as *mut AclSizeInformation).cast(),
        size_of::<AclSizeInformation>() as u32, ACL_SIZE_INFORMATION_CLASS) } == 0 {
        return Err(format!("GetAclInformation: {}", io::Error::last_os_error()));
    }
    for index in 0..info.ace_count {
        let mut ace = ptr::null_mut();
        if unsafe { GetAce(acl, index, &mut ace) } == 0 {
            return Err(format!("GetAce({index}): {}", io::Error::last_os_error()));
        }
        if ace.is_null() { return Err(format!("GetAce({index}) returned null")); }
        let header = unsafe { &*ace.cast::<AceHeader>() };
        if header.ace_type == ACCESS_ALLOWED_ACE_TYPE && header.ace_size >= 16 &&
            unsafe { EqualSid(ace.cast::<u8>().add(8).cast(), sid) } != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn set_child_default_dacl(process: Handle, profile: &AppContainerProfile) -> Result<(), String> {
    let mut raw = ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY | TOKEN_ADJUST_DEFAULT, &mut raw) } == 0 {
        return Err(format!("OpenProcessToken suspended child: {}", io::Error::last_os_error()));
    }
    let token = Token(raw);
    let (_user_buffer, user) = token_user(token.0)?;
    let acl = build_acl(&[(user, FILE_ALL_ACCESS, NO_INHERITANCE),
        (profile.sid, FILE_ALL_ACCESS, NO_INHERITANCE)])?;
    let input = TokenDefaultDacl { default_dacl: acl.0 };
    if unsafe { SetTokenInformation(token.0, TOKEN_DEFAULT_DACL_CLASS,
        (&input as *const TokenDefaultDacl).cast(), size_of::<TokenDefaultDacl>() as u32) } == 0 {
        return Err(format!("SetTokenInformation(TokenDefaultDacl): {}", io::Error::last_os_error()));
    }
    let mut size = 0;
    unsafe { GetTokenInformation(token.0, TOKEN_DEFAULT_DACL_CLASS,
        ptr::null_mut(), 0, &mut size); }
    if size < size_of::<TokenDefaultDacl>() as u32 || size > 4096 {
        return Err(format!("TokenDefaultDacl readback size={size}, Win32={}", io::Error::last_os_error()));
    }
    let mut words = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    let mut returned = 0;
    if unsafe { GetTokenInformation(token.0, TOKEN_DEFAULT_DACL_CLASS,
        words.as_mut_ptr().cast(), size, &mut returned) } == 0 {
        return Err(format!("TokenDefaultDacl readback: {}", io::Error::last_os_error()));
    }
    if returned < size_of::<TokenDefaultDacl>() as u32 ||
        !acl_has_sid(unsafe { (*words.as_ptr().cast::<TokenDefaultDacl>()).default_dacl }, profile.sid)? {
        return Err("TokenDefaultDacl readback lost the exact profile SID".into());
    }
    Ok(())
}

fn set_protected_synthetic_acl(path: &Path, user: *mut c_void,
    profiles: &[&AppContainerProfile], inheritance: u32) -> Result<(), String> {
    let mut entries = vec![(user, FILE_ALL_ACCESS, inheritance)];
    entries.extend(profiles.iter().map(|profile| (profile.sid,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE, inheritance)));
    let acl = build_acl(&entries)?;
    let object = open_directory(path, READ_CONTROL | WRITE_DAC).map_err(|error| error.to_string())?;
    let status = unsafe { SetSecurityInfo(object.0, FILE_OBJECT,
        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), acl.0, ptr::null_mut()) };
    if status != 0 { return Err(format!("SetSecurityInfo({path:?}) Win32={status}")); }
    for profile in profiles {
        let actual = package_aces(object.0, profile.sid).map_err(|error| error.to_string())?;
        if actual != vec![(GRANT_ACCESS,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE, inheritance)] {
            return Err(format!("protected ACL readback {path:?} profile={:?}: {actual:?}",
                profile.package_sid_string().map_err(|error| error.to_string())?));
        }
    }
    Ok(())
}

fn path_for(root: &Path, layout: &str, owner: &str) -> PathBuf {
    if layout == "ci_nested" {
        root.join("sessions").join("2026").join("10").join("03")
            .join(format!("{owner}.jsonl"))
    } else { root.join(format!("{owner}.jsonl")) }
}

fn record(report: &mut Vec<String>, name: &str, result: io::Result<()>) {
    match result {
        Ok(()) => report.push(format!("{name}=OK")),
        Err(error) => report.push(format!("{name}=ERR:{:?};{}", error.raw_os_error(), error)),
    }
}

fn try_write_dac(path: &Path) -> io::Result<()> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let raw = unsafe { CreateFileW(wide.as_ptr(), WRITE_DAC, FILE_SHARE_ALL,
        ptr::null(), OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, ptr::null_mut()) };
    if raw as isize == -1 { return Err(io::Error::last_os_error()); }
    drop(Token(raw));
    Ok(())
}

#[test]
fn synthetic_history_acl_child() {
    let Ok(role) = std::env::var("GOGOKE_TEST_HISTORY_ACL_ROLE") else { return; };
    let root = PathBuf::from(std::env::var_os("GOGOKE_TEST_HISTORY_ACL_ROOT").expect("synthetic root"));
    let layout = std::env::var("GOGOKE_TEST_HISTORY_ACL_LAYOUT").expect("layout");
    let report_path = PathBuf::from(std::env::var_os("GOGOKE_TEST_HISTORY_ACL_REPORT").expect("report"));
    let a = path_for(&root, &layout, "a");
    let b = path_for(&root, &layout, "b");
    let auth = root.join("synthetic-auth.txt");
    let mut report = Vec::new();
    match role.as_str() {
        "a_create" | "b_create" => {
            let (own, other, prefix) = if role == "a_create" { (&a, &b, "A") }
                else { (&b, &a, "B") };
            if role == "b_create" {
                record(&mut report, "B_read_A", fs::read(other).map(|_| ()));
                record(&mut report, "B_write_A", OpenOptions::new().write(true)
                    .open(other).map(|_| ()));
                record(&mut report, "B_dac_A", try_write_dac(other));
            }
            record(&mut report, &format!("{prefix}_auth_read"), fs::read(&auth).map(|_| ()));
            record(&mut report, &format!("{prefix}_auth_append"), OpenOptions::new()
                .append(true).open(&auth).and_then(|mut file| {
                    use std::io::Write;
                    file.write_all(prefix.as_bytes())
                }));
            let mkdir = own.parent().expect("history parent");
            record(&mut report, &format!("{prefix}_mkdir"), fs::create_dir_all(mkdir));
            record(&mut report, &format!("{prefix}_create"), fs::write(own,
                format!("synthetic {prefix} history").as_bytes()));
            record(&mut report, &format!("{prefix}_reopen"), fs::read(own).and_then(|bytes|
                if bytes == format!("synthetic {prefix} history").as_bytes() { Ok(()) }
                else { Err(io::Error::new(io::ErrorKind::InvalidData, "wrong synthetic history bytes")) }));
            let grand_report = report_path.with_extension("grand.txt");
            let status = Command::new(std::env::current_exe().expect("exact test image"))
                .args(["--exact", TEST_HELPER, "--nocapture"])
                .env("GOGOKE_TEST_HISTORY_ACL_ROLE", if role == "a_create" { "a_grand" } else { "b_grand" })
                .env("GOGOKE_TEST_HISTORY_ACL_REPORT", &grand_report)
                .status();
            record(&mut report, &format!("{prefix}_grandchild"), status.and_then(|status|
                if status.success() { Ok(()) } else {
                    Err(io::Error::new(io::ErrorKind::Other, format!("grandchild exit {status}")))
                }));
        }
        "a_check" | "a_grand" | "b_grand" => {
            if role == "a_check" {
                record(&mut report, "A_read_B", fs::read(&b).map(|_| ()));
                record(&mut report, "A_dac_B", try_write_dac(&b));
                record(&mut report, "A_reopen_later", fs::read(&a).map(|_| ()));
            } else {
                let (own, other, prefix) = if role == "a_grand" { (&a, &b, "A") }
                    else { (&b, &a, "B") };
                record(&mut report, &format!("{prefix}_grand_own"), fs::read(own).map(|_| ()));
                if role == "b_grand" {
                    record(&mut report, "B_grand_read_A", fs::read(other).map(|_| ()));
                }
                record(&mut report, &format!("{prefix}_grand_auth"), fs::read(&auth).map(|_| ()));
            }
        }
        _ => panic!("unknown synthetic history role"),
    }
    fs::write(report_path, report.join("\n")).expect("report real LPAC file operations");
}

fn read_report(path: &Path) -> HashMap<String, String> {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read child report {path:?}: {error}"))
        .lines().map(|line| {
            let (key, value) = line.split_once('=').expect("report key=value");
            (key.to_owned(), value.to_owned())
        }).collect()
}

fn run_child(profile: &AppContainerProfile, name: &str, executable: &Path,
    runner: &Path, root: &Path, layout: &str, role: &str) -> HashMap<String, String> {
    let report = runner.join(format!("{role}.txt"));
    let mut launch = ProcessLaunch::new(executable);
    launch.current_directory = Some(runner.to_path_buf());
    launch.protocol_stdio = true;
    launch.app_container_profile = Some(name.to_owned());
    launch.app_container_cli_identity_services = true;
    launch.app_container_internet_client = true;
    launch.environment = Some(vec![
        ("SystemRoot".into(), std::env::var("SystemRoot").expect("SystemRoot")),
        ("USERPROFILE".into(), root.to_string_lossy().into_owned()),
        ("LOCALAPPDATA".into(), runner.to_string_lossy().into_owned()),
        ("GOGOKE_TEST_HISTORY_ACL_ROLE".into(), role.into()),
        ("GOGOKE_TEST_HISTORY_ACL_ROOT".into(), root.to_string_lossy().into_owned()),
        ("GOGOKE_TEST_HISTORY_ACL_LAYOUT".into(), layout.into()),
        ("GOGOKE_TEST_HISTORY_ACL_REPORT".into(), report.to_string_lossy().into_owned()),
    ]);
    launch.arguments = vec!["--exact".into(), TEST_HELPER.into(), "--nocapture".into()];
    let managed = prepare_and_activate_with_suspended_test(&launch,
        |process| set_child_default_dacl(process, profile), |_| Ok(()))
        .unwrap_or_else(|error| panic!("real suspended {role} LPAC launch: {error}"));
    assert!(managed.wait(Duration::from_secs(20)).expect("LPAC child exit"),
        "{role} LPAC child timed out; stderr={}", managed.stderr_tail());
    assert_eq!(managed.exit_code().expect("LPAC exit code"), Some(0),
        "{role} LPAC child failed; stderr={}", managed.stderr_tail());
    drop(managed);
    read_report(&report)
}

fn leaf_aces(path: &Path, profile: &AppContainerProfile) -> Vec<(u32, u32, u32)> {
    let object = open_physical_object(path, false, READ_CONTROL)
        .unwrap_or_else(|error| panic!("read physical synthetic leaf {path:?}: {error}"));
    package_aces(object.0, profile.sid).expect("read actual leaf DACL")
}

#[test]
fn synthetic_history_acl_creation_time_qualification() {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
    let base = std::env::temp_dir().join(format!("gogoke-history-acl-{}-{nonce}", std::process::id()));
    fs::create_dir(&base).expect("isolated synthetic test root");
    let source_exe = std::env::current_exe().expect("exact cloud native test image");
    let a_name = format!("Gogoke37.HistA{}.{nonce}", std::process::id());
    let b_name = format!("Gogoke37.HistB{}.{nonce}", std::process::id());
    let a_profile = AppContainerProfile::ensure_for_cli(&a_name, true).expect("A production LPAC profile");
    let b_profile = AppContainerProfile::ensure_for_cli(&b_name, true).expect("B production LPAC profile");
    assert_ne!(a_profile.sid_identity().unwrap(), b_profile.sid_identity().unwrap());
    assert_eq!(a_profile.security_capabilities().capability_count, 3);
    assert_eq!(b_profile.security_capabilities().capability_count, 3);
    assert!(a_profile.combined_capabilities.iter().zip(&b_profile.combined_capabilities)
        .all(|(a, b)| a.attributes == b.attributes && unsafe { EqualSid(a.sid, b.sid) } != 0),
        "both real LPAC profiles must have the same three effective capabilities");
    let mut current_token = ptr::null_mut();
    if unsafe { OpenProcessToken(unsafe { GetCurrentProcess() }, TOKEN_QUERY, &mut current_token) } == 0 {
        panic!("open ordinary-user token: {}", io::Error::last_os_error());
    }
    let current_token = Token(current_token);
    let (_user_buffer, user) = token_user(current_token.0).expect("ordinary user SID");
    for (layout, inheritance) in [("no_inherit", NO_INHERITANCE), ("ci_nested", CONTAINER_INHERIT)] {
        let fixture = base.join(layout);
        let runner_a = fixture.join("runner-a");
        let runner_b = fixture.join("runner-b");
        let root = fixture.join("shared-home");
        fs::create_dir(&fixture).unwrap();
        fs::create_dir(&runner_a).unwrap();
        fs::create_dir(&runner_b).unwrap();
        fs::create_dir(&root).unwrap();
        let auth = root.join("synthetic-auth.txt");
        fs::write(&auth, b"nonsecret shared fixture").unwrap();
        a_profile.grant_fresh_session_directory(&runner_a).expect("A test executable directory");
        b_profile.grant_fresh_session_directory(&runner_b).expect("B test executable directory");
        let exe_a = runner_a.join("synthetic-a.exe");
        let exe_b = runner_b.join("synthetic-b.exe");
        fs::copy(&source_exe, &exe_a).expect("exact A test image copy");
        fs::copy(&source_exe, &exe_b).expect("exact B test image copy");
        set_protected_synthetic_acl(&auth, user, &[&a_profile, &b_profile], NO_INHERITANCE)
            .expect("shared nonsecret auth object ACL");
        set_protected_synthetic_acl(&root, user, &[&a_profile, &b_profile], inheritance)
            .expect("synthetic HOME root ACL");
        let mut reports = HashMap::new();
        reports.extend(run_child(&a_profile, &a_name, &exe_a, &runner_a, &root, layout, "a_create"));
        let a_grand = runner_a.join("a_create.grand.txt");
        if a_grand.is_file() { reports.extend(read_report(&a_grand)); }
        reports.extend(run_child(&b_profile, &b_name, &exe_b, &runner_b, &root, layout, "b_create"));
        let b_grand = runner_b.join("b_create.grand.txt");
        if b_grand.is_file() { reports.extend(read_report(&b_grand)); }
        reports.extend(run_child(&a_profile, &a_name, &exe_a, &runner_a, &root, layout, "a_check"));
        let a_leaf = path_for(&root, layout, "a");
        let b_leaf = path_for(&root, layout, "b");
        let a_dacl = if a_leaf.is_file() { Some((leaf_aces(&a_leaf, &a_profile), leaf_aces(&a_leaf, &b_profile))) }
            else { None };
        let b_dacl = if b_leaf.is_file() { Some((leaf_aces(&b_leaf, &b_profile), leaf_aces(&b_leaf, &a_profile))) }
            else { None };
        let expected = ["A_auth_read", "A_auth_append", "A_mkdir", "A_create", "A_reopen",
            "A_grandchild", "A_grand_own", "A_grand_auth", "B_auth_read", "B_auth_append",
            "B_mkdir", "B_create", "B_reopen", "B_grandchild", "B_grand_own", "B_grand_auth",
            "A_reopen_later"];
        let denied = ["B_read_A", "B_write_A", "B_dac_A", "B_grand_read_A", "A_read_B", "A_dac_B"];
        let dacl_private = [a_dacl.as_ref(), b_dacl.as_ref()].into_iter().all(|value|
            matches!(value, Some((own, other)) if own.iter().any(|(mode, _, flags)|
                *mode == GRANT_ACCESS && *flags & INHERITED_ACE == 0) && other.is_empty()));
        let qualified = expected.iter().all(|key| reports.get(*key).is_some_and(|value| value == "OK"))
            && denied.iter().all(|key| reports.get(*key).is_some_and(|value| value.starts_with("ERR:Some(5);")))
            && dacl_private;
        println!("HISTORY_ACL_QUALIFICATION layout={layout} verdict={} reports={reports:?} a_leaf_dacl={a_dacl:?} b_leaf_dacl={b_dacl:?}",
            if qualified { "CANDIDATE_QUALIFIED" } else { "CANDIDATE_REJECTED" });
        // A rejected variant is a valid experiment, never a product ISO PASS.
    }
    drop(current_token);
    drop(a_profile);
    drop(b_profile);
    for name in [&a_name, &b_name] {
        let wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().chain(Some(0)).collect();
        assert!(unsafe { DeleteAppContainerProfile(wide.as_ptr()) } >= 0,
            "remove only synthetic test profile {name}");
    }
    fs::remove_dir_all(base).expect("remove synthetic test objects");
}

#[link(name = "userenv")]
extern "system" { fn DeleteAppContainerProfile(name: *const u16) -> i32; }
