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
