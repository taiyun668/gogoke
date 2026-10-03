//! The fixed CLI's own Code Mode executable is a runtime dependency, not a
//! caller-selected tool. Grant only its exact file to the existing LPAC SID.
use crate::process::AppContainerProfile;
use crate::root::RootIdentity;
use crate::store::instance::ProgramObservation;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const VERSION: &str = "0.160.0";
const HELPER_NAME: &str = "codex-code-mode-host.exe";
// Official @openai/codex-win32-x64 0.160.0-win32-x64 npm archive member:
// package/vendor/x86_64-pc-windows-msvc/bin/codex-code-mode-host.exe.
const HELPER_DIGEST: &str =
    "sha256:1d448bfde19e7a280d600d8d0bcddf77afbe9feaec1e804905becc5f39bc9db6";
const FILE_SHARE_READ: u32 = 1;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

pub(crate) struct BoundCodexComponent {
    path: PathBuf,
    identity: RootIdentity,
    // Retain a read-only, no-write/no-delete-sharing handle through actual
    // process stop. A later same-name replacement cannot inherit this grant.
    _file: File,
}

impl BoundCodexComponent {
    pub(crate) fn prepare(program: &Path, profile: &AppContainerProfile)
        -> Result<Self, String> {
        if program.file_name().and_then(|value| value.to_str()) != Some("codex.exe") {
            return Err("native Codex component: unexpected catalog entrypoint".into());
        }
        let directory = program.parent()
            .ok_or("native Codex component: catalog directory absent")?;
        Self::prepare_at(&directory.join(HELPER_NAME), HELPER_DIGEST, profile)
    }

    fn prepare_at(path: &Path, expected_digest: &str, profile: &AppContainerProfile)
        -> Result<Self, String> {
        let file = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)
            .map_err(|error| format!("native Codex component open: {error:?}"))?;
        let identity = AppContainerProfile::capture_program_identity(path)
            .map_err(|error| format!("native Codex component identity: {error:?}"))?;
        let observed = ProgramObservation::observe(path, VERSION)
            .map_err(|error| format!("native Codex component digest: {error:?}"))?;
        if !observed.matches_pin(expected_digest, VERSION) {
            return Err("native Codex component: fixed package member digest changed".into());
        }
        profile.grant_bound_program(path, &identity)
            .map_err(|error| format!("native Codex component grant: {error:?}"))?;
        let component = Self { path: path.to_owned(), identity, _file: file };
        component.verify(profile)?;
        Ok(component)
    }

    pub(crate) fn verify(&self, profile: &AppContainerProfile) -> Result<(), String> {
        profile.verify_bound_program_grant(&self.path, &self.identity)
            .map_err(|error| format!("native Codex component verify: {error:?}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::windows::io::AsRawHandle;
    use std::process::{Command, Stdio};
    use crate::store::atomic::{Json, JsonString, Parser};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn member<'a>(value: &'a Json, name: &str) -> &'a Json {
        let Json::Object(fields) = value else { panic!("wire object expected"); };
        fields.get(&JsonString::from_str(name)).expect("required wire member")
    }

    fn text(value: &Json) -> String {
        let Json::String(value) = value else { panic!("wire string expected"); };
        value.to_well_formed_string().expect("well-formed wire string")
    }

    fn write_frame(writer: &mut impl Write, value: &str) {
        writer.write_all(&(value.len() as u32).to_le_bytes()).unwrap();
        writer.write_all(value.as_bytes()).unwrap();
        writer.flush().unwrap();
    }

    fn read_frame(reader: &mut impl Read) -> Json {
        let mut length = [0u8; 4];
        reader.read_exact(&mut length).expect("official helper frame prefix");
        let length = u32::from_le_bytes(length) as usize;
        assert!(length > 0 && length <= 64 * 1024, "bounded fixture response");
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).expect("official helper complete frame");
        Parser::parse(std::str::from_utf8(&bytes).expect("official helper UTF-8 frame"))
            .expect("official helper JSON frame")
    }

    fn response(reader: &mut impl Read, id: u32, kind: &str) -> Json {
        for _ in 0..32 {
            let value = read_frame(reader);
            if text(member(&value, "type")) != kind { continue; }
            if member(&value, "id").canonical() != id.to_string() { continue; }
            assert_eq!(text(member(member(&value, "result"), "status")), "ok",
                "official helper original response: {}", value.canonical());
            return value;
        }
        panic!("matching official helper response not observed");
    }

    // This is a cloud-only parent fixture. The child image is the actual,
    // digest-checked npm member, not a locally built Code Mode replacement.
    #[test]
    fn code_mode_lpac_parent_fixture() {
        let Some(path) = std::env::var_os("GOGOKE_TEST_CODE_MODE_IMAGE") else { return; };
        let positive = std::env::var("GOGOKE_TEST_CODE_MODE_CASE").unwrap() == "positive";
        let spawned = Command::new(&path).stdin(Stdio::piped()).stdout(Stdio::piped())
            .stderr(Stdio::inherit()).spawn();
        if !positive {
            let error = spawned.err().expect("ungranted official helper must not spawn");
            assert_eq!(error.raw_os_error(), Some(5), "original error: {error:?}");
            std::fs::write("negative-result.txt", "OFFICIAL_HELPER_WIN32_5").unwrap();
            return;
        }
        let mut child = spawned.expect("actual official helper LPAC spawn original Windows error");
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        write_frame(&mut input, r#"{"type":"connection/hello","supportedVersions":[1],"requiredCapabilities":[],"optionalCapabilities":["session-cell-execution-resource-limits","yield-observation"]}"#);
        let ready = read_frame(&mut output);
        assert_eq!(text(member(&ready, "type")), "connection/ready");
        assert_eq!(member(&ready, "selectedVersion").canonical(), "1");

        // Inspect the actual child while it is alive. Its token must be the
        // parent's LPAC SID/capabilities and its PID in the parent's exact Job.
        verify_child_identity(&child, Path::new(&path));
        write_frame(&mut input, r#"{"type":"operation/request","id":1,"request":{"method":"session/open","sessionId":"cloud-component"}}"#);
        let opened = response(&mut output, 1, "operation/response");
        assert_eq!(text(member(member(member(&opened, "result"), "value"), "type")), "session/ready");
        for (id, source) in [(2, r#"store(\"wire-check\",\"ok\");"#),
            (3, r#"text(String(load(\"wire-check\")));"#)] {
            write_frame(&mut input, &format!(r#"{{"type":"operation/request","id":{id},"request":{{"method":"session/execute","sessionId":"cloud-component","request":{{"tool_call_id":"call-{id}","enabled_tools":[],"source":"{source}","yield_time_ms":null,"max_output_tokens":null}}}}}}"#));
            let ack = response(&mut output, id, "operation/response");
            assert_eq!(text(member(member(member(&ack, "result"), "value"), "type")), "execution/started");
            let initial = response(&mut output, id, "execute/initialResponse");
            let result = member(member(member(&initial, "result"), "value"), "Result");
            assert_eq!(member(result, "error_text").canonical(), "null");
            if id == 3 {
                let Json::Array(items) = member(result, "content_items") else { panic!("content array"); };
                assert!(items.iter().any(|item| text(member(item, "type")) == "input_text"
                    && text(member(item, "text")) == "ok"), "actual cross-cell result");
            }
        }
        write_frame(&mut input, r#"{"type":"operation/request","id":4,"request":{"method":"session/shutdown","sessionId":"cloud-component"}}"#);
        let closed = response(&mut output, 4, "operation/response");
        assert_eq!(text(member(member(member(&closed, "result"), "value"), "type")), "session/closed");
        drop(input); // Frame-boundary EOF shuts down the actual host.
        assert!(child.wait().expect("official helper exit").success());
        std::fs::write("positive-result.txt", "OFFICIAL_HELPER_LPAC_JOB_EXECUTE_PASS").unwrap();
    }

    fn verify_child_identity(child: &std::process::Child, path: &Path) {
        use std::ffi::c_void;
        type Handle = *mut c_void;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> Handle;
            fn CloseHandle(handle: Handle) -> i32;
            fn QueryFullProcessImageNameW(handle: Handle, flags: u32, name: *mut u16, count: *mut u32) -> i32;
            fn QueryInformationJobObject(job: Handle, class: i32, buffer: *mut c_void, size: u32, returned: *mut u32) -> i32;
        }
        #[link(name = "advapi32")]
        extern "system" {
            fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
            fn GetTokenInformation(token: Handle, class: u32, data: *mut c_void, size: u32, returned: *mut u32) -> i32;
            fn EqualSid(first: *mut c_void, second: *mut c_void) -> i32;
        }
        let token_info = |process: Handle, class| {
            let mut token = std::ptr::null_mut();
            let opened = unsafe { OpenProcessToken(process, 8, &mut token) };
            let open_error = std::io::Error::last_os_error();
            assert_ne!(opened, 0, "actual child token open: {open_error:?}");
            let mut data = vec![0usize; 1024];
            let mut returned = 0;
            let result = unsafe { GetTokenInformation(token, class, data.as_mut_ptr().cast(),
                (data.len() * std::mem::size_of::<usize>()) as u32, &mut returned) };
            let original_error = std::io::Error::last_os_error();
            let closed = unsafe { CloseHandle(token) };
            let close_error = std::io::Error::last_os_error();
            assert_ne!(closed, 0, "actual token close: {close_error:?}; token query: {original_error:?}");
            assert_ne!(result, 0, "actual child token read: {original_error:?}");
            data
        };
        let child_handle = child.as_raw_handle();
        let parent_handle = unsafe { GetCurrentProcess() };
        let parent_sid = token_info(parent_handle, 31);
        let child_sid = token_info(child_handle, 31);
        assert!(parent_sid[0] != 0 && child_sid[0] != 0);
        assert_ne!(unsafe { EqualSid(parent_sid[0] as Handle, child_sid[0] as Handle) }, 0);
        let parent_caps = token_info(parent_handle, 30);
        let child_caps = token_info(child_handle, 30);
        #[repr(C)] struct SidAndAttributes { sid: Handle, attributes: u32 }
        let count = unsafe { *(parent_caps.as_ptr().cast::<u32>()) } as usize;
        assert_eq!(count, unsafe { *(child_caps.as_ptr().cast::<u32>()) } as usize);
        assert!(count > 0 && count <= 32);
        let offset = (4 + std::mem::align_of::<SidAndAttributes>() - 1)
            & !(std::mem::align_of::<SidAndAttributes>() - 1);
        let parent_groups = unsafe { std::slice::from_raw_parts(
            parent_caps.as_ptr().cast::<u8>().add(offset).cast::<SidAndAttributes>(), count) };
        let child_groups = unsafe { std::slice::from_raw_parts(
            child_caps.as_ptr().cast::<u8>().add(offset).cast::<SidAndAttributes>(), count) };
        for (parent, child) in parent_groups.iter().zip(child_groups) {
            assert_eq!(parent.attributes, child.attributes);
            assert_ne!(unsafe { EqualSid(parent.sid, child.sid) }, 0);
        }
        let mut image = vec![0u16; 32768];
        let mut count = image.len() as u32;
        let image_result = unsafe { QueryFullProcessImageNameW(child_handle, 0, image.as_mut_ptr(), &mut count) };
        let image_error = std::io::Error::last_os_error();
        assert_ne!(image_result, 0, "actual helper image: {image_error:?}");
        assert_eq!(String::from_utf16(&image[..count as usize]).unwrap().to_ascii_lowercase(),
            path.to_string_lossy().to_ascii_lowercase());
        #[repr(C)] struct JobMembers { assigned: u32, listed: u32, ids: [usize; 128] }
        let mut members = JobMembers { assigned: 0, listed: 0, ids: [0; 128] };
        let job_result = unsafe { QueryInformationJobObject(std::ptr::null_mut(), 3,
            (&mut members as *mut JobMembers).cast(), std::mem::size_of::<JobMembers>() as u32,
            std::ptr::null_mut()) };
        let job_error = std::io::Error::last_os_error();
        assert_ne!(job_result, 0, "query parent's exact Job: {job_error:?}");
        assert!(members.listed <= 128);
        let ids = &members.ids[..members.listed as usize];
        assert!(ids.contains(&(std::process::id() as usize)) && ids.contains(&(child.id() as usize)));
    }

    #[test]
    fn official_code_mode_member_runs_in_same_lpac_job_only_after_exact_grant() {
        use crate::process::{NativeBinding, PrepareRequest, ProcessCustodian, ProcessLaunch, StopBudgets};
        let _guard = crate::store::same_open::route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("gogoke-code-mode-real-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let profile_name = format!("Gogoke37.CodeMode.{nonce}");
        let profile = AppContainerProfile::ensure_for_cli(&profile_name, true).unwrap();
        profile.grant_fresh_session_directory(&folder).unwrap();
        let parent = folder.join("cloud-parent.exe");
        std::fs::write(&parent, std::fs::read(std::env::current_exe().unwrap()).unwrap()).unwrap();
        let program = crate::store::instance::locate_pinned_program("codex",
            "sha256:fdda5fa3cf3fb3d000b876720742857676293e4315e4b045fae6f8bd7e866d1d", VERSION)
            .expect("actual official cloud CLI catalog; mandatory, never skipped");
        let helper = program.parent().unwrap().join(HELPER_NAME);
        let digest = crate::store::digest::content_hash(&std::fs::read(&parent).unwrap());
        let run = |case: &str| {
            let mut launch = ProcessLaunch::new(&parent);
            launch.arguments = vec!["--exact".into(),
                "store::session_transport::codex_component::tests::code_mode_lpac_parent_fixture".into(),
                "--nocapture".into()];
            launch.current_directory = Some(folder.clone());
            launch.protocol_stdio = true;
            launch.app_container_profile = Some(profile_name.clone());
            launch.app_container_internet_client = true;
            launch.app_container_cli_identity_services = true;
            let runtime = folder.to_string_lossy().into_owned();
            launch.environment = Some(vec![("SystemRoot".into(), std::env::var("SystemRoot").unwrap()),
                ("HOME".into(), runtime.clone()), ("USERPROFILE".into(), runtime.clone()),
                ("APPDATA".into(), runtime.clone()), ("LOCALAPPDATA".into(), runtime.clone()),
                ("TEMP".into(), runtime.clone()), ("TMP".into(), runtime),
                ("GOGOKE_TEST_CODE_MODE_IMAGE".into(), helper.to_string_lossy().into_owned()),
                ("GOGOKE_TEST_CODE_MODE_CASE".into(), case.into())]);
            let mut custodian = ProcessCustodian::new().unwrap();
            let prepared = custodian.prepare(&PrepareRequest { launch, binding: NativeBinding {
                binary_digest_sha256: digest.clone(), profile_id: "component-fixture".into(),
                domain_id: "component-fixture".into(),
                generation: if case == "negative" { "1" } else { "2" }.into(),
            } }).unwrap();
            custodian.activate(&prepared).unwrap();
            let wait = custodian.active(&prepared.ticket).unwrap().wait(std::time::Duration::from_secs(60));
            assert!(matches!(wait, Ok(true)),
                "real helper fixture wait: case={case}, result={wait:?}, stderr={}",
                custodian.active(&prepared.ticket).unwrap().stderr_tail());
            let proof = custodian.stop(&prepared.ticket, StopBudgets::production(), || Ok(())).unwrap();
            let active = custodian.active(&prepared.ticket).unwrap();
            // This existing accessor drains only after actual Job accounting
            // proves zero writers; a failed stop keeps the live diagnostic.
            let stderr = active.stderr_tail();
            assert_eq!(proof.exit_code, Some(0), "case={case}, proof={proof:?}, stderr={stderr}");
            assert_eq!(proof.active_job_processes, Some(0), "case={case}, proof={proof:?}, stderr={stderr}");
            assert!(!proof.kill_attempted && proof.errors.is_empty(),
                "case={case}, normal whole-Job stop: {proof:?}, stderr={stderr}");
        };
        run("negative");
        assert_eq!(std::fs::read_to_string(folder.join("negative-result.txt")).unwrap(), "OFFICIAL_HELPER_WIN32_5");
        let component = BoundCodexComponent::prepare(&program, &profile).unwrap();
        component.verify(&profile).unwrap();
        run("positive");
        assert_eq!(std::fs::read_to_string(folder.join("positive-result.txt")).unwrap(), "OFFICIAL_HELPER_LPAC_JOB_EXECUTE_PASS");
        drop(component);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn fixed_codex_component_rejects_unpinned_bytes_before_grant_and_pins_exact_file() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("gogoke-code-mode-binding-{}-{nonce}",
            std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join(HELPER_NAME);
        let sibling = folder.join("other.exe");
        let bytes = b"controlled component bytes";
        std::fs::write(&path, bytes).unwrap();
        std::fs::write(&sibling, b"unrelated image").unwrap();
        let profile = AppContainerProfile::derived_for_test(&format!("Gogoke37.Component.{nonce}"))
            .unwrap();
        let identity = AppContainerProfile::capture_program_identity(&path).unwrap();
        assert!(BoundCodexComponent::prepare(&folder.join("codex.exe"), &profile).is_err(),
            "production entrypoint must reject fixture bytes against the official member pin");
        assert!(profile.verify_bound_program_grant(&path, &identity).is_err(),
            "digest rejection must not grant the file");
        let expected = crate::store::digest::content_hash(bytes);
        let held = BoundCodexComponent::prepare_at(&path, &expected, &profile).unwrap();
        held.verify(&profile).unwrap();
        let witness = profile.verify_bound_program_grant(&path, &identity).unwrap();
        assert_eq!(witness.rights, 0x0012_0089 | 0x0012_00a0);
        assert_eq!(witness.inheritance, 0);
        let sibling_identity = AppContainerProfile::capture_program_identity(&sibling).unwrap();
        assert!(profile.verify_bound_program_grant(&sibling, &sibling_identity).is_err());
        assert!(std::fs::write(&path, b"changed bytes").is_err(), "retained image cannot be written");
        assert!(std::fs::rename(&path, folder.join("replaced.exe")).is_err(),
            "retained image cannot be replaced by name");
        drop(held);
        std::fs::rename(&path, folder.join("old.exe")).unwrap();
        std::fs::write(&path, bytes).unwrap();
        assert!(profile.verify_bound_program_grant(&path, &identity).is_err(),
            "same bytes at a different physical file are not the old bound object");
        let alias = folder.join("alias.exe");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(BoundCodexComponent::prepare_at(&path, &expected, &profile).is_err(),
            "multi-link image must be rejected");
        std::fs::remove_file(&alias).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(BoundCodexComponent::prepare_at(&path, &expected, &profile).is_err(),
            "missing image must be refused, never replaced with a directory grant");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
