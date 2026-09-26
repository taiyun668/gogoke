use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::Manager;
use sha2::{Digest, Sha256};
#[cfg(target_os = "windows")]
use std::sync::Arc;

const SERVICE_TIMEOUT: Duration = Duration::from_secs(20);
// The draft path performs bounded Git preflight, CAS write and immutable readback
// after the controlled process; each remote request has its own 10s limit.
const DRAFT_SERVICE_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GoalRef {
    id: String,
    title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LedgerRef {
    repository: String,
    commit: String,
    path: String,
    content_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProductGoalRequest {
    goal: GoalRef,
    ledger: LedgerRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_controlled_task: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publish_test_draft: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fixture_driver_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ledger_merge_pull_number: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProductCallerView {
    admitted: bool,
    policy_revision: String,
    principal_id: String,
    profile_id: String,
    revocation_head: String,
    role: String,
    seat_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NativeHostView {
    reachable: bool,
    elapsed_micros: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LedgerReadbackView {
    state: String,
    git_blob: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LedgerMergeView {
    state: String,
    pull_number: u64,
    merge_commit: String,
    merged_by: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProductGoalView {
    goal: GoalRef,
    ledger: LedgerRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_controlled_task: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publish_test_draft: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fixture_driver_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ledger_merge_pull_number: Option<u64>,
    caller: ProductCallerView,
    native_host: NativeHostView,
    ledger_readback: LedgerReadbackView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ledger_merge: Option<LedgerMergeView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    controlled_task: Option<ControlledTaskView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    test_ledger_draft: Option<TestLedgerDraftView>,
    acceptance: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ControlledTaskView {
    state: String,
    source_commit: String,
    source_blob: String,
    report_sha256: String,
    model_id: String,
    relative_path: String,
    embedded_bytes_sha256: String,
    action_completion_ref: String,
    manifest_hash: String,
    decision_receipt_id: String,
    objective_outcome_content_hash: String,
    objective_outcome_receipt_id: String,
    evaluation_content_hash: String,
    evaluation_receipt_id: String,
    metrics_hash: String,
    dream_run_content_hash: String,
    dream_proposal_content_hash: String,
    dream_proposal_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fixture_driver_binding: Option<FixtureDriverBindingView>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FixtureDriverBindingView {
    driver_id: String,
    adapter_version: String,
    runtime_instance_id: String,
    launch_digest_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TestLedgerDraftView {
    state: String,
    repository: String,
    branch: String,
    commit: String,
    path: String,
    git_blob: String,
    content_hash: String,
}

#[derive(Clone)]
struct ProductRuntimePaths {
    node_runtime: PathBuf,
    service_entry: PathBuf,
    native_host: PathBuf,
    product_root: PathBuf,
    source_commit: Option<String>,
    #[cfg(target_os = "windows")]
    runtime_lease: Option<Arc<crate::resource_trust::RuntimeLease>>,
}

fn require_file(path: PathBuf, component: &'static str) -> Result<PathBuf, String> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("GOGOKE_PRODUCT_COMPONENT_MISSING:{component}"))
    }
}

#[cfg(target_os = "windows")]
fn node_compatible_windows_path(path: PathBuf) -> Result<PathBuf, String> {
    let text = path
        .to_str()
        .ok_or_else(|| "GOGOKE_PRODUCT_RUNTIME_PATH_UNSUPPORTED".to_string())?;
    let result = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(drive) = text.strip_prefix(r"\\?\") {
        let bytes = drive.as_bytes();
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || bytes[2] != b'\\'
        {
            return Err("GOGOKE_PRODUCT_RUNTIME_PATH_UNSUPPORTED".to_string());
        }
        PathBuf::from(drive)
    } else {
        path
    };
    if !result.is_absolute() {
        return Err("GOGOKE_PRODUCT_RUNTIME_PATH_UNSUPPORTED".to_string());
    }
    Ok(result)
}

#[cfg(target_os = "windows")]
fn resolve_runtime_paths(app: &tauri::AppHandle) -> Result<ProductRuntimePaths, String> {
    // Tauri may return a verbatim Windows path. Node's entry resolver can
    // treat that spelling as a drive-directory lookup and exit
    // before the service starts. Preserve the same trusted install location.
    let resource_dir = node_compatible_windows_path(
        app.path()
            .resource_dir()
            .map_err(|_| "GOGOKE_PRODUCT_RESOURCE_DIR_UNAVAILABLE".to_string())?,
    )?;
    let verified = app.try_state::<crate::resource_trust::ResourceState>()
        .map(|state| state.current())
        .transpose()?;
    if verified.is_none() && !cfg!(debug_assertions) {
        return Err("GOGOKE_PRODUCT_VERIFIED_RESOURCES_UNAVAILABLE".to_string());
    }
    if let Some(resources) = &verified { resources.verify_runtime_files()?; }
    let service_root = verified.as_ref()
        .map(|resources| resources.service_root.clone())
        .unwrap_or_else(|| resource_dir.join("gogoke-service"));
    let node_runtime = require_file(service_root.join("runtime").join("node.exe"), "node-runtime")?;
    let service_entry = require_file(
        verified.as_ref()
            .map(|resources| resources.generation_root.join("dist").join("bin.mjs"))
            .unwrap_or_else(|| service_root.join("dist").join("bin.mjs")),
        "service-entry",
    )?;

    // R2-04 owns final packaging. R2-01 admits only fixed product-controlled
    // install locations; no caller path, PATH lookup, or development-tree fallback.
    let executable_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let native_candidates = [
        resource_dir.join("gogoke-native-host.exe"),
        executable_dir
            .as_ref()
            .map(|dir| dir.join("gogoke-native-host.exe"))
            .unwrap_or_default(),
    ];
    let native_host = if let Some(resources) = &verified {
        resources.native_host_path.clone()
    } else {
        native_candidates
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| "GOGOKE_PRODUCT_COMPONENT_MISSING:native-host".to_string())?
    };
    let native_host = node_compatible_windows_path(native_host)?;

    let product_root = node_compatible_windows_path(
        app.path()
            .app_data_dir()
            .map_err(|_| "GOGOKE_PRODUCT_DATA_DIR_UNAVAILABLE".to_string())?
            .join("product-authority"),
    )?;
    Ok(ProductRuntimePaths {
        node_runtime,
        service_entry,
        native_host,
        product_root,
        source_commit: verified.as_ref().map(|resources| resources.source_commit.clone()),
        runtime_lease: verified.as_ref().map(|resources| resources.runtime_lease()),
    })
}

#[cfg(not(target_os = "windows"))]
fn resolve_runtime_paths(_app: &tauri::AppHandle) -> Result<ProductRuntimePaths, String> {
    Err("GOGOKE_PRODUCT_WINDOWS_OWNER_PATH_ONLY".to_string())
}

fn validate_product_response(response: &ProductGoalView) -> Result<(), String> {
    if response.acceptance != "TEST_FIXTURE_NOT_ADOPTED"
        || !response.caller.admitted
        || response.caller.role != "controller"
        || !response.native_host.reachable
        || response.ledger_readback.state != "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED"
        || response.ledger_readback.git_blob.len() != 40
        || !response.ledger_readback.git_blob.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("GOGOKE_PRODUCT_RESPONSE_NOT_ADMITTED".to_string());
    }
    if let Some(task) = &response.controlled_task {
        if response.run_controlled_task != Some(true)
            || task.state != "VALIDATED_TEST_RESULT_NOT_ADOPTED"
            || task.source_commit.len() != 40
            || task.source_blob.len() != 40
            || task.report_sha256.len() != 64
            || task.embedded_bytes_sha256.len() != 64
            || task.action_completion_ref.is_empty()
            || !task.manifest_hash.starts_with("sha256:")
            || task.manifest_hash.len() != 71
            || task.decision_receipt_id.is_empty()
            || !task.objective_outcome_content_hash.starts_with("sha256:")
            || task.objective_outcome_content_hash.len() != 71
            || task.objective_outcome_receipt_id.is_empty()
            || !task.evaluation_content_hash.starts_with("sha256:")
            || task.evaluation_content_hash.len() != 71
            || task.evaluation_receipt_id.is_empty()
            || !task.metrics_hash.starts_with("sha256:")
            || task.metrics_hash.len() != 71
            || !task.dream_run_content_hash.starts_with("sha256:")
            || task.dream_run_content_hash.len() != 71
            || !task.dream_proposal_content_hash.starts_with("sha256:")
            || task.dream_proposal_content_hash.len() != 71
            || task.dream_proposal_state != "DRAFT_TEST_ONLY_NOT_ACTIVATED"
        {
            return Err("GOGOKE_CONTROLLED_TASK_NOT_VALIDATED".to_string());
        }
        if let Some(binding) = &task.fixture_driver_binding {
            if response.fixture_driver_id.as_deref() != Some(binding.driver_id.as_str())
                || binding.adapter_version != "1.0.0"
                || !binding.runtime_instance_id.starts_with("runtime-r2-03-")
                || !binding.launch_digest_sha256.starts_with("sha256:")
                || binding.launch_digest_sha256.len() != 71
            {
                return Err("GOGOKE_NOVEL_FIXTURE_BINDING_NOT_VALIDATED".to_string());
            }
        }
    }
    if let Some(merge) = &response.ledger_merge {
        if merge.state != "PR_MERGE_ACCEPTED_FACT_VERIFIED"
            || merge.pull_number == 0
            || merge.merge_commit != response.ledger.commit
            || merge.merged_by != "taiyun668"
        {
            return Err("GOGOKE_LEDGER_MERGE_NOT_VERIFIED".to_string());
        }
    }
    if let Some(draft) = &response.test_ledger_draft {
        if response.publish_test_draft != Some(true)
            || draft.state != "DRAFT_COMMITTED_NOT_ADOPTED"
            || draft.repository != "taiyun668/gogoke"
            || draft.branch != "s1-r4-ledger-test/r2-02"
            || draft.commit.len() != 40
            || !draft.commit.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !draft.path.starts_with("apps/desktop/test-fixtures/s1-r4/ledger/r2-02-results/")
            || !draft.path.ends_with(".json")
            || draft.git_blob.len() != 40
            || !draft.git_blob.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !draft.content_hash.starts_with("sha256:")
            || draft.content_hash.len() != 71
            || !draft.content_hash[7..].bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("GOGOKE_TEST_LEDGER_DRAFT_NOT_VERIFIED".to_string());
        }
    }
    Ok(())
}

async fn run_product_service(
    paths: ProductRuntimePaths,
    request_bytes: &[u8],
    timeout: Duration,
    draft_identity: Option<(&str, &str)>,
    product_guard: Option<tokio::sync::MutexGuard<'static, ()>>,
) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    {
        // The blocking owner outlives this caller future. Dropping the join
        // receiver cannot release the verified handles or the Job while its
        // Node/native-host consumers are still running.
        let request = request_bytes.to_vec();
        let identity = draft_identity.map(|(sha, hash)| (sha.to_owned(), hash.to_owned()));
        let (reply, receiver) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            managed_service::run(paths, request, timeout, identity, product_guard, reply);
        });
        return receiver
            .await
            .map_err(|_| "GOGOKE_PRODUCT_SERVICE_OWNER_FAILED".to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (paths, request_bytes, timeout, draft_identity, product_guard);
        Err("GOGOKE_PRODUCT_WINDOWS_OWNER_PATH_ONLY".to_string())
    }
}

#[cfg(target_os = "windows")]
mod managed_service {
    use super::ProductRuntimePaths;
    use std::cmp::Ordering;
    use std::ffi::{c_void, OsStr};
    use std::fs::File;
    use std::io::{Read, Write};
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr::{null, null_mut};
    use std::time::{Duration, Instant};
    use tokio::sync::oneshot;
    use windows_sys::Win32::Foundation::{
        GetLastError, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Globalization::{
        CompareStringOrdinal, CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Environment::{
        FreeEnvironmentStringsW, GetEnvironmentStringsW,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject,
        SetInformationJobObject, TerminateJobObject, JobObjectBasicAccountingInformation,
        JobObjectExtendedLimitInformation, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
        InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
        UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
        CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    const CLEANUP_WAIT: Duration = Duration::from_secs(5);
    const RETRY_WAIT: Duration = Duration::from_secs(1);

    fn send_reply(
        reply: &mut Option<oneshot::Sender<Result<Vec<u8>, String>>>,
        value: Result<Vec<u8>, String>,
    ) {
        if let Some(channel) = reply.take() {
            let _ = channel.send(value);
        }
    }

    fn raw(handle: &OwnedHandle) -> HANDLE {
        handle.as_raw_handle() as HANDLE
    }

    fn owned(handle: HANDLE) -> OwnedHandle {
        // Called only after a successful Win32 handle-creating operation.
        unsafe { OwnedHandle::from_raw_handle(handle as _) }
    }

    fn pipe() -> Result<(OwnedHandle, OwnedHandle), String> {
        let mut read = null_mut();
        let mut write = null_mut();
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
            return Err("GOGOKE_PRODUCT_SERVICE_PIPE_FAILED".to_string());
        }
        Ok((owned(read), owned(write)))
    }

    fn parent_end_not_inherited(handle: &OwnedHandle) -> Result<(), String> {
        if unsafe { SetHandleInformation(raw(handle), HANDLE_FLAG_INHERIT, 0) } == 0 {
            Err("GOGOKE_PRODUCT_SERVICE_PIPE_FAILED".to_string())
        } else {
            Ok(())
        }
    }

    fn wide(value: &OsStr) -> Result<Vec<u16>, String> {
        let mut encoded: Vec<u16> = value.encode_wide().collect();
        if encoded.contains(&0) {
            return Err("GOGOKE_PRODUCT_RUNTIME_PATH_UNSUPPORTED".to_string());
        }
        encoded.push(0);
        Ok(encoded)
    }

    // Windows command-line quoting follows the CommandLineToArgvW backslash
    // rules used by Node's process argv parser.
    fn quote_arg(value: &OsStr) -> Vec<u16> {
        let source: Vec<u16> = value.encode_wide().collect();
        let mut out = vec![b'"' as u16];
        let mut slashes = 0;
        for unit in source {
            if unit == b'\\' as u16 {
                slashes += 1;
            } else {
                if unit == b'"' as u16 {
                    out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
                } else {
                    out.extend(std::iter::repeat_n(b'\\' as u16, slashes));
                }
                slashes = 0;
                out.push(unit);
            }
        }
        out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        out.push(b'"' as u16);
        out
    }

    fn command_line(paths: &ProductRuntimePaths) -> Vec<u16> {
        let args: [&OsStr; 6] = [
            paths.node_runtime.as_os_str(),
            paths.service_entry.as_os_str(),
            OsStr::new("--root"),
            paths.product_root.as_os_str(),
            OsStr::new("--native-host"),
            paths.native_host.as_os_str(),
        ];
        let mut result = Vec::new();
        for (index, arg) in args.into_iter().enumerate() {
            if index != 0 {
                result.push(b' ' as u16);
            }
            result.extend(quote_arg(arg));
        }
        result.push(0);
        result
    }

    struct ParentEnvironment(*const u16);

    impl Drop for ParentEnvironment {
        fn drop(&mut self) {
            unsafe { FreeEnvironmentStringsW(self.0) };
        }
    }

    fn environment_key(entry: &[u16]) -> Result<&[u16], String> {
        // Windows keeps drive-current-directory variables as =C:=C:\... .
        // Their name ends at the second equals sign, not the first.
        let start = usize::from(entry.first() == Some(&(b'=' as u16)));
        let end = entry[start..].iter().position(|unit| *unit == b'=' as u16)
            .map(|offset| start + offset)
            .ok_or("GOGOKE_PRODUCT_SERVICE_ENVIRONMENT_FAILED")?;
        if end == 0 || end >= i32::MAX as usize {
            return Err("GOGOKE_PRODUCT_SERVICE_ENVIRONMENT_FAILED".to_string());
        }
        Ok(&entry[..end])
    }

    fn key_order(left: &[u16], right: &[u16]) -> Ordering {
        match unsafe {
            CompareStringOrdinal(
                left.as_ptr(), left.len() as i32,
                right.as_ptr(), right.len() as i32, 1,
            )
        } {
            CSTR_LESS_THAN => Ordering::Less,
            CSTR_EQUAL => Ordering::Equal,
            CSTR_GREATER_THAN => Ordering::Greater,
            _ => left.cmp(right),
        }
    }

    fn environment(identity: Option<&(String, String)>) -> Result<Vec<u16>, String> {
        let parent = unsafe { GetEnvironmentStringsW() };
        if parent.is_null() {
            return Err("GOGOKE_PRODUCT_SERVICE_ENVIRONMENT_FAILED".to_string());
        }
        let parent = ParentEnvironment(parent);
        let mut entries: Vec<Vec<u16>> = Vec::new();
        let mut cursor = 0usize;
        loop {
            let start = cursor;
            while unsafe { *parent.0.add(cursor) } != 0 {
                cursor += 1;
                if cursor > 16 * 1024 * 1024 {
                    return Err("GOGOKE_PRODUCT_SERVICE_ENVIRONMENT_FAILED".to_string());
                }
            }
            if cursor == start {
                break;
            }
            let entry = unsafe { std::slice::from_raw_parts(parent.0.add(start), cursor - start) };
            let key = environment_key(entry)?;
            let node_option = key_order(key, &"NODE_OPTIONS".encode_utf16().collect::<Vec<_>>())
                == Ordering::Equal;
            let node_path = key_order(key, &"NODE_PATH".encode_utf16().collect::<Vec<_>>())
                == Ordering::Equal;
            let draft_key = ["GOGOKE_EXECUTION_EVIDENCE_SHA", "GOGOKE_SERVICE_ENTRY_SHA256"]
                .iter().any(|name| key_order(key, &name.encode_utf16().collect::<Vec<_>>())
                    == Ordering::Equal);
            if !node_option && !node_path && !(identity.is_some() && draft_key) {
                // Canonicalize case-insensitive duplicates before sorting.
                if let Some(prior) = entries.iter().position(|prior| {
                    environment_key(prior).is_ok_and(|prior_key|
                        key_order(prior_key, key) == Ordering::Equal)
                }) {
                    entries[prior] = entry.to_vec();
                } else {
                    entries.push(entry.to_vec());
                }
            }
            cursor += 1;
        }
        if let Some((sha, hash)) = identity {
            for (key, value) in [
                ("GOGOKE_EXECUTION_EVIDENCE_SHA", sha.as_str()),
                ("GOGOKE_SERVICE_ENTRY_SHA256", hash.as_str()),
            ] {
                let mut entry: Vec<u16> = key.encode_utf16().collect();
                entry.push(b'=' as u16);
                entry.extend(value.encode_utf16());
                entries.push(entry);
            }
        }
        entries.sort_by(|left, right| {
            key_order(
                environment_key(left).expect("validated environment key"),
                environment_key(right).expect("validated environment key"),
            )
        });
        let mut block = Vec::new();
        for entry in entries {
            block.extend(entry);
            block.push(0);
        }
        block.push(0);
        if block.len() == 1 {
            block.push(0);
        }
        Ok(block)
    }

    struct AttributeList {
        storage: Vec<usize>,
        initialized: bool,
    }

    impl AttributeList {
        fn new(handles: &[HANDLE; 3]) -> Result<Self, String> {
            let mut bytes = 0usize;
            unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes) };
            if bytes == 0 {
                return Err("GOGOKE_PRODUCT_SERVICE_HANDLE_LIST_FAILED".to_string());
            }
            let mut result = Self {
                storage: vec![0; bytes.div_ceil(size_of::<usize>())],
                initialized: false,
            };
            if unsafe { InitializeProcThreadAttributeList(result.ptr(), 1, 0, &mut bytes) } == 0 {
                return Err("GOGOKE_PRODUCT_SERVICE_HANDLE_LIST_FAILED".to_string());
            }
            result.initialized = true;
            if unsafe {
                UpdateProcThreadAttribute(
                    result.ptr(), 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(), size_of_val(handles), null_mut(), null(),
                )
            } == 0 {
                return Err("GOGOKE_PRODUCT_SERVICE_HANDLE_LIST_FAILED".to_string());
            }
            Ok(result)
        }

        fn ptr(&mut self) -> *mut c_void {
            self.storage.as_mut_ptr().cast()
        }
    }

    impl Drop for AttributeList {
        fn drop(&mut self) {
            if self.initialized {
                unsafe { DeleteProcThreadAttributeList(self.ptr()) };
            }
        }
    }

    struct ManagedProcess {
        process: OwnedHandle,
        job: OwnedHandle,
        process_id: u32,
        assigned: bool,
    }

    impl Drop for ManagedProcess {
        fn drop(&mut self) {
            // Keep the Job handle (and the caller's lease) alive even if a
            // worker panics before reaching the normal settlement path.
            while !settled(self).unwrap_or(false) {
                terminate(self);
                std::thread::sleep(RETRY_WAIT);
            }
        }
    }

    fn active(job: &OwnedHandle) -> Result<u32, String> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        if unsafe {
            QueryInformationJobObject(
                raw(job), JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32, null_mut(),
            )
        } == 0 {
            Err("GOGOKE_PRODUCT_SERVICE_JOB_QUERY_FAILED".to_string())
        } else {
            Ok(info.ActiveProcesses)
        }
    }

    fn process_exited(process: &OwnedHandle) -> Result<bool, String> {
        match unsafe { WaitForSingleObject(raw(process), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err("GOGOKE_PRODUCT_SERVICE_WAIT_FAILED".to_string()),
        }
    }

    fn settled(process: &ManagedProcess) -> Result<bool, String> {
        Ok(process_exited(&process.process)? && active(&process.job)? == 0)
    }

    fn wait_settled(process: &ManagedProcess, bound: Duration) -> Result<bool, String> {
        let until = Instant::now() + bound;
        loop {
            if settled(process)? {
                return Ok(true);
            }
            if Instant::now() >= until {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn terminate(process: &ManagedProcess) {
        if process.assigned {
            unsafe { TerminateJobObject(raw(&process.job), 1) };
        } else {
            unsafe { TerminateProcess(raw(&process.process), 1) };
        }
    }

    fn retain_until_exit(process: &ManagedProcess) {
        // An unconfirmed exit must not release the verified resource lease.
        // This detached blocking owner keeps both Job and process handles alive.
        loop {
            terminate(process);
            if settled(process).unwrap_or(false) {
                return;
            }
            std::thread::sleep(RETRY_WAIT);
        }
    }

    fn launch(
        paths: &ProductRuntimePaths,
        identity: Option<&(String, String)>,
        reply: &mut Option<oneshot::Sender<Result<Vec<u8>, String>>>,
    ) -> Result<(ManagedProcess, OwnedHandle, OwnedHandle, OwnedHandle), String> {
        let (stdin_read, stdin_write) = pipe()?;
        let (stdout_read, stdout_write) = pipe()?;
        let (stderr_read, stderr_write) = pipe()?;
        for handle in [&stdin_write, &stdout_read, &stderr_read] {
            parent_end_not_inherited(handle)?;
        }
        let inherited = [raw(&stdin_read), raw(&stdout_write), raw(&stderr_write)];
        let mut attributes = AttributeList::new(&inherited)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = inherited[0];
        startup.StartupInfo.hStdOutput = inherited[1];
        startup.StartupInfo.hStdError = inherited[2];
        startup.lpAttributeList = attributes.ptr();
        let executable = wide(paths.node_runtime.as_os_str())?;
        let mut command = command_line(paths);
        let service_root = paths.service_entry.parent().and_then(std::path::Path::parent)
            .ok_or("GOGOKE_PRODUCT_SERVICE_ROOT_UNAVAILABLE")?;
        let current_dir = wide(service_root.as_os_str())?;
        let environment = environment(identity)?;

        let job = unsafe { CreateJobObjectW(null(), null()) };
        if job.is_null() {
            return Err("GOGOKE_PRODUCT_SERVICE_JOB_FAILED".to_string());
        }
        let job = owned(job);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                raw(&job), JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0 {
            return Err("GOGOKE_PRODUCT_SERVICE_JOB_FAILED".to_string());
        }
        let mut info = PROCESS_INFORMATION::default();
        let created = unsafe {
            CreateProcessW(
                executable.as_ptr(), command.as_mut_ptr(), null(), null(), 1,
                CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT
                    | EXTENDED_STARTUPINFO_PRESENT,
                environment.as_ptr().cast(), current_dir.as_ptr(),
                &startup.StartupInfo, &mut info,
            )
        };
        if created == 0 {
            return Err(format!("GOGOKE_PRODUCT_SERVICE_START_FAILED:{}", unsafe { GetLastError() }));
        }
        let process = owned(info.hProcess);
        let thread = owned(info.hThread);
        let mut managed = ManagedProcess { process, job, process_id: info.dwProcessId, assigned: false };
        // No request is sent, and the primary thread cannot execute, until
        // the whole future process tree is under this no-breakaway Job.
        if unsafe { AssignProcessToJobObject(raw(&managed.job), raw(&managed.process)) } == 0 {
            terminate(&managed);
            if unsafe { WaitForSingleObject(raw(&managed.process), CLEANUP_WAIT.as_millis() as u32) }
                != WAIT_OBJECT_0
            {
                // Caller will receive an error while this owner retains the
                // suspended process and lease until termination is confirmed.
                send_reply(reply, Err("GOGOKE_PRODUCT_SERVICE_JOB_ASSIGN_EXIT_UNCONFIRMED".to_string()));
                retain_until_exit(&managed);
            }
            return Err("GOGOKE_PRODUCT_SERVICE_JOB_ASSIGN_FAILED".to_string());
        }
        managed.assigned = true;
        if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
            terminate(&managed);
            if !wait_settled(&managed, CLEANUP_WAIT).unwrap_or(false) {
                send_reply(reply, Err("GOGOKE_PRODUCT_SERVICE_RESUME_EXIT_UNCONFIRMED".to_string()));
                retain_until_exit(&managed);
            }
            return Err("GOGOKE_PRODUCT_SERVICE_RESUME_FAILED".to_string());
        }
        drop(thread);
        drop(stdin_read);
        drop(stdout_write);
        drop(stderr_write);
        Ok((managed, stdin_write, stdout_read, stderr_read))
    }

    pub(super) fn run(
        paths: ProductRuntimePaths,
        request: Vec<u8>,
        timeout: Duration,
        identity: Option<(String, String)>,
        _product_guard: Option<tokio::sync::MutexGuard<'static, ()>>,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    ) {
        let mut reply = Some(reply);
        // The paths own an Arc<RuntimeLease>; keep them in this detached
        // owner, including during unbounded post-error exit confirmation.
        if std::fs::create_dir_all(&paths.product_root).is_err() {
            send_reply(&mut reply, Err("GOGOKE_PRODUCT_ROOT_UNAVAILABLE".to_string()));
            return;
        }
        let (managed, stdin, stdout, stderr) = match launch(&paths, identity.as_ref(), &mut reply) {
            Ok(value) => value,
            Err(error) => {
                send_reply(&mut reply, Err(error));
                return;
            }
        };
        let stdout_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            File::from(stdout).read_to_end(&mut bytes).map(|_| bytes)
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            File::from(stderr).read_to_end(&mut bytes)
        });
        let writer = std::thread::spawn(move || File::from(stdin).write_all(&request));
        let failure = match unsafe {
            WaitForSingleObject(raw(&managed.process), timeout.as_millis() as u32)
        } {
            WAIT_OBJECT_0 => None,
            WAIT_TIMEOUT => {
                terminate(&managed);
                Some("GOGOKE_PRODUCT_SERVICE_TIMEOUT".to_string())
            }
            _ => {
                terminate(&managed);
                Some("GOGOKE_PRODUCT_SERVICE_WAIT_FAILED".to_string())
            }
        };
        if !wait_settled(&managed, CLEANUP_WAIT).unwrap_or(false) {
            terminate(&managed);
            if !wait_settled(&managed, CLEANUP_WAIT).unwrap_or(false) {
                send_reply(&mut reply, Err("GOGOKE_PRODUCT_SERVICE_EXIT_UNCONFIRMED".to_string()));
                retain_until_exit(&managed);
                return;
            }
        }
        let write_result = writer.join()
            .map_err(|_| "GOGOKE_PRODUCT_SERVICE_REQUEST_FAILED".to_string())
            .and_then(|value| value.map_err(|_| "GOGOKE_PRODUCT_SERVICE_REQUEST_FAILED".to_string()));
        let output = stdout_reader.join()
            .map_err(|_| "GOGOKE_PRODUCT_SERVICE_OUTPUT_FAILED".to_string())
            .and_then(|value| value.map_err(|_| "GOGOKE_PRODUCT_SERVICE_OUTPUT_FAILED".to_string()));
        let _ = stderr_reader.join();
        let result = if let Some(error) = failure {
            Err(error)
        } else if let Err(error) = write_result {
            Err(error)
        } else {
            let mut exit_code = 0;
            if unsafe { GetExitCodeProcess(raw(&managed.process), &mut exit_code) } == 0 {
                Err("GOGOKE_PRODUCT_SERVICE_WAIT_FAILED".to_string())
            } else if exit_code != 0 {
                Err(format!("GOGOKE_PRODUCT_SERVICE_FAILED:{exit_code}"))
            } else {
                output
            }
        };
        let _ = managed.process_id;
        let _ = &paths.runtime_lease;
        send_reply(&mut reply, result);
    }
}

async fn run_product_process(
    paths: ProductRuntimePaths,
    request: &ProductGoalRequest,
    product_guard: tokio::sync::MutexGuard<'static, ()>,
) -> Result<ProductGoalView, String> {
    let request_bytes =
        serde_json::to_vec(request).map_err(|_| "GOGOKE_PRODUCT_REQUEST_ENCODE_FAILED".to_string())?;
    let draft_identity = if request.publish_test_draft == Some(true) {
        if request.run_controlled_task != Some(true) {
            return Err("GOGOKE_TEST_DRAFT_REQUIRES_CONTROLLED_TASK".to_string());
        }
        let sha = paths.source_commit.as_deref()
            .ok_or_else(|| "GOGOKE_TEST_DRAFT_VERIFIED_SOURCE_UNAVAILABLE".to_string())?;
        if sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("GOGOKE_TEST_DRAFT_VERIFIED_SOURCE_INVALID".to_string());
        }
        let entry = tokio::fs::read(&paths.service_entry).await
            .map_err(|_| "GOGOKE_TEST_DRAFT_SERVICE_HASH_UNAVAILABLE".to_string())?;
        Some((sha.to_owned(), format!("sha256:{:x}", Sha256::digest(&entry))))
    } else {
        None
    };
    if let Some(driver_id) = &request.fixture_driver_id {
        if request.publish_test_draft != Some(true)
            || driver_id.len() != 27
            || !driver_id.starts_with("mock_novel_")
            || !driver_id[11..].bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("GOGOKE_NOVEL_FIXTURE_DRIVER_ID_INVALID".to_string());
        }
    }
    let timeout = if draft_identity.is_some() { DRAFT_SERVICE_TIMEOUT } else { SERVICE_TIMEOUT };
    let output = run_product_service(paths, &request_bytes, timeout,
        draft_identity.as_ref().map(|(sha, hash)| (sha.as_str(), hash.as_str())),
        Some(product_guard)).await?;
    let response: ProductGoalView = serde_json::from_slice(&output)
        .map_err(|_| "GOGOKE_PRODUCT_RESPONSE_DECODE_FAILED".to_string())?;
    validate_product_response(&response)?;
    if response.goal.id != request.goal.id
        || response.goal.title != request.goal.title
        || response.ledger.repository != request.ledger.repository
        || response.ledger.commit != request.ledger.commit
        || response.ledger.path != request.ledger.path
        || response.ledger.content_hash != request.ledger.content_hash
        || response.run_controlled_task != request.run_controlled_task
        || response.publish_test_draft != request.publish_test_draft
        || response.fixture_driver_id != request.fixture_driver_id
        || response.ledger_merge_pull_number != request.ledger_merge_pull_number
        || (request.ledger_merge_pull_number.is_some()) != response.ledger_merge.is_some()
        || response.ledger_merge.as_ref().is_some_and(|merge|
            Some(merge.pull_number) != request.ledger_merge_pull_number)
        || (request.run_controlled_task == Some(true)) != response.controlled_task.is_some()
        || (request.publish_test_draft == Some(true)) != response.test_ledger_draft.is_some()
    {
        return Err("GOGOKE_PRODUCT_RESPONSE_IDENTITY_MISMATCH".to_string());
    }
    Ok(response)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductReadinessCaller {
    admitted: bool,
    role: String,
    principal_id: String,
    seat_id: String,
    policy_revision: String,
    revocation_head: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductReadinessView {
    state: String,
    caller: ProductReadinessCaller,
}

pub(crate) async fn verify_product_startup(app: &tauri::AppHandle) -> Result<(), String> {
    let paths = resolve_runtime_paths(app)?;
    let output = run_product_service(paths, b"{\"operation\":\"readiness\"}", SERVICE_TIMEOUT, None, None).await?;
    let response: ProductReadinessView = serde_json::from_slice(&output)
        .map_err(|_| "GOGOKE_PRODUCT_READINESS_DECODE_FAILED".to_string())?;
    if response.state != "PRODUCT_SERVICE_NATIVE_CONTROLLER_ADMITTED"
        || !response.caller.admitted
        || response.caller.role != "controller"
        || response.caller.principal_id.is_empty()
        || response.caller.seat_id.is_empty()
        || response.caller.policy_revision.is_empty()
        || response.caller.revocation_head.is_empty()
    {
        return Err("GOGOKE_PRODUCT_READINESS_NOT_ADMITTED".to_string());
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn gogoke_r2_goal_probe(
    app: tauri::AppHandle,
    request: ProductGoalRequest,
) -> Result<ProductGoalView, String> {
    let guard = PRODUCT_RUNTIME_GATE.lock().await;
    let paths = resolve_runtime_paths(&app)?;
    run_product_process(paths, &request, guard).await
}

static PRODUCT_RUNTIME_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn acquire_product_gate() -> tokio::sync::MutexGuard<'static, ()> {
    PRODUCT_RUNTIME_GATE.lock().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_requires_native_controller_admission_and_non_adoption_marker() {
        let request = ProductGoalRequest {
            goal: GoalRef {
                id: "goal-r2-01".into(),
                title: "fixture".into(),
            },
            ledger: LedgerRef {
                repository: "fixture/gogoke-r2-01".into(),
                commit: "0123456789abcdef0123456789abcdef01234567".into(),
                path: "goals/r2-01.json".into(),
                content_hash: format!("sha256:{}", "a".repeat(64)),
            },
            run_controlled_task: None,
            publish_test_draft: None,
            fixture_driver_id: None,
            ledger_merge_pull_number: None,
        };
        let valid = ProductGoalView {
            goal: request.goal.clone(),
            ledger: request.ledger.clone(),
            run_controlled_task: None,
            publish_test_draft: None,
            fixture_driver_id: None,
            ledger_merge_pull_number: None,
            caller: ProductCallerView {
                admitted: true,
                policy_revision: "1".into(),
                principal_id: "owner:fixture".into(),
                profile_id: "profile:fixture".into(),
                revocation_head: "0".into(),
                role: "controller".into(),
                seat_id: "owner-seat:fixture".into(),
            },
            native_host: NativeHostView {
                reachable: true,
                elapsed_micros: 1,
            },
            ledger_readback: LedgerReadbackView {
                state: "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED".into(),
                git_blob: "a".repeat(40),
            },
            ledger_merge: None,
            controlled_task: None,
            test_ledger_draft: None,
            acceptance: "TEST_FIXTURE_NOT_ADOPTED".into(),
        };
        assert!(validate_product_response(&valid).is_ok());
        let mut invalid = valid.clone();
        invalid.caller.admitted = false;
        assert!(validate_product_response(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid.acceptance = "ADOPTED".into();
        assert!(validate_product_response(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid.ledger_readback.state = "ADOPTED".into();
        assert!(validate_product_response(&invalid).is_err());
    }
}
