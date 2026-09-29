use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::Manager;
use sha2::{Digest, Sha256};
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
    resource_set_id: Option<String>,
    #[cfg(target_os = "windows")]
    runtime_lease: Option<Arc<crate::resource_trust::RuntimeLease>>,
}

#[derive(Clone)]
struct ExistingHostAttachment {
    service_pipe: String,
    service_capability: String,
}

struct Design37ProductOwner {
    host: Arc<super::design37_host::Design37Host>,
    paths: ProductRuntimePaths,
}

static DESIGN37_PRODUCT_OWNER: std::sync::Mutex<Option<Design37ProductOwner>> =
    std::sync::Mutex::new(None);

fn same_product_resource_generation(left: &ProductRuntimePaths, right: &ProductRuntimePaths) -> bool {
    let same = left.node_runtime == right.node_runtime
        && left.service_entry == right.service_entry
        && left.native_host == right.native_host
        && left.product_root == right.product_root
        && left.source_commit == right.source_commit
        && left.resource_set_id == right.resource_set_id;
    #[cfg(target_os = "windows")]
    let same = same && left.runtime_lease.is_some() == right.runtime_lease.is_some();
    same
}

fn retained_design37_host(
    paths: &ProductRuntimePaths,
) -> Result<Option<(Arc<super::design37_host::Design37Host>, ExistingHostAttachment)>, String> {
    let owner = DESIGN37_PRODUCT_OWNER.lock()
        .map_err(|_| "GOGOKE_DESIGN37_OWNER_LOCK_POISONED".to_string())?;
    let Some(owner) = owner.as_ref() else { return Ok(None) };
    if !same_product_resource_generation(&owner.paths, paths) {
        return Err("GOGOKE_DESIGN37_RESOURCE_GENERATION_CHANGED".to_string());
    }
    if !owner.host.is_running()? {
        return Err("GOGOKE_DESIGN37_HOST_EXITED".to_string());
    }
    Ok(Some((Arc::clone(&owner.host), ExistingHostAttachment {
        service_pipe: owner.host.service_pipe().to_string(),
        service_capability: owner.host.capability().to_string(),
    })))
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
        resource_set_id: verified.as_ref().map(|resources| resources.set_id.clone()),
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
        // Every managed service call uses the same physical Product root.
        // Keep this guard in the detached owner through process-tree exit;
        // a cancelled caller must not admit another native-host early.
        let service_guard = PRODUCT_SERVICE_GATE.lock().await;
        let existing = retained_design37_host(&paths)?;
        return run_product_service_with_guard(paths, request_bytes, timeout,
            draft_identity, product_guard, service_guard, existing).await;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (paths, request_bytes, timeout, draft_identity, product_guard);
        Err("GOGOKE_PRODUCT_WINDOWS_OWNER_PATH_ONLY".to_string())
    }
}

#[cfg(target_os = "windows")]
async fn run_product_service_with_guard(
    paths: ProductRuntimePaths,
    request_bytes: &[u8],
    timeout: Duration,
    draft_identity: Option<(&str, &str)>,
    product_guard: Option<tokio::sync::MutexGuard<'static, ()>>,
    service_guard: tokio::sync::MutexGuard<'static, ()>,
    existing: Option<(Arc<super::design37_host::Design37Host>, ExistingHostAttachment)>,
) -> Result<Vec<u8>, String> {
    // The blocking owner outlives this caller future. Dropping the join
    // receiver cannot release verified handles, the host or the Node Job.
    let request = request_bytes.to_vec();
    let identity = draft_identity.map(|(sha, hash)| (sha.to_owned(), hash.to_owned()));
    let (reply, receiver) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || {
        if let Some((host, attachment)) = existing {
            let _retained_host = host;
            managed_service::run_existing(paths, request, timeout, identity,
                product_guard, service_guard, attachment, reply);
        } else {
            managed_service::run(paths, request, timeout, identity,
                product_guard, service_guard, reply);
        }
    });
    receiver.await.map_err(|_| "GOGOKE_PRODUCT_SERVICE_OWNER_FAILED".to_string())?
}

#[cfg(target_os = "windows")]
mod managed_service {
    use super::{node_compatible_windows_path, ExistingHostAttachment, ProductRuntimePaths};
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use std::cmp::Ordering;
    use std::ffi::{c_void, OsStr};
    use std::fs::File;
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
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
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
        STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    const CLEANUP_WAIT: Duration = Duration::from_secs(5);
    const RETRY_WAIT: Duration = Duration::from_secs(1);
    const FAILURE_OUTPUT_TAIL_BYTES: usize = 4096;

    enum LaunchMode {
        Legacy,
        Existing(ExistingHostAttachment),
    }

    fn service_input(mode: &LaunchMode, request: Vec<u8>) -> Result<Vec<u8>, String> {
        match mode {
            LaunchMode::Legacy => Ok(request),
            LaunchMode::Existing(attachment) => {
                if request.is_empty() || request.len() > 4 * 1024 * 1024 {
                    return Err("GOGOKE_DESIGN37_SERVICE_REQUEST_SIZE_INVALID".to_string());
                }
                serde_json::to_vec(&serde_json::json!({
                    "servicePipe": attachment.service_pipe.as_str(),
                    "serviceCapability": attachment.service_capability.as_str(),
                    "requestBytesBase64": base64::engine::general_purpose::STANDARD.encode(request),
                })).map_err(|_| "GOGOKE_DESIGN37_SERVICE_ENVELOPE_FAILED".to_string())
            }
        }
    }

    fn read_tail(mut reader: impl Read) -> std::io::Result<Vec<u8>> {
        let mut tail = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let count = reader.read(&mut chunk)?;
            if count == 0 { break; }
            tail.extend_from_slice(&chunk[..count]);
            if tail.len() > FAILURE_OUTPUT_TAIL_BYTES {
                tail.drain(..tail.len() - FAILURE_OUTPUT_TAIL_BYTES);
            }
        }
        Ok(tail)
    }

    fn with_failure_output(mut error: String, stream: &str, bytes: &[u8]) -> String {
        if !bytes.is_empty() {
            error.push_str(":");
            error.push_str(stream);
            error.push_str("_TAIL:");
            error.push_str(&String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(FAILURE_OUTPUT_TAIL_BYTES)..]));
        }
        error
    }

    struct ModulePolicy {
        path: std::path::PathBuf,
        import_specifier: String,
        lease: Option<crate::resource_trust::RuntimeLease>,
    }

    impl ModulePolicy {
        fn new(paths: &ProductRuntimePaths) -> Result<Option<Self>, String> {
            let Some(resource_lease) = &paths.runtime_lease else {
                return Ok(None); // Local debug path has no verified resource state.
            };
            let module_paths = resource_lease.module_file_paths();
            if module_paths.is_empty() || !module_paths.contains(&paths.service_entry) {
                return Err("GOGOKE_MODULE_ENTRY_NOT_LEASED".to_string());
            }
            let normalized = module_paths.iter()
                .map(|path| node_compatible_windows_path(path.clone())
                    .and_then(|path| path.to_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "GOGOKE_MODULE_POLICY_PATH_UNSUPPORTED".to_string())))
                .collect::<Result<Vec<_>, _>>()?;
            let bytes = serde_json::to_vec(&normalized)
                .map_err(|_| "GOGOKE_MODULE_POLICY_ENCODE_FAILED".to_string())?;
            let hash = format!("{:x}", Sha256::digest(&bytes));
            // The policy lease denies DELETE sharing on its parent directory.
            // Keep it beside the native root so RootLock can pin that root.
            let path = paths.product_root.with_file_name(format!(
                ".gogoke-module-policy-{}.json", uuid::Uuid::new_v4().simple()
            ));
            let mut output = OpenOptions::new().write(true).create_new(true)
                .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
                .open(&path)
                .map_err(|_| "GOGOKE_MODULE_POLICY_CREATE_FAILED".to_string())?;
            if output.write_all(&bytes).is_err() || output.sync_all().is_err() {
                drop(output);
                let _ = std::fs::remove_file(&path);
                return Err("GOGOKE_MODULE_POLICY_WRITE_FAILED".to_string());
            }
            drop(output);
            let mut lease = crate::resource_trust::RuntimeLease::default();
            if let Err(error) = lease.pin_generated_file(&path, &bytes) {
                drop(lease);
                let _ = std::fs::remove_file(&path);
                return Err(error);
            }
            let mut policy = Self { path, import_specifier: String::new(), lease: Some(lease) };
            let policy_path = node_compatible_windows_path(policy.path.clone())?;
            let policy_path = policy_path.to_str()
                .ok_or_else(|| "GOGOKE_MODULE_POLICY_PATH_UNSUPPORTED".to_string())?;
            let path_literal = serde_json::to_string(policy_path)
                .map_err(|_| "GOGOKE_MODULE_POLICY_ENCODE_FAILED".to_string())?;
            let hash_literal = serde_json::to_string(&hash)
                .map_err(|_| "GOGOKE_MODULE_POLICY_ENCODE_FAILED".to_string())?;
            let bootstrap = format!("{}\ninstallGuard({path_literal}, {hash_literal});\n",
                include_str!("module_guard.mjs"));
            let encoded = base64::engine::general_purpose::STANDARD.encode(bootstrap);
            policy.import_specifier = format!("--import=data:text/javascript;base64,{encoded}");
            Ok(Some(policy))
        }
    }

    impl Drop for ModulePolicy {
        fn drop(&mut self) {
            self.lease.take();
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[cfg(test)]
    static TEST_JOB_HANDLE: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    #[cfg(test)]
    pub(super) fn test_job_handle() -> HANDLE {
        TEST_JOB_HANDLE.load(std::sync::atomic::Ordering::SeqCst) as HANDLE
    }

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

    fn command_line(paths: &ProductRuntimePaths, policy: Option<&ModulePolicy>,
        mode: &LaunchMode) -> Vec<u16> {
        let mut args: Vec<&OsStr> = vec![paths.node_runtime.as_os_str()];
        if let Some(policy) = policy {
            args.push(OsStr::new(policy.import_specifier.as_str()));
        }
        args.extend([
            paths.service_entry.as_os_str(), OsStr::new("--root"),
            paths.product_root.as_os_str(), OsStr::new("--native-host"),
            paths.native_host.as_os_str(),
        ]);
        if matches!(mode, LaunchMode::Existing(_)) {
            args.push(OsStr::new("--existing-design37-host"));
        }
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
        // Windows drive-current-directory entries begin with '='; their name
        // ends at the second equals sign.
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
        fn new(handles: &[HANDLE; 3], job: &HANDLE) -> Result<Self, String> {
            let mut bytes = 0usize;
            unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes) };
            if bytes == 0 {
                return Err("GOGOKE_PRODUCT_SERVICE_HANDLE_LIST_FAILED".to_string());
            }
            let mut result = Self {
                storage: vec![0; bytes.div_ceil(size_of::<usize>())],
                initialized: false,
            };
            if unsafe { InitializeProcThreadAttributeList(result.ptr(), 2, 0, &mut bytes) } == 0 {
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
            if unsafe {
                UpdateProcThreadAttribute(
                    result.ptr(), 0, PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                    (job as *const HANDLE).cast(), size_of::<HANDLE>(), null_mut(), null(),
                )
            } == 0 {
                return Err("GOGOKE_PRODUCT_SERVICE_JOB_FAILED".to_string());
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
        policy: Option<&ModulePolicy>,
        mode: &LaunchMode,
        identity: Option<&(String, String)>,
        reply: &mut Option<oneshot::Sender<Result<Vec<u8>, String>>>,
    ) -> Result<(ManagedProcess, OwnedHandle, OwnedHandle, OwnedHandle), String> {
        let (stdin_read, stdin_write) = pipe()?;
        let (stdout_read, stdout_write) = pipe()?;
        let (stderr_read, stderr_write) = pipe()?;
        for handle in [&stdin_write, &stdout_read, &stderr_read] {
            parent_end_not_inherited(handle)?;
        }
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
        let inherited = [raw(&stdin_read), raw(&stdout_write), raw(&stderr_write)];
        let job_handle = raw(&job);
        let mut attributes = AttributeList::new(&inherited, &job_handle)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = inherited[0];
        startup.StartupInfo.hStdOutput = inherited[1];
        startup.StartupInfo.hStdError = inherited[2];
        startup.lpAttributeList = attributes.ptr();
        let executable = wide(paths.node_runtime.as_os_str())?;
        let mut command = command_line(paths, policy, mode);
        let service_root = paths.service_entry.parent().and_then(std::path::Path::parent)
            .ok_or("GOGOKE_PRODUCT_SERVICE_ROOT_UNAVAILABLE")?;
        let current_dir = wide(service_root.as_os_str())?;
        let environment = environment(identity)?;

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
        #[cfg(test)]
        if std::env::var_os("GOGOKE_TEST_OUTER_JOB_CREATE_PAUSE").is_some() {
            println!("GOGOKE_OUTER_JOB_CREATED:{}", managed.process_id);
            std::io::stdout().flush().expect("flush exact Node PID to test parent");
            loop { std::thread::park(); }
        }
        // JOB_LIST already contains the suspended child when CreateProcess returns.
        // Retain the existing explicit same-Job check before its thread can run.
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

    #[cfg(test)]
    pub(super) fn test_launch_paused(node: std::path::PathBuf, root: std::path::PathBuf) {
        let paths = ProductRuntimePaths {
            node_runtime: node,
            service_entry: root.join("service/dist/bin.mjs"),
            native_host: root.join("unused-native-host.exe"),
            product_root: root.join("product"),
            source_commit: None,
            resource_set_id: None,
            runtime_lease: None,
        };
        let mut reply = None;
        let _ = launch(&paths, None, &LaunchMode::Legacy, None, &mut reply)
            .expect("test Node launch reaches post-CreateProcess pause");
        panic!("post-CreateProcess pause returned unexpectedly");
    }

    pub(super) fn run(
        paths: ProductRuntimePaths,
        request: Vec<u8>,
        timeout: Duration,
        identity: Option<(String, String)>,
        _product_guard: Option<tokio::sync::MutexGuard<'static, ()>>,
        _service_guard: tokio::sync::MutexGuard<'static, ()>,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    ) {
        run_with_mode(paths, request, timeout, identity, _product_guard,
            _service_guard, LaunchMode::Legacy, reply);
    }

    pub(super) fn run_existing(
        paths: ProductRuntimePaths,
        request: Vec<u8>,
        timeout: Duration,
        identity: Option<(String, String)>,
        product_guard: Option<tokio::sync::MutexGuard<'static, ()>>,
        service_guard: tokio::sync::MutexGuard<'static, ()>,
        attachment: ExistingHostAttachment,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    ) {
        run_with_mode(paths, request, timeout, identity, product_guard,
            service_guard, LaunchMode::Existing(attachment), reply);
    }

    fn run_with_mode(
        paths: ProductRuntimePaths,
        request: Vec<u8>,
        timeout: Duration,
        identity: Option<(String, String)>,
        _product_guard: Option<tokio::sync::MutexGuard<'static, ()>>,
        _service_guard: tokio::sync::MutexGuard<'static, ()>,
        mode: LaunchMode,
        reply: oneshot::Sender<Result<Vec<u8>, String>>,
    ) {
        let mut reply = Some(reply);
        // The paths own an Arc<RuntimeLease>; keep them in this detached
        // owner, including during unbounded post-error exit confirmation.
        if std::fs::create_dir_all(&paths.product_root).is_err() {
            send_reply(&mut reply, Err("GOGOKE_PRODUCT_ROOT_UNAVAILABLE".to_string()));
            return;
        }
        let policy = match ModulePolicy::new(&paths) {
            Ok(policy) => policy,
            Err(error) => {
                send_reply(&mut reply, Err(error));
                return;
            }
        };
        let request = match service_input(&mode, request) {
            Ok(bytes) => bytes,
            Err(error) => {
                send_reply(&mut reply, Err(error));
                return;
            }
        };
        let (managed, stdin, stdout, stderr) = match launch(&paths, policy.as_ref(), &mode, identity.as_ref(), &mut reply) {
            Ok(value) => value,
            Err(error) => {
                send_reply(&mut reply, Err(error));
                return;
            }
        };
        #[cfg(test)]
        TEST_JOB_HANDLE.store(raw(&managed.job) as usize, std::sync::atomic::Ordering::SeqCst);
        let stdout_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            File::from(stdout).read_to_end(&mut bytes).map(|_| bytes)
        });
        let stderr_reader = std::thread::spawn(move || read_tail(File::from(stderr)));
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
        let stderr_tail = stderr_reader.join()
            .map_err(|_| "GOGOKE_PRODUCT_SERVICE_STDERR_READ_FAILED".to_string())
            .and_then(|value| value.map_err(|error| format!("GOGOKE_PRODUCT_SERVICE_STDERR_READ_FAILED:WIN32_{}", error.raw_os_error().unwrap_or(0))));
        let mut exit_code = 0;
        let exit_code_available = unsafe { GetExitCodeProcess(raw(&managed.process), &mut exit_code) } != 0;
        let result = if let Some(error) = failure {
            Err(error)
        } else if !exit_code_available {
            Err("GOGOKE_PRODUCT_SERVICE_WAIT_FAILED".to_string())
        } else if exit_code != 0 {
            let mut error = format!("GOGOKE_PRODUCT_SERVICE_FAILED:{exit_code}");
            if let Ok(bytes) = &output {
                error = with_failure_output(error, "STDOUT", bytes);
            }
            match &stderr_tail {
                Ok(bytes) => error = with_failure_output(error, "STDERR", bytes),
                Err(read_error) => error.push_str(&format!(":{read_error}")),
            }
            Err(error)
        } else if let Err(error) = write_result {
            Err(error)
        } else {
            output
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

fn validate_product_readiness(output: &[u8]) -> Result<(), String> {
    let response: ProductReadinessView = serde_json::from_slice(output)
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

pub(crate) async fn verify_product_startup(app: &tauri::AppHandle) -> Result<(), String> {
    let paths = resolve_runtime_paths(app)?;
    let output = run_product_service(paths, b"{\"operation\":\"readiness\"}", SERVICE_TIMEOUT, None, None).await?;
    validate_product_readiness(&output)
}

/// Start the long-lived host and retain the exact resource generation used for
/// its executable. The service connector receives only this host's endpoint.
fn spawn_design37_host(
    app: &tauri::AppHandle,
) -> Result<(super::design37_host::Design37Host, ProductRuntimePaths), String> {
    let paths = resolve_runtime_paths(app)?;
    let host = super::design37_host::Design37Host::spawn(&paths.native_host, &paths.product_root)?;
    Ok((host, paths))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Design37RegisterCodexRequest {
    request_id: String,
    instance_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Design37RegisterCodexReceipt {
    schema: String,
    family: String,
    operation: String,
    request_id: String,
    target_id: String,
    status: String,
    previous_revision: String,
    revision: String,
    result: serde_json::Map<String, serde_json::Value>,
}

fn canonical_v37_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphabetic()
        && bytes[1..].iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
}

/// The installed product's Owner plane forwards exact request bytes over its
/// retained process-object-verified User pipe. It never accepts a host path,
/// endpoint, process identity, capacity proof or service capability from JS.
#[tauri::command]
pub(crate) async fn gogoke_design37_user_operation(
    app: tauri::AppHandle,
    frame: String,
) -> Result<String, String> {
    if frame.is_empty() || frame.len() > 4 * 1024 * 1024 {
        return Err("GOGOKE_DESIGN37_USER_FRAME_SIZE_INVALID".to_string());
    }
    let product_guard = PRODUCT_RUNTIME_GATE.lock().await;
    let paths = resolve_runtime_paths(&app)?;
    let (host, _) = retained_design37_host(&paths)?
        .ok_or_else(|| "GOGOKE_DESIGN37_USER_HOST_NOT_STARTED".to_string())?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let _product_guard = product_guard;
        // Cancellation drops only the response receiver. The blocking owner
        // retains the guard until the native operation has settled.
        if sender.send(host.request_user(frame.as_bytes())).is_err() {
            eprintln!("GOGOKE_DESIGN37_USER_RESPONSE_RECEIVER_CLOSED");
        }
    });
    let response = receiver.await
        .map_err(|error| format!("GOGOKE_DESIGN37_USER_OWNER_FAILED:{error}"))??;
    let response = String::from_utf8(response)
        .map_err(|error| format!("GOGOKE_DESIGN37_USER_RESPONSE_UTF8_FAILED:{error}"))?;
    if response.starts_with("ERR\t") {
        return Err(format!("GOGOKE_DESIGN37_NATIVE_USER_OPERATION_FAILED:{response}"));
    }
    Ok(response)
}

fn validate_design37_register_receipt(
    bytes: &[u8], request: &Design37RegisterCodexRequest,
) -> Result<Design37RegisterCodexReceipt, String> {
    let receipt: Design37RegisterCodexReceipt = serde_json::from_slice(bytes)
        .map_err(|error| format!("GOGOKE_DESIGN37_USER_RECEIPT_DECODE_FAILED:{error}"))?;
    if receipt.schema != "gogoke.37.operations.v1" || receipt.family != "K-INSTANCE"
        || receipt.operation != "register" || receipt.request_id != request.request_id
        || receipt.target_id != request.instance_id
        || !["APPLIED", "REPLAYED", "DENIED", "STALE", "CONFLICT", "UNSUPPORTED", "UNKNOWN", "FAILED"]
            .contains(&receipt.status.as_str())
        || receipt.previous_revision.parse::<u64>().ok()
            .is_none_or(|value| value.to_string() != receipt.previous_revision)
        || receipt.revision.parse::<u64>().ok()
            .is_none_or(|value| value.to_string() != receipt.revision)
    {
        return Err("GOGOKE_DESIGN37_USER_RECEIPT_MISMATCH".to_string());
    }
    Ok(receipt)
}

/// Explicit product User action. The frontend supplies only two canonical IDs;
/// Tauri chooses the native host, User pipe, domain, operation and driver.
#[tauri::command]
pub(crate) async fn gogoke_design37_register_codex_instance(
    app: tauri::AppHandle,
    request: Design37RegisterCodexRequest,
) -> Result<Design37RegisterCodexReceipt, String> {
    if !canonical_v37_id(&request.request_id) || !canonical_v37_id(&request.instance_id) {
        return Err("GOGOKE_DESIGN37_REGISTER_IDS_INVALID".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        let product_guard = PRODUCT_RUNTIME_GATE.lock().await;
        let service_guard = PRODUCT_SERVICE_GATE.lock().await;
        let current_paths = resolve_runtime_paths(&app)?;
        let (host, attachment, paths, service_guard) = match retained_design37_host(&current_paths)? {
            Some((host, attachment)) => (host, attachment, current_paths, service_guard),
            None => {
                let app_for_spawn = app.clone();
                // Keep the service gate in the blocking owner if this command
                // is cancelled while the exact native host starts.
                let (spawned, service_guard) = tokio::task::spawn_blocking(move || {
                    (spawn_design37_host(&app_for_spawn), service_guard)
                }).await.map_err(|_| "GOGOKE_DESIGN37_HOST_OWNER_FAILED".to_string())?;
                let (spawned, pinned_paths) = spawned?;
                if !same_product_resource_generation(&current_paths, &pinned_paths) {
                    return Err("GOGOKE_DESIGN37_RESOURCE_GENERATION_CHANGED".to_string());
                }
                let host = Arc::new(spawned);
                let attachment = ExistingHostAttachment {
                    service_pipe: host.service_pipe().to_string(),
                    service_capability: host.capability().to_string(),
                };
                let mut owner = DESIGN37_PRODUCT_OWNER.lock()
                    .map_err(|_| "GOGOKE_DESIGN37_OWNER_LOCK_POISONED".to_string())?;
                if owner.is_some() {
                    return Err("GOGOKE_DESIGN37_OWNER_ALREADY_STARTED".to_string());
                }
                *owner = Some(Design37ProductOwner {
                    host: Arc::clone(&host), paths: pinned_paths.clone(),
                });
                drop(owner);
                (host, attachment, pinned_paths, service_guard)
            }
        };
        let output = run_product_service_with_guard(paths,
            b"{\"operation\":\"readiness\"}", SERVICE_TIMEOUT, None, None,
            service_guard, Some((Arc::clone(&host), attachment))).await?;
        validate_product_readiness(&output)?;
        let frame = serde_json::to_vec(&serde_json::json!({
            "schema": "gogoke.37.operations.v1",
            "family": "K-INSTANCE", "operation": "register",
            "requestId": request.request_id.as_str(), "targetId": request.instance_id.as_str(),
            "domainId": "global", "expectedRevision": "0",
            "payload": { "driverId": "codex" },
        })).map_err(|_| "GOGOKE_DESIGN37_USER_REQUEST_ENCODE_FAILED".to_string())?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let _product_guard = product_guard;
            let _ = sender.send(host.request_user(&frame));
        });
        let response = receiver.await
            .map_err(|_| "GOGOKE_DESIGN37_USER_OWNER_FAILED".to_string())??;
        return validate_design37_register_receipt(&response, &request);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Err("GOGOKE_PRODUCT_WINDOWS_OWNER_PATH_ONLY".to_string())
    }
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
#[cfg(target_os = "windows")]
static PRODUCT_SERVICE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn acquire_product_gate() -> tokio::sync::MutexGuard<'static, ()> {
    PRODUCT_RUNTIME_GATE.lock().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn design37_product_composition_reuses_one_tauri_owned_host() {
        // The cloud job stages the same Node, service bundle and native binary
        // that the product resource verifier admits. Exercise the actual
        // Tauri-side host owner and managed Node entry, not a parser stub.
        let node = PathBuf::from(std::env::var_os("GOGOKE_CONTROLLED_NODE_PATH")
            .expect("cloud test requires the staged signed Node runtime"));
        assert!(node.is_file(), "staged signed Node runtime");
        let resources = node.parent().and_then(Path::parent)
            .expect("staged resource directory");
        let service_entry = resources.join("dist/bin.mjs");
        let native_host = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries/gogoke-native-host-x86_64-pc-windows-msvc.exe");
        assert!(service_entry.is_file(), "staged product service bundle");
        assert!(native_host.is_file(), "staged native host");
        let product_root = std::env::temp_dir().join(format!(
            "gogoke-design37-composition-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&product_root).expect("owned product root");
        let host = super::super::design37_host::Design37Host::spawn(&native_host, &product_root)
            .expect("Tauri-side owner starts native host and binds User pipe");
        let host_pid = host.host_pid();
        let attachment = ExistingHostAttachment {
            service_pipe: host.service_pipe().to_string(),
            service_capability: host.capability().to_string(),
        };
        let paths = ProductRuntimePaths {
            node_runtime: node,
            service_entry,
            native_host,
            product_root: product_root.clone(),
            source_commit: None,
            resource_set_id: None,
            runtime_lease: None,
        };
        let readiness = || {
            let (reply, receiver) = tokio::sync::oneshot::channel();
            let service_guard = PRODUCT_SERVICE_GATE.blocking_lock();
            managed_service::run_existing(paths.clone(), b"{\"operation\":\"readiness\"}".to_vec(),
                SERVICE_TIMEOUT, None, None, service_guard, attachment.clone(), reply);
            let bytes = receiver.blocking_recv().expect("managed Node owner returns")
                .expect("Node attaches to Tauri-owned native host");
            validate_product_readiness(&bytes).expect("native controller admission");
        };
        readiness();
        assert!(host.is_running().expect("retained host status"));
        assert_eq!(host.host_pid(), host_pid, "Node disconnect cannot replace host");
        let missing_instance_receipt = host.request_user(br#"{"schema":"gogoke.37.operations.v1","family":"K-INSTANCE","operation":"install-state","requestId":"probeA","targetId":"instanceA","domainId":"global","expectedRevision":"0","payload":{}}"#)
            .expect("retained User pipe remains connected after Node exit");
        let receipt: serde_json::Value = serde_json::from_slice(&missing_instance_receipt)
            .expect("native User receipt");
        assert_eq!(receipt["status"], "CONFLICT");
        assert_eq!(receipt["requestId"], "probeA");
        readiness();
        assert_eq!(host.host_pid(), host_pid, "second Node connects to same host");
        drop(host); // User EOF terminates the exact retained native host.
        std::fs::remove_dir_all(&product_root).expect("owned root released on host drop");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn outer_job_creation_owner_helper() {
        if std::env::var("GOGOKE_TEST_OUTER_JOB_CREATE_PAUSE").as_deref() != Ok("1") {
            return; // The parent test activates this helper in a separate process.
        }
        let node = PathBuf::from(std::env::var_os("GOGOKE_CONTROLLED_NODE_PATH")
            .expect("cloud test requires the staged signed Node runtime"));
        let root = PathBuf::from(std::env::var_os("GOGOKE_TEST_OUTER_JOB_ROOT")
            .expect("parent-owned fixture root"));
        managed_service::test_launch_paused(node, root);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn outer_job_owner_hard_exit_kills_suspended_node() {
        use std::io::{BufRead, BufReader};
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use std::process::{Child, Command, Stdio};
        use std::sync::mpsc;
        use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, TerminateProcess, WaitForSingleObject,
            PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };

        struct ExactProcessCleanup {
            owner: Child,
            node: Option<OwnedHandle>,
            node_pid: Option<u32>,
            owner_reaped: bool,
        }

        impl Drop for ExactProcessCleanup {
            fn drop(&mut self) {
                if self.node.is_none() && !self.owner_reaped {
                    if let Some(pid) = self.node_pid {
                        // The owner still holds its process handle here, so
                        // this PID cannot have been reused for another process.
                        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
                        if !handle.is_null() {
                            self.node = Some(unsafe { OwnedHandle::from_raw_handle(handle as _) });
                        }
                    }
                }
                if !self.owner_reaped {
                    let _ = self.owner.kill();
                    let _ = self.owner.wait();
                }
                if let Some(node) = &self.node {
                    let handle = node.as_raw_handle() as _;
                    if unsafe { WaitForSingleObject(handle, 0) } == WAIT_TIMEOUT {
                        unsafe { TerminateProcess(handle, 1) };
                        let _ = unsafe { WaitForSingleObject(handle, 5000) };
                    }
                }
            }
        }

        let node = PathBuf::from(std::env::var_os("GOGOKE_CONTROLLED_NODE_PATH")
            .expect("cloud test requires the staged signed Node runtime"));
        assert!(node.is_file(), "controlled Node must be the staged cloud executable");
        let root = std::env::temp_dir().join(format!(
            "gogoke-outer-job-exit-{}", uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(root.join("service/dist"))
            .expect("parent-owned service directory for Node current directory");
        let owner = Command::new(std::env::current_exe().expect("current cloud test binary"))
            .args(["outer_job_creation_owner_helper", "--nocapture"])
            .env("GOGOKE_TEST_OUTER_JOB_CREATE_PAUSE", "1")
            .env("GOGOKE_TEST_OUTER_JOB_ROOT", &root)
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn exact owning test process");
        let mut cleanup = ExactProcessCleanup {
            owner, node: None, node_pid: None, owner_reaped: false,
        };
        let stdout = cleanup.owner.stdout.take().expect("owner stdout pipe");
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let marker = BufReader::new(stdout).lines()
                .filter_map(Result::ok)
                .find_map(|line| line.split_once("GOGOKE_OUTER_JOB_CREATED:")
                    .and_then(|(_, pid)| pid.split_whitespace().next())
                    .and_then(|pid| pid.parse::<u32>().ok()));
            let _ = sender.send(marker);
        });
        let pid = receiver.recv_timeout(Duration::from_secs(30))
            .expect("one bounded wait for post-CreateProcess marker")
            .expect("owner must report exact suspended Node PID");
        cleanup.node_pid = Some(pid);
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
        assert!(!handle.is_null(), "open exact suspended Node before owner termination");
        cleanup.node = Some(unsafe { OwnedHandle::from_raw_handle(handle as _) });
        let node_handle = cleanup.node.as_ref().expect("exact Node handle").as_raw_handle() as _;
        assert_eq!(unsafe { WaitForSingleObject(node_handle, 0) }, WAIT_TIMEOUT,
            "Node must still be suspended when owner is terminated");
        cleanup.owner.kill().expect("hard-terminate exact owning test process");
        let owner_status = cleanup.owner.wait().expect("confirm owner process exit");
        cleanup.owner_reaped = true;
        assert!(!owner_status.success(), "owner must exit by termination");
        let node_wait = unsafe { WaitForSingleObject(node_handle, 5000) };
        drop(cleanup); // On failure, terminate and reap the exact Node before asserting.
        std::fs::remove_dir_all(&root).expect("remove parent-owned test fixture");
        assert_eq!(node_wait, WAIT_OBJECT_0,
            "closing the hard-terminated owner's kill-on-close Job must exit exact Node");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn managed_product_failure_reports_bounded_service_output() {
        use std::fs;
        let node = PathBuf::from(std::env::var_os("GOGOKE_CONTROLLED_NODE_PATH")
            .expect("cloud test requires the staged signed Node runtime"));
        assert!(node.is_file());
        let root = std::env::temp_dir().join(format!(
            "gogoke-service-output-test-{}", uuid::Uuid::new_v4().simple()
        ));
        let dist = root.join("service/dist");
        fs::create_dir_all(&dist).expect("owned fixture directory");
        let entry = dist.join("bin.mjs");
        fs::write(&entry,
            "process.stdout.write('service detail'); process.stderr.write('host detail'); process.exit(17);\n"
        ).expect("owned fixture entry");
        let paths = ProductRuntimePaths {
            node_runtime: node,
            service_entry: entry,
            native_host: root.join("unused-native-host.exe"),
            product_root: root.join("product"),
            source_commit: None,
            resource_set_id: None,
            runtime_lease: None,
        };
        let (reply, receiver) = tokio::sync::oneshot::channel();
        let service_guard = PRODUCT_SERVICE_GATE.blocking_lock();
        managed_service::run(paths, b"{}".to_vec(), Duration::from_secs(10), None, None, service_guard, reply);
        let error = receiver.blocking_recv().expect("managed owner reply").unwrap_err();
        assert!(error.starts_with("GOGOKE_PRODUCT_SERVICE_FAILED:17"), "{error}");
        assert!(error.contains(":STDOUT_TAIL:service detail"), "{error}");
        assert!(error.contains(":STDERR_TAIL:host detail"), "{error}");
        fs::remove_dir_all(&root).expect("remove settled owned fixture");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn managed_product_launch_rejects_poisoned_generation_module() {
        use std::fs;
        let node = PathBuf::from(std::env::var_os("GOGOKE_CONTROLLED_NODE_PATH")
            .expect("cloud test requires the staged signed Node runtime"));
        assert!(node.is_file());
        let root = std::env::temp_dir().join(format!(
            "gogoke-module-guard-test-{}", uuid::Uuid::new_v4().simple()
        ));
        let dist = root.join("service/generations/signed/dist");
        // Node searches this ancestor after the selected generation but
        // before the signed service-level node_modules directory.
        let poison = root.join("service/generations/node_modules/@ff-labs/fff-node");
        fs::create_dir_all(&dist).expect("owned service directory");
        fs::create_dir_all(&poison).expect("owned poison directory");
        let marker = root.join("poison-executed.txt");
        let entry = dist.join("bin.mjs");
        fs::write(&entry,
            "import { createRequire } from 'node:module';\nconst require = createRequire(import.meta.url);\ntry { require('@ff-labs/fff-node'); } catch { process.stdout.write('optional dependency skipped'); }\n"
        ).expect("owned entry");
        fs::write(poison.join("package.json"), b"{\"main\":\"index.cjs\"}\n")
            .expect("owned poison metadata");
        fs::write(poison.join("index.cjs"), format!(
            "require('node:fs').writeFileSync({}, 'executed');\n",
            serde_json::to_string(&marker.to_string_lossy().to_string()).expect("marker literal")
        )).expect("owned poison module");
        let mut lease = crate::resource_trust::RuntimeLease::default();
        lease.pin_generated_file(&entry, &fs::read(&entry).expect("entry bytes"))
            .expect("pin exact entry");
        let paths = ProductRuntimePaths {
            node_runtime: node,
            service_entry: entry,
            native_host: root.join("unused-native-host.exe"),
            product_root: root.join("product"),
            source_commit: None,
            resource_set_id: None,
            runtime_lease: Some(Arc::new(lease)),
        };
        let (reply, receiver) = tokio::sync::oneshot::channel();
        let service_guard = PRODUCT_SERVICE_GATE.blocking_lock();
        managed_service::run(paths, b"{}".to_vec(), Duration::from_secs(10), None, None, service_guard, reply);
        let result = receiver.blocking_recv().expect("managed owner reply");
        let error = result.expect_err("poisoned generation module must be rejected");
        assert!(error.starts_with("GOGOKE_PRODUCT_SERVICE_FAILED:78:STDERR_TAIL:GOGOKE_MODULE_NOT_LISTED"), "{error}");
        assert!(!marker.exists(), "poison module body must never execute");
        fs::remove_dir_all(&root).expect("remove owned fixture after settled Job");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn cancelled_reply_retains_job_and_lease_until_accounted_settlement() {
        use std::fs;
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::JobObjects::{
            IsProcessInJob, QueryInformationJobObject, JobObjectBasicAccountingInformation,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        };
        use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, WaitForSingleObject};

        let node = PathBuf::from(std::env::var_os("GOGOKE_CONTROLLED_NODE_PATH")
            .expect("cloud test requires the staged signed Node runtime"));
        assert!(node.is_file(), "controlled Node must be the staged cloud executable");
        let holder = PathBuf::from(std::env::var_os("GOGOKE_CONTROLLED_HOLD_CHILD_PATH")
            .expect("cloud test requires the signed file-holding child"));
        assert!(holder.is_file(), "controlled child must be a cloud executable");
        let root = std::env::temp_dir().join(format!(
            "gogoke-product-job-test-{}", uuid::Uuid::new_v4().simple()
        ));
        let service = root.join("service");
        let dist = service.join("dist");
        fs::create_dir_all(&dist).expect("owned test service directory");
        let task_dir = root.join("task");
        fs::create_dir(&task_dir).expect("owned task directory");
        let held_file = task_dir.join("held.bin");
        fs::write(&held_file, b"task bytes held by child").expect("owned task file");
        let marker = root.join("processes.txt");
        let child_ready = root.join("child-ready.txt");
        let release = root.join("release.txt");
        let entry = dist.join("bin.mjs");
        let marker_literal = serde_json::to_string(&marker.to_string_lossy().to_string())
            .expect("test marker path literal");
        let release_literal = serde_json::to_string(&release.to_string_lossy().to_string())
            .expect("release path literal");
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let nonce_literal = serde_json::to_string(&nonce).expect("test nonce literal");
        let ps_quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "''"));
        let child_script = format!(
            "$stream = [IO.FileStream]::new({}, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read);\n\
             [IO.File]::WriteAllText({}, '{}' + ' ' + $PID);\n\
             while ($true) {{ Start-Sleep -Seconds 1 }}\n",
            ps_quote(&held_file), ps_quote(&child_ready), nonce
        );
        let child_literal = serde_json::to_string(&child_script).expect("child script literal");
        let holder_literal = serde_json::to_string(&holder.to_string_lossy().to_string())
            .expect("controlled holder path literal");
        fs::write(&entry, format!(
            "import {{ spawn }} from 'node:child_process';\n\
             import {{ writeFileSync, existsSync }} from 'node:fs';\n\
             const child = spawn({holder_literal}, ['-NoProfile', '-NonInteractive', '-Command', {child_literal}], {{ stdio: 'ignore', windowsHide: true }});\n\
             writeFileSync({marker_literal}, String(process.pid) + ' ' + String(child.pid) + ' ' + {nonce_literal});\n\
             child.unref();\n\
             const hold = setInterval(() => {{ if (existsSync({release_literal})) {{ clearInterval(hold); process.exit(0); }} }}, 25);\n"
        )).expect("owned test service script");
        let mut pinned = crate::resource_trust::RuntimeLease::default();
        pinned.pin_generated_file(&entry, &fs::read(&entry).expect("owned service bytes"))
            .expect("lease owned service entry");
        let lease = Arc::new(pinned);
        let weak = Arc::downgrade(&lease);
        let paths = ProductRuntimePaths {
            node_runtime: node,
            service_entry: entry.clone(),
            native_host: root.join("unused-native-host.exe"),
            product_root: root.join("product"),
            source_commit: None,
            resource_set_id: None,
            runtime_lease: Some(lease),
        };
        let (reply, receiver) = tokio::sync::oneshot::channel();
        let gate = PRODUCT_RUNTIME_GATE.try_lock().expect("owned product gate");
        let service_guard = PRODUCT_SERVICE_GATE.blocking_lock();
        drop(receiver); // Simulate a cancelled Tauri caller before Node starts.
        let owner = std::thread::spawn(move || {
            // Cold PowerShell startup on a shared cloud runner is fixture setup,
            // not the Job-settlement boundary under test.
            managed_service::run(paths, b"{}".to_vec(), Duration::from_secs(30), None, Some(gate), service_guard, reply);
        });
        let fixture_started = Instant::now();
        let deadline = fixture_started + Duration::from_secs(20);
        while (!marker.is_file() || !child_ready.is_file() || managed_service::test_job_handle().is_null())
            && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let marker_text = fs::read_to_string(&marker).expect("root process marker");
        let fields: Vec<&str> = marker_text.split_whitespace().collect();
        assert_eq!(fields.len(), 3, "root/child/nonce marker");
        let root_pid: u32 = fields[0].parse().expect("root PID");
        let child_pid: u32 = fields[1].parse().expect("child PID");
        assert_eq!(fields[2], nonce, "root marker nonce");
        if !child_ready.is_file() {
            let child_handle = unsafe { OpenProcess(0x0010_1000, 0, child_pid) };
            let child_wait = if child_handle.is_null() { None } else {
                let child_handle = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(child_handle as _) };
                Some(unsafe { WaitForSingleObject(child_handle.as_raw_handle() as _, 0) })
            };
            panic!("child fixture did not become ready within {:?}; child_wait={child_wait:?}; job_present={}",
                fixture_started.elapsed(), !managed_service::test_job_handle().is_null());
        }
        assert_eq!(fs::read_to_string(&child_ready).expect("child self-ready marker"),
            format!("{nonce} {child_pid}"));
        let root_handle = unsafe { OpenProcess(0x0010_1000, 0, root_pid) };
        let child_handle = unsafe { OpenProcess(0x0010_1001, 0, child_pid) };
        assert!(!root_handle.is_null() && !child_handle.is_null(), "exact live root and child handles");
        let root_handle = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(root_handle as _) };
        let child = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(child_handle as _) };
        let job = managed_service::test_job_handle();
        let mut root_in_job = 0;
        let mut child_in_job = 0;
        assert_ne!(unsafe { IsProcessInJob(root_handle.as_raw_handle() as _, job, &mut root_in_job) }, 0);
        assert_ne!(unsafe { IsProcessInJob(child.as_raw_handle() as _, job, &mut child_in_job) }, 0);
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        assert_ne!(unsafe { QueryInformationJobObject(job, JobObjectBasicAccountingInformation,
            (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            std::ptr::null_mut()) }, 0);
        let active_before_release = accounting.ActiveProcesses;
        let root_wait_before = unsafe { WaitForSingleObject(root_handle.as_raw_handle() as _, 0) };
        let child_wait_before = unsafe { WaitForSingleObject(child.as_raw_handle() as _, 0) };
        let lease_held_before = weak.upgrade().is_some();
        let gate_held_before = PRODUCT_RUNTIME_GATE.try_lock().is_err();
        let held_open_probe = fs::OpenOptions::new().write(true).open(&held_file);
        let held_open_denied = held_open_probe.as_ref().err()
            .and_then(std::io::Error::raw_os_error) == Some(32);
        drop(held_open_probe);
        fs::write(&release, b"release").expect("release owned root barrier");
        owner.join().expect("process owner settles after cancelling reply");
        // These are the two acceptance operations. Each is attempted exactly
        // once, immediately after owner settlement, with no retry or sleep.
        let file_delete = fs::remove_file(&held_file);
        let dir_delete = fs::remove_dir(&task_dir);
        let root_wait_after = unsafe { WaitForSingleObject(root_handle.as_raw_handle() as _, 0) };
        let child_wait_after = unsafe { WaitForSingleObject(child.as_raw_handle() as _, 0) };
        let lease_released_after = weak.upgrade().is_none();
        let gate_released_after = PRODUCT_RUNTIME_GATE.try_lock().is_ok();
        if child_wait_after == WAIT_TIMEOUT {
            unsafe { TerminateProcess(child.as_raw_handle() as _, 1) };
            let _ = unsafe { WaitForSingleObject(child.as_raw_handle() as _, 5000) };
        }
        eprintln!("JOB_CUSTODY_EVIDENCE root_in={root_in_job} child_in={child_in_job} active_before={active_before_release} root_before={root_wait_before} child_before={child_wait_before} root_after={root_wait_after} child_after={child_wait_after} held_open_denied={held_open_denied} file_delete={file_delete:?} dir_delete={dir_delete:?} lease_before={lease_held_before} gate_before={gate_held_before} lease_after={lease_released_after} gate_after={gate_released_after}");
        if file_delete.is_err() || dir_delete.is_err() {
            panic!("task file/directory one-shot delete failed after owner settlement; retained owned fixture: {}", root.display());
        }
        drop(child);
        drop(root_handle);
        fs::remove_file(&marker).expect("remove owned PID marker");
        fs::remove_file(&child_ready).expect("remove owned child ready marker");
        fs::remove_file(&release).expect("remove owned release marker");
        fs::remove_file(&entry).expect("remove owned service script");
        fs::remove_dir(&dist).expect("remove owned dist directory");
        fs::remove_dir(&service).expect("remove owned service directory");
        fs::remove_dir(root.join("product")).expect("remove owned product root");
        fs::remove_dir(&root).expect("remove owned fixture root");
        assert_eq!(root_in_job, 1, "root must be in exact Gogoke Job");
        assert_eq!(child_in_job, 1, "child must be in exact Gogoke Job");
        assert!(active_before_release >= 2, "Job must contain active root and child");
        assert_eq!(root_wait_before, WAIT_TIMEOUT);
        assert_eq!(child_wait_before, WAIT_TIMEOUT);
        assert!(held_open_denied, "child file holder must deny mutation before release");
        assert!(lease_held_before && gate_held_before, "cancelled caller cannot release custody while Job active");
        assert_eq!(root_wait_after, WAIT_OBJECT_0, "owned root handle must signal before settlement");
        assert!(lease_released_after && gate_released_after, "owner releases custody after settlement");
        let lane = std::env::var("GOGOKE_BUILD_LANE").expect("cloud build lane");
        assert!(lane == "frozen" || lane == "repro", "controlled build lane");
        let receipt = serde_json::json!({
            "schema": "gogoke.r2-06a.job-custody-delete.v1",
            "sourceCommit": std::env::var("GITHUB_SHA").expect("exact cloud source SHA"),
            "runId": std::env::var("GITHUB_RUN_ID").expect("exact cloud run ID"),
            "runAttempt": std::env::var("GITHUB_RUN_ATTEMPT").expect("exact cloud run attempt"),
            "lane": lane.clone(),
            "state": "PASS",
            "rootInExactJob": true,
            "childInExactJob": true,
            "childHeldTaskFileBeforeSettlement": true,
            "oneShotFileDeleteAfterOwner": true,
            "oneShotDirectoryDeleteAfterOwner": true,
            "rootHandleSignaledAtOwnerReturn": true,
            "childHandleWaitAtOwnerReturn": child_wait_after,
            "leaseHeldWhileJobActive": true,
            "gateHeldWhileJobActive": true,
        });
        let receipt_path = PathBuf::from(std::env::var_os("RUNNER_TEMP").expect("cloud runner temp"))
            .join(format!("gogoke-job-custody-delete-{lane}.json"));
        fs::write(receipt_path, serde_json::to_vec_pretty(&receipt).expect("receipt JSON"))
            .expect("write cloud custody receipt");
    }

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
