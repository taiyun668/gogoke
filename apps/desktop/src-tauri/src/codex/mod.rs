use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

use tauri::{AppHandle, Emitter, Manager, State};

pub(crate) mod args;
pub(crate) mod config;
pub(crate) mod home;

use crate::backend::app_server::{NativeAssociation, NativeEventPosition};
pub(crate) use crate::backend::app_server::WorkspaceSession;
use crate::backend::events::AppServerEvent;
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
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    original_request_ref: Option<OriginalRequestRef>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OriginalRequestRef {
    frame_sha256: String,
    selection_row_id: String,
}

fn valid_original_request_ref(original: &OriginalRequestRef) -> bool {
    original.frame_sha256.len() == 64
        && original
            .frame_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        && canonical_binding_generation(&original.selection_row_id).is_ok_and(|row_id| row_id > 0)
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
    live: Option<VisibleLiveReply>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VisibleLiveReply {
    state: String,
    #[serde(default)]
    pending_questions: Vec<Value>,
}

async fn visible_user_frame(app: &AppHandle, frame: Value) -> Result<String, String> {
    crate::public_runtime::product_entry::gogoke_design37_user_operation(
        app.clone(),
        frame.to_string(),
    )
    .await
}

async fn read_visible_route(app: &AppHandle, workspace_id: &str) -> Result<VisibleRouteReply, String> {
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
    Ok(reply)
}

async fn visible_route(app: &AppHandle, workspace_id: &str) -> Result<VisibleRouteReply, String> {
    let reply = read_visible_route(app, workspace_id).await?;
    match reply.state.as_str() {
        "LEGACY" => Err("GOGOKE_VISIBLE_LEGACY_MODEL_ROUTE_UNSUPPORTED".into()),
        "NATIVE" if reply.association.is_some() => Ok(reply),
        "NATIVE" => Err("GOGOKE_VISIBLE_ROUTE_ASSOCIATION_INVALID".into()),
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
) -> Result<Option<NativeAssociation>, String> {
    let Some(app) = VISIBLE_APP.get() else {
        return Ok(None);
    };
    let route = visible_route(app, workspace_id).await?;
    route.association.map(Some).ok_or("GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE".into())
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
    if command == "visible-conversation-operate"
        && matches!(reply.state.as_str(), "APPLIED" | "UNKNOWN")
    {
        if method.is_some_and(|method| reply.method.as_deref() != Some(method)) {
            return Err("GOGOKE_VISIBLE_ORIGINAL_METHOD_MISMATCH".into());
        }
        if !reply
            .original_request_ref
            .as_ref()
            .is_some_and(valid_original_request_ref)
        {
            return Err("GOGOKE_VISIBLE_ORIGINAL_REQUEST_REF_INVALID".into());
        }
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

fn native_event_text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value.get(field).and_then(Value::as_str).filter(|text| !text.is_empty())
        .ok_or_else(|| format!("GOGOKE_NATIVE_EVENT_FIELD_INVALID:{field}"))
}

/// Validate the complete page before any notification reaches the UI. The
/// original envelope stays intact; this cursor never authorizes a model write.
fn native_event_page(reply: &VisibleReadReply, association: &NativeAssociation,
    position: &NativeEventPosition) -> Result<(NativeEventPosition, Vec<Value>), String> {
    if !matches!(reply.state.as_str(), "PARTIAL" | "APPLIED") {
        return Err(visible_failure(&reply.state, reply.reason.as_deref(), None));
    }
    let result = reply.response.as_ref().and_then(|response| response.get("result"))
        .ok_or("GOGOKE_NATIVE_EVENT_RESULT_MISSING")?;
    let marker = result.get("nativeEvents").ok_or("GOGOKE_NATIVE_EVENT_MARKER_MISSING")?;
    let expected_state = if reply.state == "PARTIAL" { "PARTIAL" } else { "COMPLETE" };
    if marker.get("state").and_then(Value::as_str) != Some(expected_state) {
        return Err("GOGOKE_NATIVE_EVENT_STATE_MISMATCH".into());
    }
    let high = native_event_text(marker, "highWater")?;
    let high_number = canonical_binding_generation(high)?;
    let after = canonical_binding_generation(native_event_text(marker, "afterSourceId")?)?;
    if after > high_number || after < position.last_source
        || position.page_high.as_deref().is_some_and(|previous| previous != high) {
        return Err("GOGOKE_NATIVE_EVENT_WATERMARK_MISMATCH".into());
    }
    let notifications = result.get("notifications").and_then(Value::as_array)
        .ok_or("GOGOKE_NATIVE_EVENT_NOTIFICATIONS_INVALID")?;
    let refs = marker.get("sourceRefs").and_then(Value::as_array)
        .ok_or("GOGOKE_NATIVE_EVENT_REFS_INVALID")?;
    if notifications.len() != refs.len() { return Err("GOGOKE_NATIVE_EVENT_REFS_COUNT_MISMATCH".into()); }
    let mut next_position = position.clone();
    let mut original = Vec::new();
    for (item, source) in notifications.iter().zip(refs) {
        if item.get("sourceRef") != Some(source) || source.as_object().is_none_or(|fields| fields.len() != 5) {
            return Err("GOGOKE_NATIVE_EVENT_REF_MISMATCH".into());
        }
        let raw_id = canonical_binding_generation(native_event_text(source, "rawSourceId")?)?;
        let ordinal = canonical_binding_generation(native_event_text(source, "sourceCursor")?)?;
        let operation = native_event_text(source, "operationId")?;
        let epoch = native_event_text(source, "sourceEpoch")?;
        if native_event_text(source, "generation")? != association.binding_generation
            || raw_id <= after || raw_id <= next_position.last_source || raw_id > high_number
            || ordinal == 0 || ordinal <= next_position.last_ordinal
            || next_position.operation.as_deref().is_some_and(|value| value != operation)
            || next_position.epoch.as_deref().is_some_and(|value| value != epoch) {
            return Err("GOGOKE_NATIVE_EVENT_SOURCE_CHANGED_OR_REPEATED".into());
        }
        let frame = item.get("notification").ok_or("GOGOKE_NATIVE_EVENT_NOTIFICATION_MISSING")?;
        let method = native_event_text(frame, "method")?;
        if frame.get("id").is_some() || frame.get("result").is_some() || frame.get("error").is_some() {
            return Err("GOGOKE_NATIVE_EVENT_IS_NOT_NOTIFICATION".into());
        }
        let params = frame.get("params").ok_or("GOGOKE_NATIVE_EVENT_PARAMS_MISSING")?;
        let thread = if method == "thread/started" {
            native_event_text(params.get("thread").ok_or("GOGOKE_NATIVE_EVENT_THREAD_MISSING")?, "id")?
        } else { native_event_text(params, "threadId")? };
        if next_position.thread.as_deref().is_some_and(|value| value != thread) {
            return Err("GOGOKE_NATIVE_EVENT_THREAD_CHANGED".into());
        }
        next_position.last_source = raw_id;
        next_position.last_ordinal = ordinal;
        next_position.operation = Some(operation.to_owned());
        next_position.epoch = Some(epoch.to_owned());
        next_position.thread = Some(thread.to_owned());
        original.push(item.clone());
    }
    let next = marker.get("nextCursor").ok_or("GOGOKE_NATIVE_EVENT_NEXT_CURSOR_MISSING")?;
    let resume = marker.get("resumeCursor").ok_or("GOGOKE_NATIVE_EVENT_RESUME_CURSOR_MISSING")?;
    if reply.state == "PARTIAL" {
        let cursor = next.as_str().filter(|value| !value.is_empty())
            .ok_or("GOGOKE_NATIVE_EVENT_PARTIAL_CURSOR_INVALID")?;
        if !resume.is_null() || !next_position.seen_pages.insert(cursor.to_owned()) {
            return Err("GOGOKE_NATIVE_EVENT_PARTIAL_CURSOR_REPEATED".into());
        }
        next_position.params = json!({"cursor": cursor});
        next_position.page_high = Some(high.to_owned());
    } else {
        let cursor = resume.as_str().filter(|value| !value.is_empty())
            .ok_or("GOGOKE_NATIVE_EVENT_COMPLETE_CURSOR_INVALID")?;
        if !next.is_null() { return Err("GOGOKE_NATIVE_EVENT_COMPLETE_HAS_PAGE_CURSOR".into()); }
        next_position.params = json!({"resumeCursor": cursor});
        next_position.page_high = None;
        next_position.seen_pages.clear();
    }
    Ok((next_position, original))
}

pub(crate) async fn start_native_visible_events(session: &Arc<WorkspaceSession>) -> Result<(), String> {
    let Some((app, reader_id, retained_position)) = session.begin_native_events()? else { return Ok(()); };
    // Establish the attachment watermark before the observing call resolves. Existing
    // content is restored by the full original thread/read path, never by
    // replaying historical text deltas into an already rendered conversation.
    let seeded: Result<(NativeAssociation, NativeEventPosition), String> = async {
        let association = session.native_association()?.ok_or("GOGOKE_NATIVE_EVENT_ASSOCIATION_MISSING")?;
        if let Some(position) = retained_position { return Ok((association, position)); }
        let mut position = NativeEventPosition::default();
        loop {
            let params = if position.params.is_null() { json!({}) } else { position.params.clone() };
            let reply = visible_read(&app, &session.owner_workspace_id, &association,
                "native-events", Some(params)).await?;
            let app_state = app.state::<AppState>();
            let sessions = app_state.sessions.lock().await;
            if !sessions.get(&session.owner_workspace_id).is_some_and(|value| Arc::ptr_eq(value, session))
                || !session.native_event_reader_current(reader_id, &association)? {
                return Err("GOGOKE_NATIVE_EVENT_ATTACHMENT_CHANGED_DURING_READ".into());
            }
            position = native_event_page(&reply, &association, &position)?.0;
            if reply.state == "APPLIED" {
                if !session.commit_native_events(reader_id, &association, position.clone(), || Ok(()))? {
                    return Err("GOGOKE_NATIVE_EVENT_ATTACHMENT_CHANGED_BEFORE_COMMIT".into());
                }
                return Ok((association, position));
            }
        }
    }.await;
    let (seed_association, seed_position) = match seeded {
        Ok(seed) => seed,
        Err(error) => {
            if let Err(finish) = session.finish_native_events(reader_id, Some(error.clone())) {
                return Err(format!("{error}; reader state: {finish}"));
            }
            return Err(error);
        }
    };
    let weak = Arc::downgrade(session);
    tauri::async_runtime::spawn(async move {
        let association = seed_association;
        let mut position = seed_position;
        let result: Result<(), String> = async {
            loop {
                let Some(session) = weak.upgrade() else { return Ok(()); };
                let workspace = &session.owner_workspace_id;
                let current = &association;
                if !session.native_event_reader_current(reader_id, current)? { return Ok(()); }
                let params = if position.params.is_null() { json!({}) } else { position.params.clone() };
                let reply = visible_read(&app, workspace, &current, "native-events", Some(params)).await;
                // Both cache replacement and resume can happen while the USER
                // read is in flight. Neither permits emitting its old result.
                let app_state = app.state::<AppState>();
                let sessions = app_state.sessions.lock().await;
                if !sessions.get(workspace).is_some_and(|value| Arc::ptr_eq(value, &session)) { return Ok(()); }
                if !session.native_event_reader_current(reader_id, current)? { return Ok(()); }
                let reply = reply?;
                let (next_position, notifications) = native_event_page(&reply, &current, &position)?;
                let delivered = session.commit_native_events(reader_id, current, next_position.clone(), || {
                    for item in &notifications {
                        app.emit("app-server-event", json!({
                            "workspace_id": workspace, "message": item["notification"],
                            "nativeAssociation": current, "nativeSourceRef": item["sourceRef"],
                        })).map_err(|error| format!("GOGOKE_NATIVE_EVENT_DELIVERY_FAILED:{error}"))?;
                    }
                    Ok(())
                })?;
                drop(sessions);
                if !delivered { return Ok(()); }
                position = next_position;
                if reply.state == "APPLIED" {
                    // Observation cadence only; never a timeout, stop proof,
                    // retry policy or source-completeness inference.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }.await;
        if let Some(session) = weak.upgrade() {
            if let Err(error) = &result { eprintln!("GOGOKE_NATIVE_EVENT_READER_FAILED:{error}"); }
            if let Err(error) = session.finish_native_events(reader_id, result.err()) {
                eprintln!("GOGOKE_NATIVE_EVENT_READER_FINISH_FAILED:{error}");
            }
        }
    });
    Ok(())
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

fn canonical_binding_generation(value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|error| format!("GOGOKE_NATIVE_BINDING_GENERATION_INVALID:{error}"))?;
    if parsed.to_string() != value {
        return Err("GOGOKE_NATIVE_BINDING_GENERATION_NONCANONICAL".into());
    }
    Ok(parsed)
}

fn validated_resume_association(
    response: &Value,
    old: &NativeAssociation,
    requested_thread_id: &str,
) -> Result<NativeAssociation, String> {
    let rpc_id = response
        .get("id")
        .ok_or("GOGOKE_NATIVE_RESUME_RPC_ID_MISSING")?;
    if !matches!(rpc_id, Value::String(id) if !id.is_empty())
        && !matches!(rpc_id, Value::Number(id) if id.is_i64() || id.is_u64())
    {
        return Err("GOGOKE_NATIVE_RESUME_RPC_ID_INVALID".into());
    }
    let result = response
        .get("result")
        .and_then(Value::as_object)
        .ok_or("GOGOKE_NATIVE_RESUME_RESULT_MISSING")?;
    if result
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        != Some(requested_thread_id)
    {
        return Err("GOGOKE_NATIVE_RESUME_THREAD_ID_MISMATCH".into());
    }
    let next: NativeAssociation = serde_json::from_value(
        result
            .get("nativeAssociation")
            .cloned()
            .ok_or("GOGOKE_NATIVE_RESUME_ASSOCIATION_MISSING")?,
    )
    .map_err(|error| format!("GOGOKE_NATIVE_RESUME_ASSOCIATION_INVALID:{error}"))?;
    let old_generation = canonical_binding_generation(&old.binding_generation)?;
    let next_generation = canonical_binding_generation(&next.binding_generation)?;
    if old_generation.checked_add(1) != Some(next_generation) {
        return Err("GOGOKE_NATIVE_RESUME_GENERATION_NOT_NEXT".into());
    }
    let mut same_generation = next.clone();
    same_generation.binding_generation = old.binding_generation.clone();
    if &same_generation != old {
        return Err("GOGOKE_NATIVE_RESUME_ASSOCIATION_CHANGED".into());
    }
    let sources = result
        .get("sourceRefs")
        .and_then(Value::as_array)
        .ok_or("GOGOKE_NATIVE_RESUME_SOURCES_MISSING")?;
    if sources.len() != 2 {
        return Err("GOGOKE_NATIVE_RESUME_SOURCES_INCOMPLETE".into());
    }
    let mut kinds = HashSet::new();
    let mut operation_id: Option<&str> = None;
    for source in sources {
        let fields = source
            .as_object()
            .ok_or("GOGOKE_NATIVE_RESUME_SOURCE_INVALID")?;
        let kind = fields
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("GOGOKE_NATIVE_RESUME_SOURCE_KIND_MISSING")?;
        if !matches!(kind, "RESUME_ACK" | "THREAD_STARTED") || !kinds.insert(kind) {
            return Err("GOGOKE_NATIVE_RESUME_SOURCE_KIND_INVALID".into());
        }
        let operation = fields
            .get("operationId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or("GOGOKE_NATIVE_RESUME_OPERATION_MISSING")?;
        if let Some(first) = operation_id {
            if first != operation {
                return Err("GOGOKE_NATIVE_RESUME_OPERATION_MISMATCH".into());
            }
        } else {
            operation_id = Some(operation);
        }
        if fields.get("generation").and_then(Value::as_str)
            != Some(next.binding_generation.as_str())
            || ["sourceEpoch", "sourceCursor"].iter().any(|key| {
                fields
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
            })
        {
            return Err("GOGOKE_NATIVE_RESUME_SOURCE_GENERATION_OR_CURSOR_INVALID".into());
        }
    }
    if !kinds.contains("RESUME_ACK") || !kinds.contains("THREAD_STARTED") {
        return Err("GOGOKE_NATIVE_RESUME_SOURCES_INCOMPLETE".into());
    }
    Ok(next)
}

async fn native_visible_effect(
    app: &AppHandle,
    state: &AppState,
    workspace_id: &str,
    native_request_id: Option<String>,
    expected_association: Option<NativeAssociation>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let request_id = stable_native_request_id(native_request_id)?;
    let before_dispatch = |error: String| visible_not_dispatched(&request_id, error);
    native_visible_params(method, &params).map_err(&before_dispatch)?;
    let requested_thread_id = if method == "thread/resume" {
        Some(
            params
                .get("threadId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| before_dispatch("GOGOKE_NATIVE_RESUME_THREAD_ID_REQUIRED".into()))?
                .to_owned(),
        )
    } else {
        None
    };
    let session = state
        .sessions
        .lock()
        .await
        .get(workspace_id)
        .cloned()
        .ok_or_else(|| before_dispatch("GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE".into()))?;
    if session.owner_workspace_id != workspace_id {
        return Err(before_dispatch("GOGOKE_NATIVE_WORKSPACE_OWNER_MISMATCH".into()));
    }
    let association = session
        .native_association().map_err(&before_dispatch)?
        .ok_or_else(|| before_dispatch("GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE".into()))?;
    require_original_association(expected_association.as_ref(), &association).map_err(&before_dispatch)?;
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
        "APPLIED" => {
            let response = reply.response.ok_or("GOGOKE_VISIBLE_RESPONSE_MISSING")?;
            if let Some(requested_thread_id) = requested_thread_id {
                let next =
                    validated_resume_association(&response, &association, &requested_thread_id)?;
                let sessions = state.sessions.lock().await;
                if !sessions
                    .get(workspace_id)
                    .is_some_and(|current| Arc::ptr_eq(&session, current))
                {
                    return Err("GOGOKE_NATIVE_SESSION_REPLACED_BEFORE_RESUME_COMMIT".into());
                }
                session.advance_native_association(&association, next)?;
                drop(sessions);
                // The original resume remains APPLIED even if its read-only
                // observer fails; that original read reason is retained apart.
                if let Err(error) = start_native_visible_events(&session).await {
                    eprintln!("GOGOKE_NATIVE_RESUME_EVENT_READER_FAILED:{error}");
                }
            }
            Ok(response)
        }
        "UNKNOWN" | "DENIED" | "UNSUPPORTED" => Err(visible_failure(
            &reply.state,
            reply.reason.as_deref(),
            Some(&request_id),
        )),
        _ => Err("GOGOKE_VISIBLE_OPERATION_STATE_INVALID".into()),
    }
}

// Emitted only before visible_operation is called; an IPC failure after the
// dispatch boundary must retain UNKNOWN and cannot use this marker.
fn visible_not_dispatched(request_id: &str, error: String) -> String {
    format!("GOGOKE_VISIBLE_NOT_DISPATCHED:{request_id}:{error}")
}

fn caller_not_dispatched(request_id: Option<&str>, error: &str) -> String {
    match stable_native_request_id(request_id.map(str::to_owned)) {
        Ok(request_id) => visible_not_dispatched(&request_id, error.into()),
        Err(_) => error.into(),
    }
}

fn require_original_association(
    expected: Option<&NativeAssociation>,
    current: &NativeAssociation,
) -> Result<(), String> {
    match expected {
        Some(expected) if expected == current => Ok(()),
        Some(_) => Err("GOGOKE_NATIVE_CALLER_ASSOCIATION_CHANGED".into()),
        None => Err("GOGOKE_NATIVE_ORIGINAL_ASSOCIATION_REQUIRED".into()),
    }
}

pub(crate) async fn native_visible_stop_with_intent(
    app: &AppHandle,
    state: &AppState,
    workspace_id: &str,
    native_request_id: Option<String>,
    expected_association: Option<NativeAssociation>,
) -> Result<Value, String> {
    let request_id = stable_native_request_id(native_request_id)?;
    let before_dispatch = |error: String| visible_not_dispatched(&request_id, error);
    let session = state
        .sessions
        .lock()
        .await
        .get(workspace_id)
        .cloned()
        .ok_or_else(|| before_dispatch("GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE".into()))?;
    if session.owner_workspace_id != workspace_id {
        return Err(before_dispatch("GOGOKE_NATIVE_WORKSPACE_OWNER_MISMATCH".into()));
    }
    let association = session
        .native_association().map_err(&before_dispatch)?
        .ok_or_else(|| before_dispatch("GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE".into()))?;
    require_original_association(expected_association.as_ref(), &association)
        .map_err(&before_dispatch)?;
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
        let sessions = state.sessions.lock().await;
        if !sessions
            .get(workspace_id)
            .is_some_and(|current| Arc::ptr_eq(&session, current))
        {
            return Err("GOGOKE_NATIVE_SESSION_REPLACED_BEFORE_STOP_COMMIT".into());
        }
        let fact = reply.stop_fact.ok_or("GOGOKE_NATIVE_STOP_FACT_MISSING")?;
        let response = reply.response.ok_or("GOGOKE_NATIVE_STOP_RESPONSE_MISSING")?;
        if response.get("result").and_then(|value| value.get("stopFact"))
            .and_then(Value::as_str) != Some(fact.as_str())
        {
            return Err("GOGOKE_NATIVE_STOP_RESPONSE_FACT_MISMATCH".into());
        }
        session.note_native_stop_fact(&association, fact)?;
        Ok(response)
    } else {
        Err(visible_failure(
            &reply.state,
            reply.reason.as_deref(),
            Some(&request_id),
        ))
    }
}

/// The caller persists this original intent before dispatch. Cleanup may use
/// only the resulting physical H proof, never an interrupt acknowledgement.
#[tauri::command]
pub(crate) async fn stop_native_visible_session(
    workspace_id: String,
    native_request_id: Option<String>,
    expected_association: Option<NativeAssociation>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return Err(caller_not_dispatched(native_request_id.as_deref(),
            "GOGOKE_NATIVE_PHYSICAL_STOP_REMOTE_UNSUPPORTED"));
    }
    native_visible_stop_with_intent(&app, &state, &workspace_id,
        native_request_id, expected_association).await
}

#[tauri::command]
pub(crate) async fn recover_native_visible_request(
    workspace_id: String,
    native_request_id: String,
    expected_association: Option<NativeAssociation>,
    state: State<'_, AppState>,
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
        "APPLIED" => {
            let response = reply.response.ok_or("GOGOKE_VISIBLE_RESPONSE_MISSING")?;
            if reply.method.as_deref() == Some("physical-stop") {
                let fact = reply.stop_fact.as_deref()
                    .filter(|fact| !fact.is_empty())
                    .ok_or("GOGOKE_NATIVE_STOP_FACT_MISSING")?;
                if response.get("result").and_then(|value| value.get("stopFact"))
                    .and_then(Value::as_str) != Some(fact)
                {
                    return Err("GOGOKE_NATIVE_STOP_RESPONSE_FACT_MISMATCH".into());
                }
                let sessions = state.sessions.lock().await;
                if let Some(session) = sessions.get(&workspace_id) {
                    // Original recovery is independent of a current attachment.
                    // A replaced attachment must never receive the old proof.
                    if session.owner_workspace_id == workspace_id {
                        match session.native_association() {
                            Ok(Some(cached)) if cached == association => {
                                if let Err(error) = session.note_native_stop_fact(&association, fact.to_owned()) {
                                    eprintln!("GOGOKE_NATIVE_RECOVER_STOP_CACHE_SYNC_FAILED:{error}");
                                }
                            }
                            Err(error) => eprintln!("GOGOKE_NATIVE_RECOVER_STOP_CACHE_READ_FAILED:{error}"),
                            _ => {}
                        }
                    }
                }
            }
            if reply.method.as_deref() == Some("thread/resume")
                && reply
                    .original_request_ref
                    .as_ref()
                    .is_some_and(valid_original_request_ref)
            {
                let Some(thread_id) = response
                    .get("result")
                    .and_then(|result| result.get("thread"))
                    .and_then(|thread| thread.get("id"))
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                else {
                    return Ok(response);
                };
                let Ok(next) = validated_resume_association(&response, &association, thread_id)
                else {
                    return Ok(response);
                };
                // Recovery is authorized by the original journal. This read only
                // decides whether an existing UI cache still describes its current choice.
                let current_route_matches =
                    visible_route(&app, &workspace_id).await.is_ok_and(|route| {
                        route.state == "NATIVE" && route.association.as_ref() == Some(&next)
                    });
                if current_route_matches {
                    let sessions = state.sessions.lock().await;
                    let mut reader_session = None;
                    if let Some(session) = sessions.get(&workspace_id) {
                        if session.owner_workspace_id == workspace_id {
                            match session.native_association() {
                                Ok(Some(cached)) if cached == association => {
                                    match session.advance_native_association(&association, next) {
                                        Ok(()) => reader_session = Some(session.clone()),
                                        Err(error) => eprintln!("GOGOKE_NATIVE_RECOVER_CACHE_SYNC_FAILED:{error}"),
                                    }
                                }
                                Err(error) => {
                                    eprintln!("GOGOKE_NATIVE_RECOVER_CACHE_READ_FAILED:{error}")
                                }
                                _ => {}
                            }
                        }
                    }
                    drop(sessions);
                    if let Some(session) = reader_session {
                        if let Err(error) = start_native_visible_events(&session).await {
                            eprintln!("GOGOKE_NATIVE_RECOVER_EVENT_READER_FAILED:{error}");
                        }
                    }
                }
            }
            Ok(response)
        }
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

pub(crate) async fn reject_native_workspace(
    app: &AppHandle,
    workspace_id: &str,
    surface: &str,
) -> Result<(), String> {
    crate::public_runtime::product_entry::ensure_design37_user_host(app).await?;
    if native_session_active(app.state::<AppState>().inner(), workspace_id).await {
        return Err(format!("GOGOKE_NATIVE_{surface}_UNSUPPORTED"));
    }
    let reply = read_visible_route(app, workspace_id).await?;
    match reply.state.as_str() {
        // Compatibility file/configuration operations do not grant permission
        // to start a model. Every Gogoke model route remains native-only.
        "NEEDS_SETUP" | "LEGACY" if reply.association.is_none() => Ok(()),
        "NATIVE" => Err(format!("GOGOKE_NATIVE_{surface}_UNSUPPORTED")),
        "UNKNOWN" => Err(visible_failure("UNKNOWN", reply.reason.as_deref(), None)),
        _ => Err("GOGOKE_VISIBLE_ROUTE_ASSOCIATION_INVALID".into()),
    }
}

async fn native_association(
    state: &AppState,
    workspace_id: &str,
) -> Result<NativeAssociation, String> {
    let session = state
        .sessions
        .lock()
        .await
        .get(workspace_id)
        .cloned()
        .ok_or_else(|| "GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE".to_string())?;
    session
        .native_association()?
        .ok_or("GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE".into())
}

/// Read the actual transport attachment, never infer it from workspace labels
/// or use an unavailable native route as permission to call the legacy writer.
#[tauri::command]
pub(crate) async fn native_visible_transport(
    workspace_id: String,
    observe: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    if remote_backend::is_remote_mode(&*state).await {
        return Ok(json!({"schema": VISIBLE_SCHEMA, "workspaceId": workspace_id,
            "state": "REMOTE", "association": null}));
    }
    let session = state.sessions.lock().await.get(&workspace_id).cloned();
    let mut event_error = None;
    let (kind, association) = match session {
        Some(session) if session.is_native() => {
            if session.owner_workspace_id != workspace_id {
                return Err("GOGOKE_NATIVE_WORKSPACE_OWNER_MISMATCH".into());
            }
            if observe == Some(true) {
                if let Err(error) = start_native_visible_events(&session).await {
                    event_error = Some(error);
                }
            }
            event_error = event_error.or(session.native_event_error()?);
            if !state.sessions.lock().await.get(&workspace_id).is_some_and(|current| Arc::ptr_eq(current, &session)) {
                return Err("GOGOKE_NATIVE_SESSION_REPLACED_DURING_TRANSPORT_READ".into());
            }
            let association = session.native_association()?
                .ok_or("GOGOKE_NATIVE_ASSOCIATION_UNAVAILABLE")?;
            ("NATIVE", Some(association))
        }
        Some(_) => ("LEGACY", None),
        None => ("DISCONNECTED", None),
    };
    Ok(json!({"schema": VISIBLE_SCHEMA, "workspaceId": workspace_id,
        "state": kind, "association": association, "nativeEventReadError": event_error}))
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
    let live = reply.live.ok_or("GOGOKE_NATIVE_LIVE_STATE_MISSING")?;
    visible_live_observation(&live, reply.reason.as_deref())
}

fn visible_live_observation(live: &VisibleLiveReply, reason: Option<&str>) -> Result<bool, String> {
    // The producer binds LIVE to the exact retained physical H holder, and
    // STOPPED to the original complete H proof. A read receipt alone proves
    // neither; UNKNOWN keeps the original observation error.
    match live.state.as_str() {
        "LIVE" => Ok(true),
        "STOPPED" => Ok(false),
        "UNKNOWN" => Err(format!("{}:pendingQuestions={}",
            visible_failure("UNKNOWN", reason, None), live.pending_questions.len())),
        _ => Err("GOGOKE_NATIVE_LIVE_STATE_INVALID".into()),
    }
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
    _default_codex_bin: Option<String>,
    _codex_args: Option<String>,
    app_handle: AppHandle,
    _codex_home: Option<PathBuf>,
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
        _ => Err("GOGOKE_VISIBLE_MODEL_ROUTE_UNSUPPORTED".into()),
    }
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
    expected_association: Option<NativeAssociation>,
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
            expected_association,
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
    expected_association: Option<NativeAssociation>,
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
            expected_association,
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
    reject_native_workspace(&app, &workspace_id, "LIVE_SUBSCRIPTION").await?;
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
    reject_native_workspace(&app, &workspace_id, "LIVE_SUBSCRIPTION").await?;
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
    expected_association: Option<NativeAssociation>,
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
            return Err(caller_not_dispatched(native_request_id.as_deref(),
                "GOGOKE_NATIVE_CALLER_SELECTION_OR_ATTACHMENT_UNSUPPORTED"));
        }
        if text.trim().is_empty() {
            return Err(caller_not_dispatched(native_request_id.as_deref(), "GOGOKE_NATIVE_TEXT_REQUIRED"));
        }
        return native_visible_effect(
            &app,
            &state,
            &workspace_id,
            native_request_id,
            expected_association,
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
    expected_association: Option<NativeAssociation>,
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
            return Err(caller_not_dispatched(native_request_id.as_deref(), "GOGOKE_NATIVE_ATTACHMENT_UNSUPPORTED"));
        }
        if text.trim().is_empty() || turn_id.trim().is_empty() {
            return Err(caller_not_dispatched(native_request_id.as_deref(), "GOGOKE_NATIVE_STEER_INPUT_INVALID"));
        }
        return native_visible_effect(
            &app,
            &state,
            &workspace_id,
            native_request_id,
            expected_association,
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
    expected_association: Option<NativeAssociation>,
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
            expected_association,
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
    reject_native_workspace(&app, &workspace_id, "LEGACY_ACCOUNT_READ").await?;
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
    reject_native_workspace(&app, &workspace_id, "LEGACY_LOGIN").await?;
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
    reject_native_workspace(&app, &workspace_id, "LEGACY_LOGIN").await?;
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
    expected_association: Option<NativeAssociation>,
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
            expected_association,
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
    app: AppHandle,
) -> Result<Value, String> {
    reject_native_workspace(&app, &workspace_id, "LEGACY_APPROVAL_RULE").await?;
    codex_core::remember_approval_rule_core(&state.workspaces, workspace_id, command).await
}

#[tauri::command]
pub(crate) async fn get_config_model(
    workspace_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Value, String> {
    reject_native_workspace(&app, &workspace_id, "LEGACY_CONFIG_READ").await?;
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
    reject_native_workspace(&app, &workspace_id, "AUX_MODEL").await?;
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
    reject_native_workspace(&app, &workspace_id, "AUX_MODEL").await?;
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
    reject_native_workspace(&app, &workspace_id, "AUX_MODEL").await?;
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

    fn association(generation: &str) -> NativeAssociation {
        NativeAssociation {
            domain_id: "domainA".into(),
            session_id: "sessionA".into(),
            seat_id: "seatA".into(),
            incarnation: "incA".into(),
            authorization_generation: "4".into(),
            binding_generation: generation.into(),
            instance_id: "instanceA".into(),
        }
    }

    fn event_reply() -> VisibleReadReply {
        let source = json!({"rawSourceId":"5","operationId":"opA","generation":"2",
            "sourceEpoch":"nonceA","sourceCursor":"3"});
        VisibleReadReply { schema: VISIBLE_SCHEMA.into(), workspace_id: "workspaceA".into(),
            association: Some(association("2")), state: "APPLIED".into(), live: None, reason: None,
            response: Some(json!({"result":{
                "notifications":[{"notification":{"method":"item/agentMessage/delta",
                    "params":{"threadId":"threadA","turnId":"turnA","itemId":"itemA","delta":"hello"}},
                    "sourceRef":source}],
                "nativeEvents":{"state":"COMPLETE","highWater":"7","afterSourceId":"0",
                    "nextCursor":null,"resumeCursor":"resume:7:original","sourceRefs":[source]}
            }})) }
    }

    fn native_notifications_preserve_original_envelope_and_scope() {
        let reply = event_reply();
        let (position, events) = native_event_page(&reply, &association("2"), &NativeEventPosition::default()).unwrap();
        assert_eq!(events[0]["notification"]["params"]["delta"], "hello");
        assert_eq!(position.params, json!({"resumeCursor":"resume:7:original"}));
        assert!(native_event_page(&reply, &association("3"), &NativeEventPosition::default()).is_err());
        assert!(native_event_page(&reply, &association("2"), &position).is_err());
        let mut wrong_ref = event_reply();
        wrong_ref.response.as_mut().unwrap()["result"]["notifications"][0]["sourceRef"]["operationId"] = json!("other");
        assert!(native_event_page(&wrong_ref, &association("2"), &NativeEventPosition::default()).is_err());
        let mut server_request = event_reply();
        server_request.response.as_mut().unwrap()["result"]["notifications"][0]["notification"]["id"] = json!(11);
        assert!(native_event_page(&server_request, &association("2"), &NativeEventPosition::default()).is_err());
        let prior_thread = NativeEventPosition { thread: Some("otherThread".into()), ..NativeEventPosition::default() };
        assert!(native_event_page(&reply, &association("2"), &prior_thread).is_err());
    }

    fn native_event_partial_page_cannot_change_water_or_repeat_cursor() {
        let mut reply = event_reply();
        reply.state = "PARTIAL".into();
        let marker = &mut reply.response.as_mut().unwrap()["result"]["nativeEvents"];
        marker["state"] = json!("PARTIAL");
        marker["nextCursor"] = json!("page:7:5:original");
        marker["resumeCursor"] = Value::Null;
        let (position, _) = native_event_page(&reply, &association("2"), &NativeEventPosition::default()).unwrap();
        let mut next = event_reply();
        let result = &mut next.response.as_mut().unwrap()["result"];
        result["notifications"] = json!([]);
        result["nativeEvents"]["sourceRefs"] = json!([]);
        result["nativeEvents"]["afterSourceId"] = json!("5");
        result["nativeEvents"]["highWater"] = json!("8");
        assert!(native_event_page(&next, &association("2"), &position).is_err());
        let mut repeated = reply;
        let result = &mut repeated.response.as_mut().unwrap()["result"];
        result["notifications"] = json!([]);
        result["nativeEvents"]["sourceRefs"] = json!([]);
        result["nativeEvents"]["afterSourceId"] = json!("5");
        assert!(native_event_page(&repeated, &association("2"), &position).is_err());
    }

    #[test]
    fn resume_cache_requires_original_two_sources_and_actual_next_generation() {
        let old = association("2");
        let mut reply = json!({"id":7,"result":{
            "thread":{"id":"threadA"},"nativeAssociation":association("3"),
            "sourceRefs":[
                {"kind":"RESUME_ACK","operationId":"opA","generation":"3",
                    "sourceEpoch":"epochA","sourceCursor":"cursor1"},
                {"kind":"THREAD_STARTED","operationId":"opA","generation":"3",
                    "sourceEpoch":"epochA","sourceCursor":"cursor2"}
            ]
        }});
        assert_eq!(
            validated_resume_association(&reply, &old, "threadA").unwrap(),
            association("3")
        );
        reply["result"]["sourceRefs"][1]["operationId"] = json!("opB");
        assert!(validated_resume_association(&reply, &old, "threadA").is_err());
        reply["result"]["sourceRefs"][1]["operationId"] = json!("opA");
        reply["result"]["nativeAssociation"]["bindingGeneration"] = json!("4");
        assert!(validated_resume_association(&reply, &old, "threadA").is_err());
        reply["result"]["nativeAssociation"]["bindingGeneration"] = json!("3");
        reply["result"]["thread"]["id"] = json!("other");
        assert!(validated_resume_association(&reply, &old, "threadA").is_err());
    }

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

    #[test]
    fn effect_requires_original_caller_association_before_dispatch() {
        native_notifications_preserve_original_envelope_and_scope();
        native_event_partial_page_cannot_change_water_or_repeat_cursor();
        let current = association("2");
        assert!(require_original_association(None, &current).is_err());
        assert!(require_original_association(Some(&current), &current).is_ok());
        for field in 0..7 {
            let mut changed = current.clone();
            match field {
                0 => changed.domain_id.push('x'),
                1 => changed.session_id.push('x'),
                2 => changed.seat_id.push('x'),
                3 => changed.incarnation.push('x'),
                4 => changed.authorization_generation.push('x'),
                5 => changed.binding_generation.push('x'),
                _ => changed.instance_id.push('x'),
            }
            assert!(require_original_association(Some(&changed), &current).is_err());
        }
    }

    #[test]
    fn unknown_live_read_keeps_original_reason_without_alive_or_stop_claim() {
        let mut live = VisibleLiveReply { state: "UNKNOWN".into(), pending_questions: vec![] };
        let error = visible_live_observation(&live, Some("original H handle absent")).unwrap_err();
        assert!(error.contains("original H handle absent"));
        live.state = "LIVE".into();
        assert_eq!(visible_live_observation(&live, None).unwrap(), true);
        live.state = "STOPPED".into();
        assert_eq!(visible_live_observation(&live, None).unwrap(), false);
        live.state = "APPLIED".into();
        assert!(visible_live_observation(&live, None).is_err());
    }
}
