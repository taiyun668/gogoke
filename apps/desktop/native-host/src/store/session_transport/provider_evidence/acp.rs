//! ACP v1 JSONL field extraction for pinned OpenCode 1.18.32 and Grok Build 1.0.41.
//!
//! This is data from one already captured stdout line. The caller must persist
//! the original OriginBoundFrame in A before decoding and supply the pending
//! native request ID and method from its own stdin journal. A decoded value
//! cannot establish process identity, permissions, delivery, or a stopped Job.
//!
//! References: archived gogo-party `packages/seat-runtime` Grok ACP path;
//! `docs/research/2026-09-26-execution-layer-capability-table.md`;
//! `third_party/t3code/apps/server/src/gogoke/adapters/{grok,opencode}`;
//! ACP v1 schema and OpenCode v1.18.32 `packages/opencode/src/acp/service.ts`.

use crate::store::atomic::{Json, JsonString, Parser};
use std::collections::BTreeMap;

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_SAFE_ID: i64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RpcId {
    Number(i64),
    String(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingMethod {
    Initialize,
    SessionNew,
    SessionLoad,
    SessionResume,
    SessionSetConfigOption,
    SessionPrompt,
}

/// Created from the exact native stdin request, not from a caller's later label.
pub(crate) struct Pending<'a> {
    pub(crate) id: &'a RpcId,
    pub(crate) method: PendingMethod,
    /// Required for session/load or session/resume. It is the original ID.
    pub(crate) requested_session_id: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Error,
}

/// All Json payloads below are vendor data. The initialize capabilities are
/// only declarations, including when a live no-login initialize was observed.
pub(crate) enum Observation {
    Initialize {
        id: RpcId,
        declared_capabilities: Json,
        result: Json,
    },
    SessionNew {
        id: RpcId,
        session_id: String,
        result: Json,
    },
    SessionLoad {
        id: RpcId,
        /// None is a valid ACK shape; no ID is fabricated from the request.
        echoed_session_id: Option<String>,
        result: Json,
    },
    SessionResume {
        id: RpcId,
        /// OpenCode 1.18.32 can ACK without echoing the requested ID.
        echoed_session_id: Option<String>,
        result: Json,
    },
    SessionConfigOption {
        id: RpcId,
        config_options: Json,
        result: Json,
    },
    Prompt {
        id: RpcId,
        stop_reason: StopReason,
        result: Json,
    },
    SessionUpdate {
        session_id: String,
        update: Json,
    },
    PermissionRequest {
        id: RpcId,
        params: Json,
    },
    RemoteError {
        id: RpcId,
        error: Json,
        /// Preserves the complete original response, including vendor fields.
        raw_frame: Vec<u8>,
    },
    /// Unknown but well-formed JSON-RPC data stays visible to the caller.
    Unhandled { raw_frame: Vec<u8> },
}

pub(crate) struct DecodeError {
    pub(crate) reason: String,
    pub(crate) raw_frame: Vec<u8>,
}

fn invalid(frame: &[u8], reason: impl Into<String>) -> DecodeError {
    DecodeError { reason: reason.into(), raw_frame: frame.to_vec() }
}

fn key(name: &str) -> JsonString { JsonString::from_str(name) }

fn copy_json(value: &Json) -> Json {
    match value {
        Json::Null => Json::Null,
        Json::Bool(value) => Json::Bool(*value),
        Json::Number(value) => Json::Number(value.clone()),
        Json::String(value) => Json::String(JsonString::from_units(value.units().to_vec())),
        Json::Array(values) => Json::Array(values.iter().map(copy_json).collect()),
        Json::Object(fields) => Json::Object(fields.iter().map(|(name, value)|
            (JsonString::from_units(name.units().to_vec()), copy_json(value))).collect()),
    }
}

fn field<'a>(fields: &'a BTreeMap<JsonString, Json>, name: &str) -> Option<&'a Json> {
    fields.get(&key(name))
}

fn object<'a>(value: &'a Json, frame: &[u8], name: &str)
    -> Result<&'a BTreeMap<JsonString, Json>, DecodeError> {
    if let Json::Object(fields) = value { Ok(fields) }
    else { Err(invalid(frame, format!("{name} must be an object"))) }
}

fn text(value: &Json, frame: &[u8], name: &str) -> Result<String, DecodeError> {
    let Json::String(value) = value else { return Err(invalid(frame, format!("{name} must be text"))); };
    let found = value.to_well_formed_string()
        .ok_or_else(|| invalid(frame, format!("{name} contains an invalid surrogate")))?;
    if found.is_empty() || found.contains('\0') {
        return Err(invalid(frame, format!("{name} is empty or contains NUL")));
    }
    Ok(found)
}

fn rpc_id(value: &Json, frame: &[u8]) -> Result<RpcId, DecodeError> {
    match value {
        Json::String(_) => Ok(RpcId::String(text(value, frame, "JSON-RPC id")?)),
        Json::Number(number) => {
            let parsed = number.parse::<i64>()
                .map_err(|error| invalid(frame, format!("JSON-RPC id must be an integer: {error}")))?;
            if !(-MAX_SAFE_ID..=MAX_SAFE_ID).contains(&parsed) {
                return Err(invalid(frame, "JSON-RPC id exceeds the safe integer range"));
            }
            Ok(RpcId::Number(parsed))
        }
        _ => Err(invalid(frame, "JSON-RPC id must be a number or string")),
    }
}

fn stop_reason(value: &Json, frame: &[u8]) -> Result<StopReason, DecodeError> {
    match text(value, frame, "session/prompt.stopReason")?.as_str() {
        "end_turn" => Ok(StopReason::EndTurn),
        "max_tokens" => Ok(StopReason::MaxTokens),
        "max_turn_requests" => Ok(StopReason::MaxTurnRequests),
        "refusal" => Ok(StopReason::Refusal),
        "cancelled" => Ok(StopReason::Cancelled),
        "error" => Ok(StopReason::Error),
        _ => Err(invalid(frame, "unrecognized session/prompt.stopReason")),
    }
}

/// Decode one bounded ACP JSONL line (with or without its terminal LF).
/// Session/cancel is a client notification, never a stop observation here.
pub(crate) fn decode(frame: &[u8], pending: Option<&Pending<'_>>)
    -> Result<Observation, DecodeError> {
    if frame.is_empty() || frame.len() > MAX_FRAME_BYTES {
        return Err(invalid(frame, "empty or oversized ACP frame"));
    }
    let body = frame.strip_suffix(b"\n").unwrap_or(frame);
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    if body.is_empty() || body.contains(&b'\n') || body.contains(&b'\r') {
        return Err(invalid(frame, "ACP frame must contain one JSON line"));
    }
    if !super::stream_json::depth_ok(body) {
        return Err(invalid(frame, "ACP JSON depth or string framing invalid"));
    }
    let utf8 = std::str::from_utf8(body)
        .map_err(|error| invalid(frame, format!("ACP UTF-8: {error}")))?;
    let parsed = Parser::parse(utf8)
        .map_err(|error| invalid(frame, format!("ACP JSON: {error:?}")))?;
    let fields = object(&parsed, frame, "ACP frame")?;
    if !matches!(field(fields, "jsonrpc"), Some(Json::String(value))
        if value.to_well_formed_string().as_deref() == Some("2.0")) {
        return Err(invalid(frame, "ACP jsonrpc must be 2.0"));
    }
    let method = field(fields, "method");
    let result = field(fields, "result");
    let error = field(fields, "error");
    if let Some(method) = method {
        if result.is_some() || error.is_some() {
            return Err(invalid(frame, "ACP method overlaps result or error"));
        }
        let method = text(method, frame, "ACP method")?;
        let params = field(fields, "params")
            .ok_or_else(|| invalid(frame, "ACP method params missing"))?;
        let params_obj = object(params, frame, "ACP method params")?;
        if let Some(id) = field(fields, "id") {
            let id = rpc_id(id, frame)?;
            if method == "session/request_permission" {
                return Ok(Observation::PermissionRequest { id, params: copy_json(params) });
            }
            return Ok(Observation::Unhandled { raw_frame: frame.to_vec() });
        }
        if method == "session/update" {
            let session_id = text(field(params_obj, "sessionId")
                .ok_or_else(|| invalid(frame, "session/update.sessionId missing"))?,
                frame, "session/update.sessionId")?;
            let update = field(params_obj, "update")
                .ok_or_else(|| invalid(frame, "session/update.update missing"))?;
            object(update, frame, "session/update.update")?;
            return Ok(Observation::SessionUpdate { session_id, update: copy_json(update) });
        }
        // A cancel notification, including one echoed by a provider, proves no
        // cancellation. Only the matching original prompt response can do so.
        return Ok(Observation::Unhandled { raw_frame: frame.to_vec() });
    }
    let id = rpc_id(field(fields, "id")
        .ok_or_else(|| invalid(frame, "ACP response id missing"))?, frame)?;
    if result.is_some() == error.is_some() {
        return Err(invalid(frame, "ACP result/error must be strictly exclusive"));
    }
    let Some(pending) = pending else {
        return Ok(Observation::Unhandled { raw_frame: frame.to_vec() });
    };
    if id != *pending.id {
        return Err(invalid(frame, "ACP response id does not match pending request"));
    }
    if let Some(error) = error {
        object(error, frame, "ACP error")?;
        return Ok(Observation::RemoteError { id, error: copy_json(error), raw_frame: frame.to_vec() });
    }
    let result = result.expect("exclusive result");
    let result_obj = object(result, frame, "ACP result")?;
    match pending.method {
        PendingMethod::Initialize => {
            if !matches!(field(result_obj, "protocolVersion"), Some(Json::Number(value)) if value == "1") {
                return Err(invalid(frame, "initialize.protocolVersion must be 1"));
            }
            let capabilities = field(result_obj, "agentCapabilities")
                .ok_or_else(|| invalid(frame, "initialize.agentCapabilities missing"))?;
            object(capabilities, frame, "initialize.agentCapabilities")?;
            Ok(Observation::Initialize { id, declared_capabilities: copy_json(capabilities), result: copy_json(result) })
        }
        PendingMethod::SessionNew => {
            let session_id = text(field(result_obj, "sessionId")
                .ok_or_else(|| invalid(frame, "session/new.sessionId missing"))?, frame, "session/new.sessionId")?;
            Ok(Observation::SessionNew { id, session_id, result: copy_json(result) })
        }
        PendingMethod::SessionLoad | PendingMethod::SessionResume => {
            let wanted = pending.requested_session_id
                .filter(|value| !value.is_empty() && !value.contains('\0'))
                .ok_or_else(|| invalid(frame, "session/load original requested session ID missing"))?;
            let echoed_session_id = field(result_obj, "sessionId")
                .map(|value| text(value, frame, "session/load.sessionId"))
                .transpose()?;
            if echoed_session_id.as_deref().is_some_and(|found| found != wanted) {
                return Err(invalid(frame, "session/load echoed a different session ID"));
            }
            if pending.method == PendingMethod::SessionLoad {
                Ok(Observation::SessionLoad { id, echoed_session_id, result: copy_json(result) })
            } else {
                Ok(Observation::SessionResume { id, echoed_session_id, result: copy_json(result) })
            }
        }
        PendingMethod::SessionPrompt => {
            let reason = field(result_obj, "stopReason")
                .ok_or_else(|| invalid(frame, "session/prompt.stopReason missing"))?;
            Ok(Observation::Prompt { id, stop_reason: stop_reason(reason, frame)?, result: copy_json(result) })
        }
        PendingMethod::SessionSetConfigOption => {
            let options = field(result_obj, "configOptions")
                .ok_or_else(|| invalid(frame, "session/set_config_option.configOptions missing"))?;
            if !matches!(options, Json::Array(_)) {
                return Err(invalid(frame, "session/set_config_option.configOptions must be an array"));
            }
            Ok(Observation::SessionConfigOption { id,
                config_options: copy_json(options), result: copy_json(result) })
        }
    }
}

/// The success ACK must report the exact requested value as current for one
/// select option, and include it among that option's values. Notifications
/// and caller-supplied labels never establish this fact.
pub(crate) fn confirms_config_value(observation: &Observation,
    config_id: &str, value: &str) -> bool {
    let Observation::SessionConfigOption { config_options: Json::Array(options), .. } = observation
        else { return false };
    let mut found = false;
    for option in options {
        let Json::Object(fields) = option else { return false };
        let Some(Json::String(id)) = field(fields, "id") else { return false };
        if id.to_well_formed_string().as_deref() != Some(config_id) { continue; }
        if found { return false; }
        found = true;
        if !matches!(field(fields, "type"), Some(Json::String(kind))
            if kind.to_well_formed_string().as_deref() == Some("select")) { return false; }
        if !matches!(field(fields, "currentValue"), Some(Json::String(current))
            if current.to_well_formed_string().as_deref() == Some(value)) { return false; }
        let Some(Json::Array(choices)) = field(fields, "options") else { return false };
        if !choices.iter().any(|choice| matches!(choice, Json::Object(fields)
            if matches!(field(fields, "value"), Some(Json::String(candidate))
                if candidate.to_well_formed_string().as_deref() == Some(value)))) {
            return false;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_ack_requires_original_typed_id_and_exact_offered_current_value() {
        let id = RpcId::String("setting-model".to_owned());
        let pending = Pending { id: &id, method: PendingMethod::SessionSetConfigOption,
            requested_session_id: Some("vendor-session") };
        let raw = br#"{"jsonrpc":"2.0","id":"setting-model","result":{"configOptions":[{"id":"model","type":"select","currentValue":"provider/model","options":[{"value":"provider/model"}]}]}}"#;
        let observed = decode(raw, Some(&pending)).ok().expect("matching original response");
        assert!(confirms_config_value(&observed, "model", "provider/model"));
        assert!(!confirms_config_value(&observed, "model", "other/model"));
        assert!(!confirms_config_value(&observed, "effort", "high"));
        let numeric = RpcId::Number(1);
        let wrong_type = Pending { id: &numeric, method: PendingMethod::SessionSetConfigOption,
            requested_session_id: Some("vendor-session") };
        assert!(decode(raw, Some(&wrong_type)).is_err());
    }
}
