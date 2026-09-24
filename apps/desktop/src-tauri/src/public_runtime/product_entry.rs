use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tauri::Manager;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const SERVICE_TIMEOUT: Duration = Duration::from_secs(20);

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
pub(crate) struct ProductGoalView {
    goal: GoalRef,
    ledger: LedgerRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_controlled_task: Option<bool>,
    caller: ProductCallerView,
    native_host: NativeHostView,
    ledger_readback: LedgerReadbackView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    controlled_task: Option<ControlledTaskView>,
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
fn resolve_runtime_paths(app: &tauri::AppHandle) -> Result<ProductRuntimePaths, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|_| "GOGOKE_PRODUCT_RESOURCE_DIR_UNAVAILABLE".to_string())?;
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

    let product_root = app
        .path()
        .app_data_dir()
        .map_err(|_| "GOGOKE_PRODUCT_DATA_DIR_UNAVAILABLE".to_string())?
        .join("product-authority");
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
        {
            return Err("GOGOKE_CONTROLLED_TASK_NOT_VALIDATED".to_string());
        }
    }
    Ok(())
}

async fn run_product_process(
    paths: ProductRuntimePaths,
    request: &ProductGoalRequest,
) -> Result<ProductGoalView, String> {
    tokio::fs::create_dir_all(&paths.product_root)
        .await
        .map_err(|_| "GOGOKE_PRODUCT_ROOT_UNAVAILABLE".to_string())?;
    let request_bytes =
        serde_json::to_vec(request).map_err(|_| "GOGOKE_PRODUCT_REQUEST_ENCODE_FAILED".to_string())?;

    let mut command = Command::new(&paths.node_runtime);
    command
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
        .write_all(&request_bytes)
        .await
        .map_err(|_| "GOGOKE_PRODUCT_SERVICE_REQUEST_FAILED".to_string())?;
    stdin
        .shutdown()
        .await
        .map_err(|_| "GOGOKE_PRODUCT_SERVICE_REQUEST_CLOSE_FAILED".to_string())?;
    drop(stdin);

    let output = tokio::time::timeout(SERVICE_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "GOGOKE_PRODUCT_SERVICE_TIMEOUT".to_string())?
        .map_err(|_| "GOGOKE_PRODUCT_SERVICE_WAIT_FAILED".to_string())?;
    if !output.status.success() {
        return Err(format!(
            "GOGOKE_PRODUCT_SERVICE_FAILED:{}",
            output.status.code().unwrap_or(-1)
        ));
    }
    let response: ProductGoalView = serde_json::from_slice(&output.stdout)
        .map_err(|_| "GOGOKE_PRODUCT_RESPONSE_DECODE_FAILED".to_string())?;
    validate_product_response(&response)?;
    if response.goal.id != request.goal.id
        || response.goal.title != request.goal.title
        || response.ledger.repository != request.ledger.repository
        || response.ledger.commit != request.ledger.commit
        || response.ledger.path != request.ledger.path
        || response.ledger.content_hash != request.ledger.content_hash
        || response.run_controlled_task != request.run_controlled_task
        || (request.run_controlled_task == Some(true)) != response.controlled_task.is_some()
    {
        return Err("GOGOKE_PRODUCT_RESPONSE_IDENTITY_MISMATCH".to_string());
    }
    Ok(response)
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
        };
        let valid = ProductGoalView {
            goal: request.goal.clone(),
            ledger: request.ledger.clone(),
            run_controlled_task: None,
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
            controlled_task: None,
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
