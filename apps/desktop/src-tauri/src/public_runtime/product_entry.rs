use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tauri::Manager;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use sha2::{Digest, Sha256};

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

#[derive(Clone, Debug)]
struct ProductRuntimePaths {
    node_runtime: PathBuf,
    service_entry: PathBuf,
    native_host: PathBuf,
    product_root: PathBuf,
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
    let service_root = resource_dir.join("gogoke-service");
    let node_runtime = require_file(service_root.join("runtime").join("node.exe"), "node-runtime")?;
    let service_entry = require_file(service_root.join("dist").join("bin.mjs"), "service-entry")?;

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
    let native_host = native_candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "GOGOKE_PRODUCT_COMPONENT_MISSING:native-host".to_string())?;
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
) -> Result<Vec<u8>, String> {
    tokio::fs::create_dir_all(&paths.product_root)
        .await
        .map_err(|_| "GOGOKE_PRODUCT_ROOT_UNAVAILABLE".to_string())?;
    let mut command = Command::new(&paths.node_runtime);
    if let Some((sha, entry_hash)) = draft_identity {
        command.env("GOGOKE_EXECUTION_EVIDENCE_SHA", sha)
            .env("GOGOKE_SERVICE_ENTRY_SHA256", entry_hash);
    }
    command
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .arg(&paths.service_entry)
        .arg("--root")
        .arg(&paths.product_root)
        .arg("--native-host")
        .arg(&paths.native_host)
        .current_dir(
            paths
                .service_entry
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| "GOGOKE_PRODUCT_SERVICE_ROOT_UNAVAILABLE".to_string())?,
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|_| "GOGOKE_PRODUCT_SERVICE_START_FAILED".to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "GOGOKE_PRODUCT_SERVICE_STDIN_UNAVAILABLE".to_string())?;
    stdin
        .write_all(request_bytes)
        .await
        .map_err(|_| "GOGOKE_PRODUCT_SERVICE_REQUEST_FAILED".to_string())?;
    stdin
        .shutdown()
        .await
        .map_err(|_| "GOGOKE_PRODUCT_SERVICE_REQUEST_CLOSE_FAILED".to_string())?;
    drop(stdin);

    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| "GOGOKE_PRODUCT_SERVICE_TIMEOUT".to_string())?
        .map_err(|_| "GOGOKE_PRODUCT_SERVICE_WAIT_FAILED".to_string())?;
    if !output.status.success() {
        return Err(format!(
            "GOGOKE_PRODUCT_SERVICE_FAILED:{}",
            output.status.code().unwrap_or(-1)
        ));
    }
    Ok(output.stdout)
}

async fn run_product_process(
    paths: ProductRuntimePaths,
    request: &ProductGoalRequest,
) -> Result<ProductGoalView, String> {
    let request_bytes =
        serde_json::to_vec(request).map_err(|_| "GOGOKE_PRODUCT_REQUEST_ENCODE_FAILED".to_string())?;
    let draft_identity = if request.publish_test_draft == Some(true) {
        if request.run_controlled_task != Some(true) {
            return Err("GOGOKE_TEST_DRAFT_REQUIRES_CONTROLLED_TASK".to_string());
        }
        let sha = option_env!("GITHUB_SHA")
            .ok_or_else(|| "GOGOKE_TEST_DRAFT_BUILD_SHA_UNAVAILABLE".to_string())?;
        if sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("GOGOKE_TEST_DRAFT_BUILD_SHA_INVALID".to_string());
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
        draft_identity.as_ref().map(|(sha, hash)| (sha.as_str(), hash.as_str()))).await?;
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
    let output = run_product_service(paths, b"{\"operation\":\"readiness\"}", SERVICE_TIMEOUT, None).await?;
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
    let paths = resolve_runtime_paths(&app)?;
    run_product_process(paths, &request).await
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
