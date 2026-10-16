//! Vendor stream-json field extraction only. The caller must bind a captured
//! native frame to its H call, session, and generation before using this data.

use crate::store::atomic::{AtomicError, Json, JsonString, Parser};
use std::collections::BTreeMap;

// Match the existing native session RPC frame and parser-depth bounds.
const MAX_FRAME: usize = 1024 * 1024;
const MAX_DEPTH: usize = 128;

#[derive(Debug)]
pub(crate) enum StreamJsonError {
    Invalid(&'static str),
    Json(AtomicError),
    Utf8(std::str::Utf8Error),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ClaudeData {
    Init { session_id: String, model: Option<String> },
    ControlResponse { request_id: String, success: bool },
    UserReplay { session_id: String, uuid: String, text: String },
    Result { session_id: String, subtype: String, is_error: bool },
    ControlRequest { request_id: Option<String>, subtype: Option<String> },
    Unhandled { frame_type: Option<String> },
}

impl ClaudeData {
    /// A vendor terminal candidate, never a host delivery or success receipt.
    pub(crate) fn reports_success(&self) -> bool {
        matches!(self, Self::Result { subtype, is_error: false, .. } if subtype == "success")
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AntigravityData {
    Init { conversation_id: String },
    Result {
        // Missing ID is retained as missing; H may compare against its already
        // bound conversation, but this decoder cannot create or infer one.
        conversation_id: Option<String>,
        status: String,
        error: Option<String>,
        response: Option<String>,
    },
    Unhandled { event: String },
}

impl AntigravityData {
    /// Only the CLI's own terminal claim; H still has to establish origin and binding.
    pub(crate) fn reports_success(&self) -> bool {
        matches!(self, Self::Result {
            conversation_id: Some(_), status, error: None, response: Some(_),
        } if status == "SUCCESS")
    }
}

pub(crate) fn decode_claude_line(line: &[u8]) -> Result<ClaudeData, StreamJsonError> {
    let root = parse_object(line)?;
    let frame_type = optional_string(&root, "type")?;
    match frame_type.as_deref() {
        Some("system") if optional_string(&root, "subtype")?.as_deref() == Some("init") => {
            let session_id = required_id(&root, "session_id")?;
            let model = match root.get(&key("model")) {
                Some(Json::String(value)) => Some(well_formed(value)?),
                Some(Json::Object(model)) => optional_string(model, "id")?,
                _ => None,
            };
            Ok(ClaudeData::Init { session_id, model })
        }
        Some("result") => {
            let session_id = required_id(&root, "session_id")?;
            let subtype = required_id(&root, "subtype")?;
            let is_error = match root.get(&key("is_error")) {
                Some(Json::Bool(value)) => *value,
                _ => return Err(StreamJsonError::Invalid("result.is_error")),
            };
            Ok(ClaudeData::Result { session_id, subtype, is_error })
        }
        Some("control_response") => {
            let Some(Json::Object(response)) = root.get(&key("response")) else {
                return Err(StreamJsonError::Invalid("control_response.response"));
            };
            let request_id = required_id(response, "request_id")?;
            let subtype = required_id(response, "subtype")?;
            Ok(ClaudeData::ControlResponse {
                request_id, success: subtype == "success",
            })
        }
        Some("user") => {
            if !matches!(root.get(&key("parent_tool_use_id")), None | Some(Json::Null)) {
                return Ok(ClaudeData::Unhandled { frame_type });
            }
            let Some(Json::Object(message)) = root.get(&key("message")) else {
                return Ok(ClaudeData::Unhandled { frame_type });
            };
            if optional_string(message, "role").ok().flatten().as_deref() != Some("user") {
                return Ok(ClaudeData::Unhandled { frame_type });
            }
            let text = match message.get(&key("content")) {
                Some(Json::String(value)) => well_formed(value).ok(),
                Some(Json::Array(parts)) if parts.len() == 1 => {
                    let Json::Object(part) = &parts[0] else {
                        return Ok(ClaudeData::Unhandled { frame_type });
                    };
                    if optional_string(part, "type").ok().flatten().as_deref() != Some("text") {
                        return Ok(ClaudeData::Unhandled { frame_type });
                    }
                    required_id(part, "text").ok()
                }
                _ => return Ok(ClaudeData::Unhandled { frame_type }),
            };
            let Some(text)=text.filter(|text|!text.is_empty()&&!text.contains('\0'))
                else {return Ok(ClaudeData::Unhandled {frame_type})};
            let Ok(session_id) = required_id(&root, "session_id") else {
                return Ok(ClaudeData::Unhandled { frame_type });
            };
            let Ok(uuid) = required_id(&root, "uuid") else {
                return Ok(ClaudeData::Unhandled { frame_type });
            };
            Ok(ClaudeData::UserReplay { session_id, uuid, text })
        }
        Some("control_request") => {
            let request_id = optional_string(&root, "request_id")?;
            let subtype = match root.get(&key("request")) {
                Some(Json::Object(request)) => optional_string(request, "subtype")?,
                _ => None,
            };
            Ok(ClaudeData::ControlRequest { request_id, subtype })
        }
        _ => Ok(ClaudeData::Unhandled { frame_type }),
    }
}

/// Recognize only the published SDK's pure tool-result User shape. The raw
/// frame remains Unhandled output and can never acknowledge Host stdin.
pub(crate) fn is_claude_tool_result_line(line: &[u8]) -> bool {
    let Ok(root) = parse_object(line) else { return false; };
    if optional_string(&root, "type").ok().flatten().as_deref() != Some("user")
        || required_id(&root, "session_id").is_err()
        || optional_id(&root, "parent_tool_use_id").is_err() {
        return false;
    }
    let Some(Json::Object(message)) = root.get(&key("message")) else { return false; };
    if optional_string(message, "role").ok().flatten().as_deref() != Some("user") {
        return false;
    }
    let Some(Json::Array(parts)) = message.get(&key("content")) else { return false; };
    !parts.is_empty() && parts.iter().all(|part| match part {
        Json::Object(block) =>
            optional_string(block, "type").ok().flatten().as_deref()
                == Some("tool_result") && required_id(block, "tool_use_id").is_ok(),
        _ => false,
    })
}

pub(crate) fn decode_antigravity_line(line: &[u8]) -> Result<AntigravityData, StreamJsonError> {
    let root = parse_object(line)?;
    let event = required_id(&root, "event")?;
    match event.as_str() {
        "init" => Ok(AntigravityData::Init {
            conversation_id: required_id(&root, "conversation_id")?,
        }),
        "result" => {
            let result = match root.get(&key("result")) {
                Some(Json::Object(result)) => result,
                _ => return Err(StreamJsonError::Invalid("result object")),
            };
            let top_id = optional_id(&root, "conversation_id")?;
            let result_id = optional_id(result, "conversation_id")?;
            if top_id.is_some() && result_id.is_some() && top_id != result_id {
                return Err(StreamJsonError::Invalid("conflicting conversation_id"));
            }
            let status = required_id(result, "status")?;
            let error = optional_string(result, "error")?;
            let response = optional_string(result, "response")?;
            Ok(AntigravityData::Result {
                // The documented result ID is inside `result`. An envelope
                // ID alone cannot turn this into a bound terminal candidate.
                conversation_id: result_id, status, error, response,
            })
        }
        _ => Ok(AntigravityData::Unhandled { event }),
    }
}

fn parse_object(line: &[u8]) -> Result<BTreeMap<JsonString, Json>, StreamJsonError> {
    if line.is_empty() || line.len() > MAX_FRAME {
        return Err(StreamJsonError::Invalid("frame bounds"));
    }
    // OriginBoundFrame retains the terminal line ending in A. Only the parser
    // view removes it; the captured bytes and source digest remain unchanged.
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() || line.contains(&b'\n') || line.contains(&b'\r') {
        return Err(StreamJsonError::Invalid("one JSON line required"));
    }
    if !depth_ok(line) {
        return Err(StreamJsonError::Invalid("JSON depth"));
    }
    let text = std::str::from_utf8(line).map_err(StreamJsonError::Utf8)?;
    match Parser::parse(text).map_err(StreamJsonError::Json)? {
        Json::Object(fields) => Ok(fields),
        _ => Err(StreamJsonError::Invalid("object required")),
    }
}

// Same string-aware depth scan as the native session RPC parser. Parser::parse
// recurses, so this must run on the original bytes before parsing.
pub(super) fn depth_ok(bytes: &[u8]) -> bool {
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for &byte in bytes {
        if quoted {
            if escaped { escaped = false; }
            else if byte == b'\\' { escaped = true; }
            else if byte == b'"' { quoted = false; }
        } else {
            match byte {
                b'"' => quoted = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_DEPTH { return false; }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    !quoted && depth == 0
}

fn key(name: &str) -> JsonString { JsonString::from_str(name) }

fn well_formed(value: &JsonString) -> Result<String, StreamJsonError> {
    value.to_well_formed_string().ok_or(StreamJsonError::Invalid("string Unicode"))
}

fn optional_string(fields: &BTreeMap<JsonString, Json>, name: &str)
    -> Result<Option<String>, StreamJsonError>
{
    match fields.get(&key(name)) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(value)) => Ok(Some(well_formed(value)?)),
        _ => Err(StreamJsonError::Invalid("string field type")),
    }
}

fn optional_id(fields: &BTreeMap<JsonString, Json>, name: &str)
    -> Result<Option<String>, StreamJsonError>
{
    let value = optional_string(fields, name)?;
    if value.as_deref().is_some_and(|id| id.is_empty() || id.contains('\0')) {
        return Err(StreamJsonError::Invalid("empty or NUL id"));
    }
    Ok(value)
}

fn required_id(fields: &BTreeMap<JsonString, Json>, name: &str)
    -> Result<String, StreamJsonError>
{
    optional_id(fields, name)?.ok_or(StreamJsonError::Invalid("required string id"))
}

// References: gogo-party/packages/seat-runtime/src/claude-seat.ts;
// docs/research/2026-09-26-antigravity-cli-facts.md;
// third_party/t3code/apps/server/src/gogoke/adapters/{claude,antigravity}/;
// https://code.claude.com/docs/en/headless
// https://antigravity.google/docs/cli/headless/ (fixed adapter pin: 1.2.11).

#[cfg(test)]
mod claude_ack_tests {
    use super::*;

    #[test]
    fn only_the_original_user_echo_is_a_user_replay_candidate() {
        let init = br#"{"type":"control_response","response":{"subtype":"success","request_id":"original-init","response":{}}}"#;
        assert_eq!(decode_claude_line(init).ok(), Some(ClaudeData::ControlResponse {
            request_id: "original-init".into(), success: true,
        }));
        let echo = br#"{"type":"user","session_id":"native-session","uuid":"user-uuid","parent_tool_use_id":null,"message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#;
        assert_eq!(decode_claude_line(echo).ok(), Some(ClaudeData::UserReplay {
            session_id: "native-session".into(), uuid: "user-uuid".into(), text: "hello".into(),
        }));
        let untagged = br#"{"type":"user","session_id":"native-session","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#;
        assert!(matches!(decode_claude_line(untagged),
            Ok(ClaudeData::Unhandled { frame_type: Some(kind) }) if kind=="user"));
        let tool = br#"{"type":"user","session_id":"native-session","parent_tool_use_id":"tool-1","message":{"role":"user","content":[{"type":"tool_result","content":"done"}]}}"#;
        assert!(matches!(decode_claude_line(tool), Ok(ClaudeData::Unhandled { .. })));
        let tool_result = br#"{"type":"user","session_id":"native-session","parent_tool_use_id":null,"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-1","content":"done"}]}}"#;
        assert!(is_claude_tool_result_line(tool_result));
        assert!(matches!(decode_claude_line(tool_result), Ok(ClaudeData::Unhandled { .. })));
        let mixed = br#"{"type":"user","session_id":"native-session","parent_tool_use_id":null,"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-1"},{"type":"text","text":"another user"}]}}"#;
        assert!(matches!(decode_claude_line(mixed),
            Ok(ClaudeData::Unhandled { frame_type: Some(kind) }) if kind=="user"));
        assert!(!is_claude_tool_result_line(mixed));
        let terminal = br#"{"type":"result","session_id":"native-session","subtype":"success","is_error":false}"#;
        assert!(matches!(decode_claude_line(terminal), Ok(ClaudeData::Result { .. })));
    }
}
