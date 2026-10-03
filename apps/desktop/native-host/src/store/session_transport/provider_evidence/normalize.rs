//! Pure projection of captured provider frames into ACP-shaped output data.
//!
//! A must persist the exact OriginBoundFrame before calling either entry point.
//! H supplies the observed provider and its already bound native session. None
//! of these values authorize a process, settle delivery, or prove StopFact.

use super::{acp, stream_json};
use crate::store::atomic::{Json, JsonString, Parser};
use crate::store::session_transport::codex_output::NormalizedUpdate;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Provider { Claude, OpenCode, GrokBuild, Antigravity }

impl Provider {
    fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::GrokBuild => "grok-build",
            Self::Antigravity => "antigravity",
        }
    }
}

/// A candidate from the provider's own turn response, not H's delivery or
/// process-stop receipt. `reason` retains its source spelling.
pub(crate) enum Output {
    Update(NormalizedUpdate),
    Terminal { reason: String, reported_error: Option<bool> },
    PermissionData { request_id: acp::RpcId, params_json: String },
    SessionData { method: &'static str, value_json: String },
    RemoteError { error_json: String, raw_frame: Vec<u8> },
    Unhandled { method: String, raw_frame: Vec<u8> },
    Unsupported,
}

pub(crate) struct NormalizeError {
    pub(crate) reason: String,
    pub(crate) raw_frame: Vec<u8>,
}

fn invalid(frame: &[u8], reason: impl Into<String>) -> NormalizeError {
    NormalizeError { reason: reason.into(), raw_frame: frame.to_vec() }
}
fn key(name: &str) -> JsonString { JsonString::from_str(name) }
fn text(value: &str) -> Json { Json::String(key(value)) }
fn field<'a>(fields: &'a BTreeMap<JsonString, Json>, name: &str) -> Option<&'a Json> {
    fields.get(&key(name))
}
fn object<'a>(value: &'a Json, frame: &[u8], name: &str)
    -> Result<&'a BTreeMap<JsonString, Json>, NormalizeError> {
    if let Json::Object(fields) = value { Ok(fields) }
    else { Err(invalid(frame, format!("{name} must be an object"))) }
}
fn required_text(fields: &BTreeMap<JsonString, Json>, frame: &[u8], name: &str)
    -> Result<String, NormalizeError> {
    let Some(Json::String(value)) = field(fields, name) else {
        return Err(invalid(frame, format!("{name} must be text")));
    };
    value.to_well_formed_string()
        .filter(|value| !value.is_empty() && !value.contains('\0'))
        .ok_or_else(|| invalid(frame, format!("{name} is empty or invalid")))
}
fn content_text(fields: &BTreeMap<JsonString, Json>, frame: &[u8], name: &str)
    -> Result<String, NormalizeError> {
    let Some(Json::String(value)) = field(fields, name) else {
        return Err(invalid(frame, format!("{name} must be text")));
    };
    value.to_well_formed_string().filter(|value| !value.contains('\0'))
        .ok_or_else(|| invalid(frame, format!("{name} is invalid")))
}
fn copy_json(value: &Json) -> Json {
    match value {
        Json::Null => Json::Null,
        Json::Bool(value) => Json::Bool(*value),
        Json::Number(value) => Json::Number(value.clone()),
        Json::String(value) => Json::String(value.clone()),
        Json::Array(values) => Json::Array(values.iter().map(copy_json).collect()),
        Json::Object(fields) => Json::Object(fields.iter().map(|(name, value)|
            (name.clone(), copy_json(value))).collect()),
    }
}
fn nonnegative_integer(value: &Json, frame: &[u8], name: &str) -> Result<Json, NormalizeError> {
    let Json::Number(number) = value else {
        return Err(invalid(frame, format!("{name} must be a nonnegative integer")));
    };
    number.parse::<u64>()
        .map_err(|_| invalid(frame, format!("{name} must be a nonnegative integer")))?;
    Ok(Json::Number(number.clone()))
}
fn acp_tool_kind(value: &str) -> bool {
    matches!(value, "read" | "edit" | "delete" | "move" | "search" |
        "execute" | "think" | "fetch" | "switch_mode" | "other")
}
fn acp_tool_status(value: &str) -> bool {
    matches!(value, "pending" | "in_progress" | "completed" | "failed")
}
fn acp_tool_update(frame: &[u8], provider: Provider, session: &str, thread: &str,
    kind: &str, fields: &BTreeMap<JsonString, Json>) -> Result<Output, NormalizeError> {
    let tool_id = required_text(fields, frame, "toolCallId")?;
    let mut extra = vec![("toolCallId", text(&tool_id))];
    if kind == "tool_call" {
        extra.push(("title", text(&required_text(fields, frame, "title")?)));
    } else if let Some(value) = field(fields, "title") {
        if matches!(value, Json::Null) { extra.push(("title", Json::Null)); }
        else { extra.push(("title", text(&content_text(fields, frame, "title")?))); }
    }
    if let Some(value) = field(fields, "kind") {
        if kind == "tool_call_update" && matches!(value, Json::Null) {
            extra.push(("kind", Json::Null));
        } else {
            let found = required_text(fields, frame, "kind")?;
            if !acp_tool_kind(&found) { return Err(invalid(frame, "unknown ACP tool kind")); }
            extra.push(("kind", text(&found)));
        }
    }
    if let Some(value) = field(fields, "status") {
        if kind == "tool_call_update" && matches!(value, Json::Null) {
            extra.push(("status", Json::Null));
        } else {
            let found = required_text(fields, frame, "status")?;
            if !acp_tool_status(&found) { return Err(invalid(frame, "unknown ACP tool status")); }
            extra.push(("status", text(&found)));
        }
    }
    for name in ["rawInput", "rawOutput"] {
        if let Some(value) = field(fields, name) {
            extra.push((if name == "rawInput" { "rawInput" } else { "rawOutput" }, copy_json(value)));
        }
    }
    if let Some(value) = field(fields, "content") {
        match value {
            Json::Null if kind == "tool_call_update" => extra.push(("content", Json::Null)),
            Json::Array(items) => {
                for item in items {
                    let content = object(item, frame, "ACP tool content")?;
                    let content_type = required_text(content, frame, "type")?;
                    match content_type.as_str() {
                        "content" => {
                            let nested = object(field(content, "content")
                                .ok_or_else(|| invalid(frame, "ACP tool content.content missing"))?, frame, "ACP content block")?;
                            let block_type = required_text(nested, frame, "type")?;
                            let required: &[&str] = match block_type.as_str() {
                                "text" => &["text"],
                                "image" | "audio" => &["data", "mimeType"],
                                "resource_link" => &["name", "uri"],
                                "resource" => {
                                    let resource = object(field(nested, "resource")
                                        .ok_or_else(|| invalid(frame, "ACP embedded resource missing"))?, frame, "ACP embedded resource")?;
                                    content_text(resource, frame, "uri")?;
                                    if field(resource, "text").is_some() { content_text(resource, frame, "text")?; }
                                    else { content_text(resource, frame, "blob")?; }
                                    &[]
                                }
                                _ => return Ok(Output::Unhandled { method: format!("session/update/{kind}/content/{block_type}"), raw_frame: frame.to_vec() }),
                            };
                            for name in required { content_text(nested, frame, name)?; }
                        }
                        "diff" => {
                            content_text(content, frame, "path")?;
                            content_text(content, frame, "newText")?;
                            if let Some(old) = field(content, "oldText") {
                                if !matches!(old, Json::Null) { content_text(content, frame, "oldText")?; }
                            }
                        }
                        // Vendor display data: this ID grants no native access.
                        "terminal" => { content_text(content, frame, "terminalId")?; }
                        _ => return Ok(Output::Unhandled { method: format!("session/update/{kind}/content/{content_type}"), raw_frame: frame.to_vec() }),
                    }
                }
                extra.push(("content", copy_json(value)));
            }
            _ => return Err(invalid(frame, "ACP tool content must be an array")),
        }
    }
    // This is the provider's tool observation, not H's execution or grant proof.
    Ok(Output::Update(update(provider, "session/update", session, thread, kind, extra)))
}
fn acp_usage_update(frame: &[u8], provider: Provider, session: &str, thread: &str,
    fields: &BTreeMap<JsonString, Json>) -> Result<Output, NormalizeError> {
    let size = nonnegative_integer(field(fields, "size")
        .ok_or_else(|| invalid(frame, "usage_update.size missing"))?, frame, "usage_update.size")?;
    let used = nonnegative_integer(field(fields, "used")
        .ok_or_else(|| invalid(frame, "usage_update.used missing"))?, frame, "usage_update.used")?;
    // ACP `used` is context occupancy, not total billable/session tokens.
    Ok(Output::Update(update(provider, "session/update", session, thread,
        "usage_update", [("size", size), ("used", used)])))
}
fn check_binding(frame: &[u8], found: &str, expected: &str) -> Result<(), NormalizeError> {
    if expected.is_empty() || expected.contains('\0') || found != expected {
        return Err(invalid(frame, "native session does not match H binding"));
    }
    Ok(())
}
fn update(provider: Provider, method: &str, session: &str, thread: &str, kind: &str,
    extra: impl IntoIterator<Item = (&'static str, Json)>) -> NormalizedUpdate {
    let meta = BTreeMap::from([
        (key("provider"), text(provider.label())),
        (key("nativeSessionId"), text(session)),
        (key("threadId"), text(thread)),
        (key("providerMethod"), text(method)),
    ]);
    // Metadata is rebuilt from H's binding. Vendor _meta is never copied.
    let mut fields = BTreeMap::from([
        (key("sessionUpdate"), text(kind)),
        (key("_meta"), Json::Object(meta)),
    ]);
    fields.extend(extra.into_iter().map(|(name, value)| (key(name), value)));
    NormalizedUpdate { method: method.to_owned(), update_json: Json::Object(fields).canonical() }
}
fn parse_object(frame: &[u8]) -> Result<Json, NormalizeError> {
    let body = frame.strip_suffix(b"\n").unwrap_or(frame);
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    let value = std::str::from_utf8(body).map_err(|e| invalid(frame, format!("UTF-8: {e}")))?;
    Parser::parse(value).map_err(|e| invalid(frame, format!("JSON: {e:?}")))
}

/// The caller must supply a session ID observed from this same captured CLI,
/// never one copied from the untrusted frame being converted.
pub(crate) fn claude(frame: &[u8], bound_session: &str, bound_thread: &str)
    -> Result<Output, NormalizeError> {
    if bound_thread.is_empty() || bound_thread.contains('\0') {
        return Err(invalid(frame, "H thread binding missing"));
    }
    let decoded = stream_json::decode_claude_line(frame)
        .map_err(|e| invalid(frame, format!("Claude stream-json: {e:?}")))?;
    match decoded {
        stream_json::ClaudeData::Init { session_id, model } => {
            check_binding(frame, &session_id, bound_session)?;
            let value = Json::Object(BTreeMap::from([
                (key("sessionId"), text(&session_id)),
                (key("model"), model.as_deref().map(text).unwrap_or(Json::Null)),
            ]));
            Ok(Output::SessionData { method: "system/init", value_json: value.canonical() })
        }
        stream_json::ClaudeData::Result { session_id, subtype, is_error } => {
            check_binding(frame, &session_id, bound_session)?;
            Ok(Output::Terminal { reason: subtype, reported_error: Some(is_error) })
        }
        stream_json::ClaudeData::ControlRequest { request_id, subtype } =>
            Ok(Output::Unhandled { method: format!("control_request/{:?}/{:?}", subtype, request_id),
                raw_frame: frame.to_vec() }),
        stream_json::ClaudeData::Unhandled { frame_type: Some(kind) } if kind == "assistant" => {
            let parsed = parse_object(frame)?;
            let root = object(&parsed, frame, "assistant")?;
            if let Some(Json::String(id)) = field(root, "session_id") {
                let id = id.to_well_formed_string()
                    .ok_or_else(|| invalid(frame, "assistant.session_id invalid"))?;
                check_binding(frame, &id, bound_session)?;
            }
            let message = match field(root, "message") {
                Some(value) => object(value, frame, "assistant.message")?,
                None => return Ok(Output::Unhandled { method: kind, raw_frame: frame.to_vec() }),
            };
            let Some(Json::Array(content)) = field(message, "content") else {
                return Ok(Output::Unhandled { method: kind, raw_frame: frame.to_vec() });
            };
            let mut chunks = Vec::new();
            for block in content {
                let block = object(block, frame, "assistant content block")?;
                if field(block, "type").and_then(|v| if let Json::String(v) = v {
                    v.to_well_formed_string()
                } else { None }).as_deref() == Some("text") {
                    chunks.push(content_text(block, frame, "text")?);
                } else {
                    // One frame can contain text and tool blocks. Do not emit
                    // its text while silently discarding an unqualified tool.
                    return Ok(Output::Unhandled { method: kind, raw_frame: frame.to_vec() });
                }
            }
            if chunks.is_empty() {
                return Ok(Output::Unhandled { method: kind, raw_frame: frame.to_vec() });
            }
            let content = Json::Object(BTreeMap::from([
                (key("type"), text("text")), (key("text"), text(&chunks.concat())),
            ]));
            Ok(Output::Update(update(Provider::Claude, "assistant", bound_session, bound_thread,
                "agent_message_chunk", [("content", content)])))
        }
        stream_json::ClaudeData::Unhandled { frame_type } => Ok(Output::Unhandled {
            method: frame_type.unwrap_or_else(|| "unknown".to_owned()), raw_frame: frame.to_vec(),
        }),
    }
}

/// `pending` is the exact H stdin journal entry. The native ACP parser checks
/// response ID and method before this projection can see a response.
pub(crate) fn acp(frame: &[u8], provider: Provider, bound_session: &str, bound_thread: &str,
    pending: Option<&acp::Pending<'_>>) -> Result<Output, NormalizeError> {
    if provider == Provider::Antigravity { return Ok(Output::Unsupported); }
    if !matches!(provider, Provider::OpenCode | Provider::GrokBuild) {
        return Err(invalid(frame, "provider is not ACP"));
    }
    if bound_session.is_empty() || bound_session.contains('\0') ||
        bound_thread.is_empty() || bound_thread.contains('\0') {
        return Err(invalid(frame, "H native session or thread binding missing"));
    }
    if let Some(pending) = pending {
        if matches!(pending.method, acp::PendingMethod::SessionLoad | acp::PendingMethod::SessionResume) {
            let requested = pending.requested_session_id
                .ok_or_else(|| invalid(frame, "H requested session binding missing"))?;
            check_binding(frame, requested, bound_session)?;
        }
    }
    let observation = acp::decode(frame, pending)
        .map_err(|e| NormalizeError { reason: e.reason, raw_frame: e.raw_frame })?;
    match observation {
        acp::Observation::SessionUpdate { session_id, update: vendor } => {
            check_binding(frame, &session_id, bound_session)?;
            let fields = object(&vendor, frame, "session/update.update")?;
            let kind = required_text(fields, frame, "sessionUpdate")?;
            if matches!(kind.as_str(), "tool_call" | "tool_call_update") {
                return acp_tool_update(frame, provider, bound_session, bound_thread, &kind, fields);
            }
            if kind == "usage_update" {
                return acp_usage_update(frame, provider, bound_session, bound_thread, fields);
            }
            // Other update types stay as exact source rows in A.
            if !matches!(kind.as_str(), "agent_message_chunk" | "agent_thought_chunk") {
                return Ok(Output::Unhandled { method: format!("session/update/{kind}"), raw_frame: frame.to_vec() });
            }
            let content = object(field(fields, "content")
                .ok_or_else(|| invalid(frame, "text update content missing"))?, frame, "text content")?;
            if required_text(content, frame, "type")? != "text" {
                return Ok(Output::Unhandled { method: format!("session/update/{kind}/content"), raw_frame: frame.to_vec() });
            }
            let text_value = content_text(content, frame, "text")?;
            let clean_content = Json::Object(BTreeMap::from([
                (key("type"), text("text")), (key("text"), text(&text_value)),
            ]));
            Ok(Output::Update(update(provider, "session/update", bound_session, bound_thread,
                &kind, [("content", clean_content)])))
        }
        acp::Observation::Prompt { stop_reason, .. } => {
            let reason = match stop_reason {
                acp::StopReason::EndTurn => "end_turn", acp::StopReason::MaxTokens => "max_tokens",
                acp::StopReason::MaxTurnRequests => "max_turn_requests",
                acp::StopReason::Refusal => "refusal", acp::StopReason::Cancelled => "cancelled",
                acp::StopReason::Error => "error",
            };
            Ok(Output::Terminal { reason: reason.to_owned(), reported_error: None })
        }
        acp::Observation::PermissionRequest { id, params } => {
            let params_fields = object(&params, frame, "permission params")?;
            let session = required_text(params_fields, frame, "sessionId")?;
            check_binding(frame, &session, bound_session)?;
            Ok(Output::PermissionData { request_id: id, params_json: params.canonical() })
        }
        acp::Observation::RemoteError { error, raw_frame, .. } =>
            Ok(Output::RemoteError { error_json: error.canonical(), raw_frame }),
        acp::Observation::SessionNew { session_id, result, .. } => {
            check_binding(frame, &session_id, bound_session)?;
            Ok(Output::SessionData { method: "session/new", value_json: result.canonical() })
        }
        acp::Observation::SessionLoad { echoed_session_id, result, .. } => {
            if let Some(id) = echoed_session_id { check_binding(frame, &id, bound_session)?; }
            Ok(Output::SessionData { method: "session/load", value_json: result.canonical() })
        }
        acp::Observation::SessionResume { echoed_session_id, result, .. } => {
            if let Some(id) = echoed_session_id { check_binding(frame, &id, bound_session)?; }
            Ok(Output::SessionData { method: "session/resume", value_json: result.canonical() })
        }
        acp::Observation::SessionConfigOption { result, .. } =>
            Ok(Output::SessionData { method: "session/set_config_option", value_json: result.canonical() }),
        acp::Observation::Initialize { result, .. } =>
            Ok(Output::SessionData { method: "initialize", value_json: result.canonical() }),
        acp::Observation::Unhandled { raw_frame } => {
            let method = parse_object(frame).ok()
                .and_then(|value| if let Json::Object(fields) = value {
                    if let Some(Json::String(name)) = field(&fields, "method") {
                        name.to_well_formed_string()
                    } else { None }
                } else { None })
                .unwrap_or_else(|| "unknown ACP response".to_owned());
            Ok(Output::Unhandled { method, raw_frame })
        },
    }
}
