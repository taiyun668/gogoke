use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

use tauri::{AppHandle, Emitter, State};

pub(crate) mod args;
pub(crate) mod config;
pub(crate) mod home;

use crate::backend::app_server::spawn_workspace_session as spawn_workspace_session_inner;
use crate::backend::app_server::NativeAssociation;
pub(crate) use crate::backend::app_server::WorkspaceSession;
use crate::backend::events::AppServerEvent;
use crate::event_sink::TauriEventSink;
use crate::remote_backend;
use crate::shared::agents_config_core;
use crate::shared::codex_core::{self, insert_optional_nullable_string};
use crate::state::AppState;
use crate::types::WorkspaceEntry;

const VISIBLE_SCHEMA: &str = "gogoke.37.visible-conversation.v1";
static VISIBLE_APP: OnceLock<AppHandle> = OnceLock::new();

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VisibleRouteReply {
    schema: String,
    workspace_id: String,
    state: String,
    #[serde(default)]
    association: Option<NativeAssociation>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VisibleOperationReply {
    schema: String,
    workspace_id: String,
    request_id: String,
    state: String,
    #[serde(default)]
    response: Option<Value>,
    #[serde(default)]
    stop_fact: Option<String>,
    #[serde(default)]
    live: Option<bool>,
    #[serde(default)]
    association: Option<NativeAssociation>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VisibleReadReply {
    schema: String,
    workspace_id: String,
    #[serde(default)]
    association: Option<NativeAssociation>,
    state: String,
    #[serde(default)]
    response: Option<Value>,
    #[serde(default)]
    live: Option<bool>,
    #[serde(default)]
    reason: Option<String>,
}

async fn visible_user_frame(app: &AppHandle, frame: Value) -> Result<String, String> {
    crate::public_runtime::product_entry::gogoke_design37_user_operation(
        app.clone(),
        frame.to_string(),
    )
    .await
}

async fn visible_route(app: &AppHandle, workspace_id: &str) -> Result<VisibleRouteReply, String> {
    let raw = visible_user_frame(
        app,
        json!({
            "schema": "gogoke.37.owner-configuration.v1",
            "command": "visible-conversation-route",
            "workspaceId": workspace_id,
        }),
    )
    .await?;
    let reply: VisibleRouteReply = serde_json::from_str(&raw)
        .map_err(|error| format!("GOGOKE_VISIBLE_ROUTE_REPLY_INVALID:{error}"))?;
    if reply.schema != VISIBLE_SCHEMA || reply.workspace_id != workspace_id {
        return Err("GOGOKE_VISIBLE_ROUTE_IDENTITY_MISMATCH".into());
    }
    match reply.state.as_str() {
        "LEGACY" if reply.association.is_none() => Ok(reply),
        "NATIVE" if reply.association.is_some() => Ok(reply),
        "LEGACY" | "NATIVE" => Err("GOGOKE_VISIBLE_ROUTE_ASSOCIATION_INVALID".into()),
        "NEEDS_SETUP" | "UNKNOWN" => {
            Err(visible_failure(&reply.state, reply.reason.as_deref(), None))
        }
        _ => Err("GOGOKE_VISIBLE_ROUTE_STATE_INVALID".into()),
    }
}

/// A routing preflight must run before any legacy shared-session reuse.
/// None is the daemon/test process, which has no User host attachment.
pub(crate) async fn native_visible_route_preflight(
    workspace_id: &str,
) -> Result<Option<bool>, String> {
    let Some(app) = VISIBLE_APP.get() else {
        return Ok(None);
    };
    visible_route(app, workspace_id)
        .await
        .map(|route| Some(route.state == "NATIVE"))
}

fn native_visible_params(method: &str, params: &Value) -> Result<(), String> {
    let fields = params
        .as_object()
        .ok_or("GOGOKE_NATIVE_PARAMS_OBJECT_REQUIRED")?;
    let allowed: &[&str] = match method {
        "thread/start" => &[],
        "thread/resume" => &["threadId"],
        "thread/read" => &["threadId", "includeTurns", "cursor"],
        "thread/list" => &["cursor", "limit"],
        "turn/start" => &["threadId", "input"],
        "turn/steer" => &["threadId", "expectedTurnId", "input"],
        "turn/interrupt" => &["threadId", "turnId"],
        "physical-stop" => &[],
        "original-question-answer" => &["requestId", "result"],
        _ => return Err(format!("GOGOKE_NATIVE_METHOD_UNSUPPORTED:{method}")),
    };
    if let Some(field) = fields
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(format!("GOGOKE_NATIVE_FIELD_UNSUPPORTED:{field}"));
    }
    if matches!(method, "turn/start" | "turn/steer") {
        let input = fields
            .get("input")
            .and_then(Value::as_array)
            .ok_or("GOGOKE_NATIVE_INPUT_INVALID")?;
        if input.len() != 1
            || input[0].get("type").and_then(Value::as_str) != Some("text")
            || input[0].get("text").and_then(Value::as_str).is_none()
            || input[0].as_object().is_none_or(|item| item.len() != 2)
        {
            return Err("GOGOKE_NATIVE_INPUT_UNSUPPORTED".into());
        }
    }
    Ok(())
}

async fn visible_operation(
    app: &AppHandle,
    workspace_id: &str,
    command: &str,
    association: &NativeAssociation,
    request_id: &str,
    method: Option<&str>,
    params: Option<Value>,
) -> Result<VisibleOperationReply, String> {
    let mut frame = json!({
        "schema": "gogoke.37.owner-configuration.v1",
        "command": command,
        "workspaceId": workspace_id,
        "requestId": request_id,
        "expectedAssociation": association,
    });
    if let Some(method) = method {
        frame["method"] = json!(method);
    }
    if let Some(params) = params {
        frame["params"] = params;
    }
    let raw = visible_user_frame(app, frame).await?;
    let reply: VisibleOperationReply = serde_json::from_str(&raw)
        .map_err(|error| format!("GOGOKE_VISIBLE_OPERATION_REPLY_INVALID:{error}"))?;
    if reply.schema != VISIBLE_SCHEMA
        || reply.workspace_id != workspace_id
        || reply.request_id != request_id
    {
        return Err("GOGOKE_VISIBLE_OPERATION_IDENTITY_MISMATCH".into());
    }
    if reply.association.as_ref() != Some(association) {
        return Err(format!(
            "GOGOKE_VISIBLE_OPERATION_ASSOCIATION_MISMATCH:{}",
            reply.reason.as_deref().unwrap_or("reason missing")
        ));
    }
    Ok(reply)
}

async fn visible_read(
    app: &AppHandle,
    workspace_id: &str,
    association: &NativeAssociation,
    method: &str,
    params: Option<Value>,
) -> Result<VisibleReadReply, String> {
    let mut frame = json!({
        "schema": "gogoke.37.owner-configuration.v1",
        "command": "visible-conversation-read",
        "workspaceId": workspace_id,
        "expectedAssociation": association,
        "method": method,
    });
    if let Some(params) = params {
        frame["params"] = params;
    }
    let raw = visible_user_frame(app, frame).await?;
    let reply: VisibleReadReply = serde_json::from_str(&raw)
        .map_err(|error| format!("GOGOKE_VISIBLE_READ_REPLY_INVALID:{error}"))?;
    if reply.schema != VISIBLE_SCHEMA || reply.workspace_id != workspace_id {
        return Err("GOGOKE_VISIBLE_READ_IDENTITY_MISMATCH".into());
    }
    if reply.association.as_ref() != Some(association) {
        return Err(format!(
            "GOGOKE_VISIBLE_READ_ASSOCIATION_MISMATCH:{}",
            reply.reason.as_deref().unwrap_or("reason missing")
        ));
    }
    Ok(reply)
}

fn visible_failure(state: &str, reason: Option<&str>, request_id: Option<&str>) -> String {
    let reason = reason
        .filter(|reason| !reason.is_empty())
        .unwrap_or("GOGOKE_VISIBLE_REASON_MISSING");
    match request_id {
        Some(request_id) => format!("GOGOKE_VISIBLE_{state}:{request_id}:{reason}"),
        None => format!("GOGOKE_VISIBLE_{state}:{reason}"),
    }
}

pub(crate) async fn native_visible_request(
    app: &AppHandle,
    workspace_id: &str,
    association: &NativeAssociation,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    native_visible_params(method, &params)?;
    if !matches!(method, "thread/read" | "thread/list") {
        return Err("GOGOKE_NATIVE_STABLE_INTENT_REQUIRED".into());
    }
    if method == "thread/read" {
        return native_visible_history(app, workspace_id, association, params).await;
    }
    let reply = visible_read(app, workspace_id, association, method, Some(params)).await?;
    match reply.state.as_str() {
        "PARTIAL" | "APPLIED" => {
            let response = reply.response.ok_or("GOGOKE_VISIBLE_RESPONSE_MISSING")?;
            let result = response
                .get("result")
                .and_then(Value::as_object)
                .ok_or("GOGOKE_NATIVE_THREAD_LIST_RESULT_INVALID")?;
            if result.get("data").and_then(Value::as_array).is_none() {
                return Err("GOGOKE_NATIVE_THREAD_LIST_DATA_INVALID".into());
            }
            let history = result
                .get("nativeHistory")
                .and_then(Value::as_object)
                .ok_or("GOGOKE_NATIVE_THREAD_LIST_MARKER_MISSING")?;
            let expected_state = if reply.state == "PARTIAL" {
                "PARTIAL"
            } else {
                "COMPLETE"
            };
            if history.get("state").and_then(Value::as_str) != Some(expected_state)
                || history
                    .get("highWater")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                || history
                    .get("sourceRefs")
                    .and_then(Value::as_array)
                    .is_none()
            {
                return Err("GOGOKE_NATIVE_THREAD_LIST_MARKER_INVALID".into());
            }
            let next = history
                .get("nextCursor")
                .ok_or("GOGOKE_NATIVE_THREAD_LIST_CURSOR_MISSING")?;
            if result.get("nextCursor") != Some(next)
                || (reply.state == "PARTIAL" && next.as_str().is_none_or(str::is_empty))
                || (reply.state == "APPLIED" && !next.is_null())
            {
                return Err("GOGOKE_NATIVE_THREAD_LIST_CURSOR_INVALID".into());
            }
            Ok(response)
        }
        "UNKNOWN" | "DENIED" | "UNSUPPORTED" => {
            Err(visible_failure(&reply.state, reply.reason.as_deref(), None))
        }
        _ => Err("GOGOKE_VISIBLE_OPERATION_STATE_INVALID".into()),
    }
}

async fn native_visible_history(
    app: &AppHandle,
    workspace_id: &str,
    association: &NativeAssociation,
    mut params: Value,
) -> Result<Value, String> {
    let fields = params
        .as_object_mut()
        .ok_or("GOGOKE_NATIVE_HISTORY_PARAMS_INVALID")?;
    let thread_id = fields
        .get("threadId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("GOGOKE_NATIVE_HISTORY_THREAD_ID_INVALID")?
        .to_owned();
    if fields.get("includeTurns") != Some(&Value::Bool(true))
        || fields.get("cursor").is_some_and(|cursor| !cursor.is_null())
    {
        return Err("GOGOKE_NATIVE_HISTORY_FULL_READ_REQUIRED".into());
    }
    let mut high_water: Option<String> = None;
    let mut thread_base: Option<Value> = None;
    let mut turns = Vec::new();
    let mut source_refs = Vec::new();
    let mut seen_cursors = HashSet::new();
    let mut seen_turns = HashSet::new();
    let mut seen_sources = HashSet::new();
    loop {
        let reply = visible_read(
            app,
            workspace_id,
            association,
            "thread/read",
            Some(params.clone()),
        )
        .await?;
        if !matches!(reply.state.as_str(), "PARTIAL" | "APPLIED") {
            return Err(visible_failure(&reply.state, reply.reason.as_deref(), None));
        }
        let mut response = reply
            .response
            .ok_or("GOGOKE_NATIVE_HISTORY_RESPONSE_MISSING")?;
        let result = response
            .get_mut("result")
            .and_then(Value::as_object_mut)
            .ok_or("GOGOKE_NATIVE_HISTORY_RESULT_INVALID")?;
        let thread = result
            .get("thread")
            .and_then(Value::as_object)
            .ok_or("GOGOKE_NATIVE_HISTORY_THREAD_INVALID")?;
        if thread.get("id").and_then(Value::as_str) != Some(thread_id.as_str()) {
            return Err("GOGOKE_NATIVE_HISTORY_THREAD_ID_MISMATCH".into());
        }
        let page_turns = thread
            .get("turns")
            .and_then(Value::as_array)
            .ok_or("GOGOKE_NATIVE_HISTORY_TURNS_INVALID")?;
        for turn in page_turns {
            let turn_id = turn
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or("GOGOKE_NATIVE_HISTORY_TURN_ID_INVALID")?;
            if !seen_turns.insert(turn_id.to_owned()) {
                return Err("GOGOKE_NATIVE_HISTORY_TURN_REPEATED".into());
            }
        }
        turns.extend(page_turns.iter().cloned());
        let mut base = Value::Object(thread.clone());
        base.as_object_mut()
            .ok_or("GOGOKE_NATIVE_HISTORY_THREAD_INVALID")?
            .remove("turns");
        if let Some(first) = &thread_base {
            if first != &base {
                return Err("GOGOKE_NATIVE_HISTORY_THREAD_CHANGED".into());
            }
        } else {
            thread_base = Some(base);
        }
        let history = result
            .get("nativeHistory")
            .and_then(Value::as_object)
            .ok_or("GOGOKE_NATIVE_HISTORY_MARKER_MISSING")?;
        let expected_history_state = if reply.state == "PARTIAL" {
            "PARTIAL"
        } else {
            "COMPLETE"
        };
        if history.get("state").and_then(Value::as_str) != Some(expected_history_state) {
            return Err("GOGOKE_NATIVE_HISTORY_STATE_MISMATCH".into());
        }
        let page_high_water = history
            .get("highWater")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or("GOGOKE_NATIVE_HISTORY_HIGH_WATER_INVALID")?;
        if let Some(first) = &high_water {
            if first != page_high_water {
                return Err("GOGOKE_NATIVE_HISTORY_HIGH_WATER_CHANGED".into());
            }
        } else {
            high_water = Some(page_high_water.to_owned());
        }
        let refs = history
            .get("sourceRefs")
            .and_then(Value::as_array)
            .ok_or("GOGOKE_NATIVE_HISTORY_SOURCE_REFS_INVALID")?;
        for source in refs {
            let fields = source
                .as_object()
                .ok_or("GOGOKE_NATIVE_HISTORY_SOURCE_REF_INVALID")?;
            if [
                "operationId",
                "generation",
                "sourceEpoch",
                "sourceCursor",
                "rawSourceId",
            ]
            .iter()
            .any(|key| {
                fields
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
            }) {
                return Err("GOGOKE_NATIVE_HISTORY_SOURCE_REF_INVALID".into());
            }
            let identity = serde_json::to_string(source).map_err(|error| {
                format!("GOGOKE_NATIVE_HISTORY_SOURCE_REF_ENCODE_FAILED:{error}")
            })?;
            if !seen_sources.insert(identity) {
                return Err("GOGOKE_NATIVE_HISTORY_SOURCE_REF_REPEATED".into());
            }
        }
        source_refs.extend(refs.iter().cloned());
        let next = history
            .get("nextCursor")
            .ok_or("GOGOKE_NATIVE_HISTORY_CURSOR_MISSING")?;
        let next = if next.is_null() {
            None
        } else {
            Some(
                next.as_str()
                    .filter(|value| !value.is_empty())
                    .ok_or("GOGOKE_NATIVE_HISTORY_CURSOR_INVALID")?
                    .to_owned(),
            )
        };
        if reply.state == "PARTIAL" {
            let cursor = next.ok_or("GOGOKE_NATIVE_HISTORY_PARTIAL_WITHOUT_CURSOR")?;
            if !seen_cursors.insert(cursor.clone()) {
                return Err("GOGOKE_NATIVE_HISTORY_CURSOR_REPEATED".into());
            }
            params["cursor"] = Value::String(cursor);
            continue;
        }
        if next.is_some() {
            return Err("GOGOKE_NATIVE_HISTORY_APPLIED_WITH_CURSOR".into());
        }
        let result = response
            .get_mut("result")
            .and_then(Value::as_object_mut)
            .ok_or("GOGOKE_NATIVE_HISTORY_RESULT_INVALID")?;
        let thread = result
            .get_mut("thread")
            .and_then(Value::as_object_mut)
            .ok_or("GOGOKE_NATIVE_HISTORY_THREAD_INVALID")?;
        thread.insert("turns".to_string(), Value::Array(turns));
        let history = result
            .get_mut("nativeHistory")
            .and_then(Value::as_object_mut)
            .ok_or("GOGOKE_NATIVE_HISTORY_MARKER_MISSING")?;
        history.insert("state".to_string(), Value::String("COMPLETE".into()));
        history.insert("sourceRefs".to_string(), Value::Array(source_refs));
        return Ok(response);
    }
}

fn stable_native_request_id(value: Option<String>) -> Result<String, String> {
    let value = value.ok_or("GOGOKE_NATIVE_STABLE_INTENT_REQUIRED")?;
    let bytes = value.as_bytes();
    if !(1..=64).contains(&bytes.len())
        || !bytes[0].is_ascii_alphabetic()
        || !bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
    {
        return Err("GOGOKE_NATIVE_STABLE_INTENT_INVALID".into());
    }
    Ok(value)
}

async fn native_visible_effect(
    app: &AppHandle,
    state: &AppState,
    workspace_id: &str,
    native_request_id: Option<String>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    native_visible_params(method, &params)?;
    let request_id = stable_native_request_id(native_request_id)?;
    let association = native_association(state, workspace_id).await?;
    let reply = visible_operation(
        app,
        workspace_id,
        "visible-conversation-operate",
        &association,
        &request_id,
        Some(method),
        Some(params),
    )
    .await?;
    match reply.state.as_str() {
        "APPLIED" => reply
            .response
            .ok_or("GOGOKE_VISIBLE_RESPONSE_MISSING".into()),
        "UNKNOWN" | "DENIED" | "UNSUPPORTED" => Err(visible_failure(
            &reply.state,
            reply.reason.as_deref(),
            Some(&request_id),
        )),
        _ => Err("GOGOKE_VISIBLE_OPERATION_STATE_INVALID".into()),
    }
}

pub(crate) async fn native_visible_stop_with_intent(
    app: &AppHandle,
    state: &AppState,
    workspace_id: &str,
    native_request_id: Option<String>,
) -> Result<(), String> {
    let request_id = stable_native_request_id(native_request_id)?;
    let association = native_association(state, workspace_id).await?;
    let reply = visible_operation(
        app,
        workspace_id,
        "visible-conversation-operate",
        &association,
        &request_id,
        Some("physical-stop"),
        Some(json!({})),
    )
    .await?;
    if reply.state == "APPLIED"
        && reply
            .stop_fact
            .as_deref()
            .is_some_and(|fact| !fact.is_empty())
    {
        state
            .sessions
            .lock()
            .await
            .get(workspace_id)
            .ok_or("GOGOKE_NATIVE_SESSION_DISAPPEARED")?
            .note_native_stop_fact()?;
        Ok(())
    } else {
        Err(visible_failure(
            &reply.state,
            reply.reason.as_deref(),
            Some(&request_id),
        ))
    }
}

#[tauri::command]
pub(crate) async fn recover_native_visible_request(
    workspace_id: String,
    native_request_id: String,
    expected_association: Option<NativeAssociation>,
    app: AppHandle,
) -> Result<Value, String> {
    let request_id = stable_native_request_id(Some(native_request_id))?;
    let association = expected_association.ok_or("GOGOKE_NATIVE_ORIGINAL_ASSOCIATION_REQUIRED")?;
    crate::public_runtime::product_entry::ensure_design37_user_host(&app).await?;
    let reply = visible_operation(
        &app,
        &workspace_id,
        "visible-conversation-recover",
        &association,
        &request_id,
        None,
        None,
    )
    .await?;
    match reply.state.as_str() {
        "APPLIED" => reply
            .response
            .ok_or("GOGOKE_VISIBLE_RESPONSE_MISSING".into()),
        "UNKNOWN" | "DENIED" | "UNSUPPORTED" => Err(visible_failure(
            &reply.state,
            reply.reason.as_deref(),
            Some(&request_id),
        )),
        _ => Err("GOGOKE_VISIBLE_RECOVER_STATE_INVALID".into()),
    }
}

async fn native_session_active(state: &AppState, workspace_id: &str) -> bool {
    state
        .sessions
        .lock()
        .await
        .get(workspace_id)
        .is_some_and(|session| session.is_native())
}

async fn native_association(
    state: &AppState,
    workspace_id: &str,
) -> Result<NativeAssociation, String> {
    state
        .sessions
        .lock()
        .await
        .get(workspace_id)
        .and_then(|session| session.native_association())
        .ok_or("GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE".into())
}

pub(crate) async fn native_visible_live_state(
    app: &AppHandle,
    workspace_id: &str,
    association: &NativeAssociation,
) -> Result<bool, String> {
    let reply = visible_read(app, workspace_id, association, "live-state", None).await?;
    if reply.state != "APPLIED" {
        return Err(visible_failure(&reply.state, reply.reason.as_deref(), None));
    }
    reply.live.ok_or("GOGOKE_NATIVE_LIVE_STATE_MISSING".into())
}

pub(crate) async fn native_visible_stop(
    app: &AppHandle,
    workspace_id: &str,
    association: &NativeAssociation,
) -> Result<(), String> {
    let _ = (app, workspace_id, association);
    Err("GOGOKE_NATIVE_PHYSICAL_STOP_STABLE_INTENT_REQUIRED".into())
}

fn emit_thread_live_event(app: &AppHandle, workspace_id: &str, method: &str, params: Value) {
    let _ = app.emit(
        "app-server-event",
        AppServerEvent {
            workspace_id: workspace_id.to_string(),
            message: json!({
                "method": method,
                "params": params,
            }),
        },
    );
}

pub(crate) async fn spawn_workspace_session(
    entry: WorkspaceEntry,
    default_codex_bin: Option<String>,
    codex_args: Option<String>,
    app_handle: AppHandle,
    codex_home: Option<PathBuf>,
) -> Result<Arc<WorkspaceSession>, String> {
    crate::public_runtime::product_entry::ensure_design37_user_host(&app_handle).await?;
    let _ = VISIBLE_APP.set(app_handle.clone());
    let route = visible_route(&app_handle, &entry.id).await?;
    match route.state.as_str() {
        "NATIVE" => {
            return Ok(WorkspaceSession::new_native(
                &entry,
                app_handle,
                route
                    .association
                    .ok_or("GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE")?,
            ))
        }
        "LEGACY" => {}
        _ => return Err("GOGOKE_VISIBLE_ROUTE_STATE_INVALID".into()),
    }
    let client_version = env!("CARGO_PKG_VERSION").to_string();
    let event_sink = TauriEventSink::new(app_handle);
    spawn_workspace_session_inner(
        entry,
        default_codex_bin,
        codex_args,
        codex_home,
        client_version,
        event_sink,
    )
    .await
}

#[tauri::command]
pub(crate) async fn codex_doctor(
    codex_bin: Option<String>,
    codex_args: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    crate::shared::codex_aux_core::codex_doctor_core(&state.app_settings, codex_bin, codex_args)
        .await
}

#[tauri::command]
pub(crate) async fn codex_update(
    codex_bin: Option<String>,
    codex_args: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    crate::shared::codex_update_core::codex_update_core(&state.app_settings, codex_bin, codex_args)
        .await
}

#[tauri::command]
pub(crate) async fn start_thread(
    workspace_id: String,
    native_request_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "start_thread",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    if native_session_active(&state, &workspace_id).await {
        return native_visible_effect(
            &app,
            &state,
            &workspace_id,
            native_request_id,
            "thread/start",
            json!({}),
        )
        .await;
    }
    codex_core::start_thread_core(&state.sessions, &state.workspaces, workspace_id).await
}

#[tauri::command]
pub(crate) async fn resume_thread(
    workspace_id: String,
    thread_id: String,
    native_request_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "resume_thread",
            json!({ "workspaceId": workspace_id, "threadId": thread_id }),
        )
        .await;
    }

    if native_session_active(&state, &workspace_id).await {
        return native_visible_effect(
            &app,
            &state,
            &workspace_id,
            native_request_id,
            "thread/resume",
            json!({"threadId": thread_id}),
        )
        .await;
    }
    codex_core::resume_thread_core(&state.sessions, workspace_id, thread_id).await
}

#[tauri::command]
pub(crate) async fn read_thread(
    workspace_id: String,
    thread_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "read_thread",
            json!({ "workspaceId": workspace_id, "threadId": thread_id }),
        )
        .await;
    }

    if native_session_active(&state, &workspace_id).await {
        let association = native_association(&state, &workspace_id).await?;
        return native_visible_request(
            &app,
            &workspace_id,
            &association,
            "thread/read",
            json!({"threadId": thread_id, "includeTurns": true, "cursor": null}),
        )
        .await;
    }
    codex_core::read_thread_core(&state.sessions, workspace_id, thread_id).await
}

#[tauri::command]
pub(crate) async fn thread_live_subscribe(
    workspace_id: String,
    thread_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "thread_live_subscribe",
            json!({ "workspaceId": workspace_id, "threadId": thread_id }),
        )
        .await;
    }

    codex_core::thread_live_subscribe_core(
        &state.sessions,
        workspace_id.clone(),
        thread_id.clone(),
    )
    .await?;
    let subscription_id = format!("{}:{}", workspace_id, thread_id);
    emit_thread_live_event(
        &app,
        &workspace_id,
        "thread/live_attached",
        json!({
            "workspaceId": workspace_id,
            "threadId": thread_id,
            "subscriptionId": subscription_id,
        }),
    );
    Ok(json!({
        "subscriptionId": subscription_id,
        "state": "live",
    }))
}

#[tauri::command]
pub(crate) async fn thread_live_unsubscribe(
    workspace_id: String,
    thread_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "thread_live_unsubscribe",
            json!({ "workspaceId": workspace_id, "threadId": thread_id }),
        )
        .await;
    }

    codex_core::thread_live_unsubscribe_core(
        &state.sessions,
        workspace_id.clone(),
        thread_id.clone(),
    )
    .await?;
    emit_thread_live_event(
        &app,
        &workspace_id,
        "thread/live_detached",
        json!({
            "workspaceId": workspace_id,
            "threadId": thread_id,
            "reason": "manual",
        }),
    );
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub(crate) async fn fork_thread(
    workspace_id: String,
    thread_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "fork_thread",
            json!({ "workspaceId": workspace_id, "threadId": thread_id }),
        )
        .await;
    }

    codex_core::fork_thread_core(&state.sessions, workspace_id, thread_id).await
}

#[tauri::command]
pub(crate) async fn list_threads(
    workspace_id: String,
    cursor: Option<String>,
    limit: Option<u32>,
    sort_key: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "list_threads",
            json!({
                "workspaceId": workspace_id,
                "cursor": cursor,
                "limit": limit,
                "sortKey": sort_key
            }),
        )
        .await;
    }

    if native_session_active(&state, &workspace_id).await {
        if sort_key.is_some() {
            return Err("GOGOKE_NATIVE_SORT_KEY_UNSUPPORTED".into());
        }
        let association = native_association(&state, &workspace_id).await?;
        return native_visible_request(
            &app,
            &workspace_id,
            &association,
            "thread/list",
            json!({"cursor": cursor, "limit": limit}),
        )
        .await;
    }
    codex_core::list_threads_core(&state.sessions, workspace_id, cursor, limit, sort_key).await
}

#[tauri::command]
pub(crate) async fn list_mcp_server_status(
    workspace_id: String,
    cursor: Option<String>,
    limit: Option<u32>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "list_mcp_server_status",
            json!({ "workspaceId": workspace_id, "cursor": cursor, "limit": limit }),
        )
        .await;
    }

    codex_core::list_mcp_server_status_core(&state.sessions, workspace_id, cursor, limit).await
}

#[tauri::command]
pub(crate) async fn archive_thread(
    workspace_id: String,
    thread_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "archive_thread",
            json!({ "workspaceId": workspace_id, "threadId": thread_id }),
        )
        .await;
    }

    codex_core::archive_thread_core(&state.sessions, workspace_id, thread_id).await
}

#[tauri::command]
pub(crate) async fn compact_thread(
    workspace_id: String,
    thread_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "compact_thread",
            json!({ "workspaceId": workspace_id, "threadId": thread_id }),
        )
        .await;
    }

    codex_core::compact_thread_core(&state.sessions, workspace_id, thread_id).await
}

#[tauri::command]
pub(crate) async fn set_thread_name(
    workspace_id: String,
    thread_id: String,
    name: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "set_thread_name",
            json!({ "workspaceId": workspace_id, "threadId": thread_id, "name": name }),
        )
        .await;
    }

    codex_core::set_thread_name_core(&state.sessions, workspace_id, thread_id, name).await
}

#[tauri::command]
pub(crate) async fn send_user_message(
    workspace_id: String,
    thread_id: String,
    text: String,
    model: Option<String>,
    effort: Option<String>,
    service_tier: Option<Option<String>>,
    access_mode: Option<String>,
    images: Option<Vec<String>>,
    app_mentions: Option<Vec<Value>>,
    collaboration_mode: Option<Value>,
    native_request_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let images = images.map(|paths| {
            paths
                .into_iter()
                .map(remote_backend::normalize_path_for_remote)
                .collect::<Vec<_>>()
        });
        let mut payload = Map::new();
        payload.insert("workspaceId".to_string(), json!(workspace_id));
        payload.insert("threadId".to_string(), json!(thread_id));
        payload.insert("text".to_string(), json!(text));
        payload.insert("model".to_string(), json!(model));
        payload.insert("effort".to_string(), json!(effort));
        insert_optional_nullable_string(&mut payload, "serviceTier", service_tier);
        payload.insert("accessMode".to_string(), json!(access_mode));
        payload.insert("images".to_string(), json!(images));
        payload.insert("appMentions".to_string(), json!(app_mentions));
        if let Some(mode) = collaboration_mode {
            if !mode.is_null() {
                payload.insert("collaborationMode".to_string(), mode);
            }
        }
        return remote_backend::call_remote(
            &*state,
            app,
            "send_user_message",
            Value::Object(payload),
        )
        .await;
    }

    if native_session_active(&state, &workspace_id).await {
        if model.is_some()
            || effort.is_some()
            || service_tier.is_some()
            || access_mode.is_some()
            || images.is_some_and(|items| !items.is_empty())
            || app_mentions.is_some_and(|items| !items.is_empty())
            || collaboration_mode.is_some_and(|value| !value.is_null())
        {
            return Err("GOGOKE_NATIVE_CALLER_SELECTION_OR_ATTACHMENT_UNSUPPORTED".into());
        }
        if text.trim().is_empty() {
            return Err("GOGOKE_NATIVE_TEXT_REQUIRED".into());
        }
        return native_visible_effect(
            &app,
            &state,
            &workspace_id,
            native_request_id,
            "turn/start",
            json!({"threadId": thread_id, "input": [{"type":"text", "text": text}]}),
        )
        .await;
    }
    codex_core::send_user_message_core(
        &state.sessions,
        &state.workspaces,
        workspace_id,
        thread_id,
        text,
        model,
        effort,
        service_tier,
        access_mode,
        images,
        app_mentions,
        collaboration_mode,
    )
    .await
}

#[tauri::command]
pub(crate) async fn turn_steer(
    workspace_id: String,
    thread_id: String,
    turn_id: String,
    text: String,
    images: Option<Vec<String>>,
    app_mentions: Option<Vec<Value>>,
    native_request_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let images = images.map(|paths| {
            paths
                .into_iter()
                .map(remote_backend::normalize_path_for_remote)
                .collect::<Vec<_>>()
        });
        return remote_backend::call_remote(
            &*state,
            app,
            "turn_steer",
            json!({
                "workspaceId": workspace_id,
                "threadId": thread_id,
                "turnId": turn_id,
                "text": text,
                "images": images,
                "appMentions": app_mentions,
            }),
        )
        .await;
    }

    if native_session_active(&state, &workspace_id).await {
        if images.is_some_and(|items| !items.is_empty())
            || app_mentions.is_some_and(|items| !items.is_empty())
        {
            return Err("GOGOKE_NATIVE_ATTACHMENT_UNSUPPORTED".into());
        }
        if text.trim().is_empty() || turn_id.trim().is_empty() {
            return Err("GOGOKE_NATIVE_STEER_INPUT_INVALID".into());
        }
        return native_visible_effect(
            &app,
            &state,
            &workspace_id,
            native_request_id,
            "turn/steer",
            json!({"threadId": thread_id, "expectedTurnId": turn_id,
                "input": [{"type":"text", "text": text}]}),
        )
        .await;
    }
    codex_core::turn_steer_core(
        &state.sessions,
        workspace_id,
        thread_id,
        turn_id,
        text,
        images,
        app_mentions,
    )
    .await
}

#[tauri::command]
pub(crate) async fn collaboration_mode_list(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "collaboration_mode_list",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    codex_core::collaboration_mode_list_core(&state.sessions, workspace_id).await
}

#[tauri::command]
pub(crate) async fn turn_interrupt(
    workspace_id: String,
    thread_id: String,
    turn_id: String,
    native_request_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "turn_interrupt",
            json!({ "workspaceId": workspace_id, "threadId": thread_id, "turnId": turn_id }),
        )
        .await;
    }

    if native_session_active(&state, &workspace_id).await {
        return native_visible_effect(
            &app,
            &state,
            &workspace_id,
            native_request_id,
            "turn/interrupt",
            json!({"threadId": thread_id, "turnId": turn_id}),
        )
        .await;
    }
    codex_core::turn_interrupt_core(&state.sessions, workspace_id, thread_id, turn_id).await
}

#[tauri::command]
pub(crate) async fn start_review(
    workspace_id: String,
    thread_id: String,
    target: Value,
    delivery: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "start_review",
            json!({
                "workspaceId": workspace_id,
                "threadId": thread_id,
                "target": target,
                "delivery": delivery,
            }),
        )
        .await;
    }

    codex_core::start_review_core(&state.sessions, workspace_id, thread_id, target, delivery).await
}

#[tauri::command]
pub(crate) async fn model_list(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "model_list",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    codex_core::model_list_core(&state.sessions, workspace_id).await
}

#[tauri::command]
pub(crate) async fn experimental_feature_list(
    workspace_id: String,
    cursor: Option<String>,
    limit: Option<u32>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "experimental_feature_list",
            json!({
                "workspaceId": workspace_id,
                "cursor": cursor,
                "limit": limit
            }),
        )
        .await;
    }

    codex_core::experimental_feature_list_core(&state.sessions, workspace_id, cursor, limit).await
}

#[tauri::command]
pub(crate) async fn set_codex_feature_flag(
    feature_key: String,
    enabled: bool,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    if remote_backend::is_remote_mode(&*state).await {
        remote_backend::call_remote(
            &*state,
            app,
            "set_codex_feature_flag",
            json!({
                "featureKey": feature_key,
                "enabled": enabled
            }),
        )
        .await?;
        return Ok(());
    }

    config::write_feature_enabled(feature_key.as_str(), enabled)
}

#[tauri::command]
pub(crate) async fn get_agents_settings(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<agents_config_core::AgentsSettingsDto, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response =
            remote_backend::call_remote(&*state, app, "get_agents_settings", json!({})).await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    agents_config_core::get_agents_settings_core()
}

#[tauri::command]
pub(crate) async fn set_agents_core_settings(
    input: agents_config_core::SetAgentsCoreInput,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<agents_config_core::AgentsSettingsDto, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response = remote_backend::call_remote(
            &*state,
            app,
            "set_agents_core_settings",
            json!({ "input": input }),
        )
        .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    agents_config_core::set_agents_core_settings_core(input)
}

#[tauri::command]
pub(crate) async fn create_agent(
    input: agents_config_core::CreateAgentInput,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<agents_config_core::AgentsSettingsDto, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response =
            remote_backend::call_remote(&*state, app, "create_agent", json!({ "input": input }))
                .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    agents_config_core::create_agent_core(input)
}

#[tauri::command]
pub(crate) async fn update_agent(
    input: agents_config_core::UpdateAgentInput,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<agents_config_core::AgentsSettingsDto, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response =
            remote_backend::call_remote(&*state, app, "update_agent", json!({ "input": input }))
                .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    agents_config_core::update_agent_core(input)
}

#[tauri::command]
pub(crate) async fn delete_agent(
    input: agents_config_core::DeleteAgentInput,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<agents_config_core::AgentsSettingsDto, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response =
            remote_backend::call_remote(&*state, app, "delete_agent", json!({ "input": input }))
                .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    agents_config_core::delete_agent_core(input)
}

#[tauri::command]
pub(crate) async fn read_agent_config_toml(
    agent_name: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response = remote_backend::call_remote(
            &*state,
            app,
            "read_agent_config_toml",
            json!({ "agentName": agent_name }),
        )
        .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    agents_config_core::read_agent_config_toml_core(agent_name.as_str())
}

#[tauri::command]
pub(crate) async fn write_agent_config_toml(
    agent_name: String,
    content: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    if remote_backend::is_remote_mode(&*state).await {
        remote_backend::call_remote(
            &*state,
            app,
            "write_agent_config_toml",
            json!({
                "agentName": agent_name,
                "content": content,
            }),
        )
        .await?;
        return Ok(());
    }

    agents_config_core::write_agent_config_toml_core(agent_name.as_str(), content.as_str())
}

#[tauri::command]
pub(crate) async fn account_rate_limits(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "account_rate_limits",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    codex_core::account_rate_limits_core(&state.sessions, workspace_id).await
}

#[tauri::command]
pub(crate) async fn account_read(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "account_read",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    codex_core::account_read_core(&state.sessions, &state.workspaces, workspace_id).await
}

#[tauri::command]
pub(crate) async fn codex_login(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "codex_login",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    codex_core::codex_login_core(&state.sessions, &state.codex_login_cancels, workspace_id).await
}

#[tauri::command]
pub(crate) async fn codex_login_cancel(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "codex_login_cancel",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    codex_core::codex_login_cancel_core(&state.sessions, &state.codex_login_cancels, workspace_id)
        .await
}

#[tauri::command]
pub(crate) async fn skills_list(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "skills_list",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    codex_core::skills_list_core(&state.sessions, &state.workspaces, workspace_id).await
}

#[tauri::command]
pub(crate) async fn apps_list(
    workspace_id: String,
    cursor: Option<String>,
    limit: Option<u32>,
    thread_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "apps_list",
            json!({
                "workspaceId": workspace_id,
                "cursor": cursor,
                "limit": limit,
                "threadId": thread_id
            }),
        )
        .await;
    }

    codex_core::apps_list_core(&state.sessions, workspace_id, cursor, limit, thread_id).await
}

#[tauri::command]
pub(crate) async fn respond_to_server_request(
    workspace_id: String,
    request_id: Value,
    result: Value,
    native_request_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    if remote_backend::is_remote_mode(&*state).await {
        remote_backend::call_remote(
            &*state,
            app,
            "respond_to_server_request",
            json!({ "workspaceId": workspace_id, "requestId": request_id, "result": result }),
        )
        .await?;
        return Ok(());
    }

    if native_session_active(&state, &workspace_id).await {
        native_visible_effect(
            &app,
            &state,
            &workspace_id,
            native_request_id,
            "original-question-answer",
            json!({"requestId": request_id, "result": result}),
        )
        .await?;
        return Ok(());
    }
    codex_core::respond_to_server_request_core(&state.sessions, workspace_id, request_id, result)
        .await
}

#[tauri::command]
pub(crate) async fn remember_approval_rule(
    workspace_id: String,
    command: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    codex_core::remember_approval_rule_core(&state.workspaces, workspace_id, command).await
}

#[tauri::command]
pub(crate) async fn get_config_model(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "get_config_model",
            json!({ "workspaceId": workspace_id }),
        )
        .await;
    }

    codex_core::get_config_model_core(&state.workspaces, workspace_id).await
}

/// Generates a commit message in the background without showing in the main chat
#[tauri::command]
pub(crate) async fn generate_commit_message(
    workspace_id: String,
    commit_message_model_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let value = remote_backend::call_remote(
            &*state,
            app,
            "generate_commit_message",
            json!({
                "workspaceId": workspace_id,
                "commitMessageModelId": commit_message_model_id,
            }),
        )
        .await?;
        return serde_json::from_value(value).map_err(|err| err.to_string());
    }

    let diff = crate::git::get_workspace_diff(&workspace_id, &state).await?;

    let commit_message_prompt = {
        let settings = state.app_settings.lock().await;
        settings.commit_message_prompt.clone()
    };
    crate::shared::codex_aux_core::generate_commit_message_core(
        &state.sessions,
        &state.workspaces,
        workspace_id,
        &diff,
        &commit_message_prompt,
        commit_message_model_id.as_deref(),
        |workspace_id, thread_id| {
            let _ = app.emit(
                "app-server-event",
                AppServerEvent {
                    workspace_id: workspace_id.to_string(),
                    message: json!({
                        "method": "codex/backgroundThread",
                        "params": {
                            "threadId": thread_id,
                            "action": "hide"
                        }
                    }),
                },
            );
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn generate_run_metadata(
    workspace_id: String,
    prompt: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return remote_backend::call_remote(
            &*state,
            app,
            "generate_run_metadata",
            json!({ "workspaceId": workspace_id, "prompt": prompt }),
        )
        .await;
    }

    crate::shared::codex_aux_core::generate_run_metadata_core(
        &state.sessions,
        &state.workspaces,
        workspace_id,
        &prompt,
        |workspace_id, thread_id| {
            let _ = app.emit(
                "app-server-event",
                AppServerEvent {
                    workspace_id: workspace_id.to_string(),
                    message: json!({
                        "method": "codex/backgroundThread",
                        "params": {
                            "threadId": thread_id,
                            "action": "hide"
                        }
                    }),
                },
            );
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn generate_agent_description(
    workspace_id: String,
    description: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<crate::shared::codex_aux_core::GeneratedAgentConfiguration, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let value = remote_backend::call_remote(
            &*state,
            app,
            "generate_agent_description",
            json!({ "workspaceId": workspace_id, "description": description }),
        )
        .await?;
        return serde_json::from_value(value).map_err(|err| err.to_string());
    }

    crate::shared::codex_aux_core::generate_agent_description_core(
        &state.sessions,
        &state.workspaces,
        workspace_id,
        &description,
        |workspace_id, thread_id| {
            let _ = app.emit(
                "app-server-event",
                AppServerEvent {
                    workspace_id: workspace_id.to_string(),
                    message: json!({
                        "method": "codex/backgroundThread",
                        "params": {
                            "threadId": thread_id,
                            "action": "hide"
                        }
                    }),
                },
            );
        },
    )
    .await
}

#[cfg(test)]
mod native_visible_boundary_tests {
    use super::*;

    #[test]
    fn caller_cannot_choose_native_runtime_fields_or_expand_methods() {
        assert!(native_visible_params("thread/start", &json!({"cwd":"."})).is_err());
        assert!(native_visible_params(
            "turn/start",
            &json!({
                "threadId":"threadA", "input":[{"type":"text","text":"hello"}],
                "approvalPolicy":"never"
            })
        )
        .is_err());
        assert!(native_visible_params("account/read", &json!({})).is_err());
        assert!(native_visible_params(
            "turn/start",
            &json!({
                "threadId":"threadA", "input":[{"type":"image","url":"file:///x"}]
            })
        )
        .is_err());
    }

    #[test]
    fn effect_requires_caller_stable_intent_id() {
        assert!(stable_native_request_id(None).is_err());
        assert!(stable_native_request_id(Some("../bad".into())).is_err());
        assert_eq!(
            stable_native_request_id(Some("intent_1".into())).unwrap(),
            "intent_1"
        );
    }
}
