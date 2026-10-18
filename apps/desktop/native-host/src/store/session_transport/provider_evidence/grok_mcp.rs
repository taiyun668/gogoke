//! Source-only codec for one opt-in Grok ACP reverse MCP server. A persists the
//! original frame first; H supplies the registered server token and later
//! validates session, prompt, generation, and writer custody. This module does
//! not grant a tool or produce a card.

use super::acp::{DecodeError, RpcId};
use crate::store::atomic::{Json, JsonString, Parser};
use std::collections::{BTreeMap, BTreeSet};

const MAX_FRAME_BYTES: usize = 64 * 1024;
const MAX_SAFE_ID: i64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OptionText {
    pub(crate) label: String,
    pub(crate) description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AskUserInput {
    pub(crate) header: String,
    pub(crate) question: String,
    pub(crate) options: Vec<OptionText>,
    pub(crate) recommended_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Initialize { protocol_version: String },
    ToolsList,
    AskUser(AskUserInput),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub(crate) outer_id: RpcId,
    pub(crate) inner_id: RpcId,
    pub(crate) server_id: String,
    pub(crate) kind: Kind,
    pub(crate) raw_frame: Vec<u8>,
}

fn invalid(frame: &[u8], reason: impl Into<String>) -> DecodeError {
    DecodeError { reason: reason.into(), raw_frame: frame.to_vec() }
}
fn key(value: &str) -> JsonString { JsonString::from_str(value) }
fn field<'a>(object: &'a BTreeMap<JsonString, Json>, name: &str) -> Option<&'a Json> {
    object.get(&key(name))
}
fn object<'a>(value: &'a Json, frame: &[u8], name: &str)
    -> Result<&'a BTreeMap<JsonString, Json>, DecodeError> {
    if let Json::Object(found) = value { Ok(found) }
    else { Err(invalid(frame, format!("{name} must be an object"))) }
}
fn only(object: &BTreeMap<JsonString, Json>, allowed: &[&str], frame: &[u8], name: &str)
    -> Result<(), DecodeError> {
    for key in object.keys() {
        let found = key.to_well_formed_string()
            .ok_or_else(|| invalid(frame, format!("{name} has invalid key")))?;
        if !allowed.contains(&found.as_str()) {
            return Err(invalid(frame, format!("{name} has unknown field {found}")));
        }
    }
    Ok(())
}
fn required<'a>(object: &'a BTreeMap<JsonString, Json>, frame: &[u8], name: &str)
    -> Result<&'a Json, DecodeError> {
    field(object, name).ok_or_else(|| invalid(frame, format!("{name} missing")))
}
fn text(value: &Json, frame: &[u8], name: &str, max: usize) -> Result<String, DecodeError> {
    let Json::String(value) = value else {
        return Err(invalid(frame, format!("{name} must be text")));
    };
    let found = value.to_well_formed_string()
        .ok_or_else(|| invalid(frame, format!("{name} has invalid surrogate")))?;
    if found.is_empty() || found.len() > max || found.contains('\0') {
        return Err(invalid(frame, format!("{name} is empty or too large")));
    }
    Ok(found)
}
fn id(value: &Json, frame: &[u8]) -> Result<RpcId, DecodeError> {
    match value {
        Json::String(_) => Ok(RpcId::String(text(value, frame, "MCP id", 128)?)),
        Json::Number(number) => {
            let found = number.parse::<i64>()
                .map_err(|error| invalid(frame, format!("MCP id must be integer: {error}")))?;
            if !(-MAX_SAFE_ID..=MAX_SAFE_ID).contains(&found) {
                return Err(invalid(frame, "MCP id outside safe integer range"));
            }
            Ok(RpcId::Number(found))
        }
        _ => Err(invalid(frame, "MCP id must be number or text")),
    }
}
fn version(value: &Json, frame: &[u8]) -> Result<String, DecodeError> {
    let found = text(value, frame, "MCP protocolVersion", 32)?;
    if !found.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-') {
        return Err(invalid(frame, "MCP protocolVersion has invalid characters"));
    }
    Ok(found)
}
pub(crate) fn validate_server_id(value: &str) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > 128
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-') {
        return Err("Grok MCP server ID must be a controlled ASCII token");
    }
    Ok(())
}

/// The shared Json parser canonicalizes every number through f64. On this
/// one-tool schema all numbers are RPC IDs or recommendedIndex, so inspect
/// their original wire tokens before reading any normalized Json::Number.
/// A string containing decimal-looking text is deliberately ignored.
fn require_integer_wire_tokens(body: &[u8], frame: &[u8]) -> Result<(), DecodeError> {
    let mut quoted = false;
    let mut escaped = false;
    let mut cursor = 0;
    while cursor < body.len() {
        let byte = body[cursor];
        if quoted {
            if escaped { escaped = false; }
            else if byte == b'\\' { escaped = true; }
            else if byte == b'"' { quoted = false; }
            cursor += 1;
        } else if byte == b'"' {
            quoted = true;
            cursor += 1;
        } else if byte == b'-' || byte.is_ascii_digit() {
            let start = cursor;
            cursor += 1;
            while cursor < body.len() && !matches!(body[cursor],
                b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n') {
                cursor += 1;
            }
            if body[start..cursor].iter().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
                return Err(invalid(frame, "Grok MCP numeric wire token must be integer"));
            }
        } else {
            cursor += 1;
        }
    }
    Ok(())
}

/// `None` means another ACP method, including permission, text/update or a
/// different vendor method. An exact sdk_call with bad shape is an error.
pub(crate) fn decode(frame: &[u8], expected_server_id: &str)
    -> Result<Option<Request>, DecodeError> {
    validate_server_id(expected_server_id).map_err(|reason| invalid(frame, reason))?;
    if frame.is_empty() || frame.len() > MAX_FRAME_BYTES {
        return Err(invalid(frame, "Grok MCP frame empty or oversized"));
    }
    let body = frame.strip_suffix(b"\n").unwrap_or(frame);
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    if body.is_empty() || body.contains(&b'\n') || body.contains(&b'\r')
        || !super::stream_json::depth_ok(body) {
        return Err(invalid(frame, "Grok MCP frame is not one bounded JSON line"));
    }
    let utf8 = std::str::from_utf8(body)
        .map_err(|error| invalid(frame, format!("Grok MCP UTF-8: {error}")))?;
    let parsed = Parser::parse(utf8)
        .map_err(|error| invalid(frame, format!("Grok MCP JSON: {error:?}")))?;
    let outer = object(&parsed, frame, "outer ACP")?;
    let Some(method) = field(outer, "method") else { return Ok(None) };
    if text(method, frame, "outer method", 64)? != "x.ai/mcp/sdk_call" {
        return Ok(None);
    }
    require_integer_wire_tokens(body, frame)?;
    only(outer, &["jsonrpc", "id", "method", "params"], frame, "outer ACP")?;
    if text(required(outer, frame, "jsonrpc")?, frame, "outer jsonrpc", 3)? != "2.0" {
        return Err(invalid(frame, "outer jsonrpc must be 2.0"));
    }
    let outer_id = id(required(outer, frame, "id")?, frame)?;
    let params = object(required(outer, frame, "params")?, frame, "sdk_call.params")?;
    only(params, &["serverId", "message"], frame, "sdk_call.params")?;
    let server_id = text(required(params, frame, "serverId")?, frame, "serverId", 128)?;
    if server_id != expected_server_id {
        return Err(invalid(frame, "sdk_call serverId does not match H registration"));
    }
    let inner = object(required(params, frame, "message")?, frame, "sdk_call.message")?;
    only(inner, &["jsonrpc", "id", "method", "params"], frame, "sdk_call.message")?;
    if text(required(inner, frame, "jsonrpc")?, frame, "inner jsonrpc", 3)? != "2.0" {
        return Err(invalid(frame, "inner jsonrpc must be 2.0"));
    }
    let inner_id = id(required(inner, frame, "id")?, frame)?;
    let method = text(required(inner, frame, "method")?, frame, "inner method", 32)?;
    let kind = match method.as_str() {
        "initialize" => {
            let params = object(required(inner, frame, "params")?, frame, "initialize.params")?;
            only(params, &["protocolVersion", "capabilities", "clientInfo"], frame,
                "initialize.params")?;
            let protocol_version = version(required(params, frame, "protocolVersion")?, frame)?;
            let capabilities = object(required(params, frame, "capabilities")?, frame,
                "initialize.capabilities")?;
            only(capabilities, &[], frame, "initialize.capabilities")?;
            let client = object(required(params, frame, "clientInfo")?, frame, "initialize.clientInfo")?;
            only(client, &["name", "version"], frame, "initialize.clientInfo")?;
            text(required(client, frame, "name")?, frame, "clientInfo.name", 128)?;
            text(required(client, frame, "version")?, frame, "clientInfo.version", 64)?;
            Kind::Initialize { protocol_version }
        }
        "tools/list" => {
            if let Some(params) = field(inner, "params") {
                let params = object(params, frame, "tools/list.params")?;
                only(params, &[], frame, "tools/list.params")?;
            }
            Kind::ToolsList
        }
        "tools/call" => {
            let params = object(required(inner, frame, "params")?, frame, "tools/call.params")?;
            only(params, &["name", "arguments"], frame, "tools/call.params")?;
            if text(required(params, frame, "name")?, frame, "tool name", 64)? != "ask_user" {
                return Err(invalid(frame, "unknown Grok MCP tool"));
            }
            let args = object(required(params, frame, "arguments")?, frame, "ask_user.arguments")?;
            only(args, &["header", "question", "options", "recommendedIndex"], frame,
                "ask_user.arguments")?;
            let header = text(required(args, frame, "header")?, frame, "header", 80)?;
            let question = text(required(args, frame, "question")?, frame, "question", 4096)?;
            let Json::Array(values) = required(args, frame, "options")? else {
                return Err(invalid(frame, "options must be array"));
            };
            if values.is_empty() || values.len() > 8 {
                return Err(invalid(frame, "options count outside ask_user schema"));
            }
            let mut options = Vec::with_capacity(values.len());
            let mut labels = BTreeSet::new();
            for value in values {
                let item = object(value, frame, "option")?;
                only(item, &["label", "description"], frame, "option")?;
                let label = text(required(item, frame, "label")?, frame, "option.label", 120)?;
                let description = text(required(item, frame, "description")?, frame,
                    "option.description", 512)?;
                if !labels.insert(label.clone()) {
                    return Err(invalid(frame, "duplicate displayed option label"));
                }
                options.push(OptionText { label, description });
            }
            let Json::Number(index) = required(args, frame, "recommendedIndex")? else {
                return Err(invalid(frame, "recommendedIndex must be integer"));
            };
            let recommended_index = index.parse::<usize>()
                .map_err(|error| invalid(frame, format!("recommendedIndex: {error}")))?;
            if recommended_index >= options.len() {
                return Err(invalid(frame, "recommendedIndex outside options"));
            }
            Kind::AskUser(AskUserInput { header, question, options, recommended_index })
        }
        _ => return Err(invalid(frame, "unknown Grok MCP method")),
    };
    Ok(Some(Request { outer_id, inner_id, server_id, kind, raw_frame: frame.to_vec() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{acp, commands::{self, AcpCommand, GrokMcpReply, Vendor}};

    const CALL: &str = r#"{"jsonrpc":"2.0","id":"outer-7","method":"x.ai/mcp/sdk_call","params":{"serverId":"srv_7","message":{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"ask_user","arguments":{"header":"Format","question":"Choose output format","options":[{"label":"Markdown","description":"Readable report"},{"label":"JSON","description":"Structured report"}],"recommendedIndex":0}}}}}"#;

    #[test]
    fn grok_mcp_nested_reply_retains_source_and_both_typed_ids() {
        let request = decode(CALL.as_bytes(), "srv_7").unwrap().unwrap();
        assert_eq!(request.outer_id, RpcId::String("outer-7".into()));
        assert_eq!(request.inner_id, RpcId::Number(7));
        assert_eq!(request.raw_frame, CALL.as_bytes());
        let reply = commands::encode_grok_mcp_reply(&request, GrokMcpReply::Answer("Markdown"))
            .unwrap();
        let reply = std::str::from_utf8(&reply).unwrap();
        assert!(reply.contains("\"id\":\"outer-7\""));
        assert!(reply.contains("\"id\":7"));
        assert!(reply.contains("\\\"answer\\\":\\\"Markdown\\\""));
        let reversed = CALL.replace("\"outer-7\"", "7")
            .replace("\"id\":7,\"method\":\"tools/call\"", "\"id\":\"inner-7\",\"method\":\"tools/call\"");
        let typed = decode(reversed.as_bytes(), "srv_7").unwrap().unwrap();
        assert_eq!(typed.outer_id, RpcId::Number(7));
        assert_eq!(typed.inner_id, RpcId::String("inner-7".into()));
    }

    #[test]
    fn grok_mcp_bootstrap_and_registration_are_explicit() {
        let default = commands::encode_acp(Vendor::Grok, Some(&RpcId::Number(1)),
            AcpCommand::SessionNew { cwd: "D:/synthetic" }).unwrap();
        assert!(!std::str::from_utf8(&default).unwrap().contains("x.ai/mcp/servers"));
        let registered = commands::encode_grok_session_new_host_mcp(
            &RpcId::Number(1), "D:/synthetic", "srv_7").unwrap();
        assert!(std::str::from_utf8(&registered).unwrap().contains("x.ai/mcp/servers"));
        assert!(commands::encode_grok_session_new_host_mcp(
            &RpcId::Number(1), "D:/synthetic", "bad/server").is_err());
        let initialize = br#"{"jsonrpc":"2.0","id":12,"method":"x.ai/mcp/sdk_call","params":{"serverId":"srv_7","message":{"jsonrpc":"2.0","id":"init-1","method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"grok","version":"1"}}}}}"#;
        let init = decode(initialize, "srv_7").unwrap().unwrap();
        let reply = commands::encode_grok_mcp_reply(&init, GrokMcpReply::Bootstrap).unwrap();
        let reply = std::str::from_utf8(&reply).unwrap();
        assert!(reply.contains("\"id\":12"));
        assert!(reply.contains("\"id\":\"init-1\""));
        assert!(reply.contains("2025-03-26"));
        let list = br#"{"jsonrpc":"2.0","id":"list-out","method":"x.ai/mcp/sdk_call","params":{"serverId":"srv_7","message":{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}}}"#;
        let list = decode(list, "srv_7").unwrap().unwrap();
        let reply = commands::encode_grok_mcp_reply(&list, GrokMcpReply::Bootstrap).unwrap();
        assert!(std::str::from_utf8(&reply).unwrap().contains("ask_user"));
        assert!(commands::encode_grok_mcp_reply(&list, GrokMcpReply::Answer("x")).is_err());
    }

    #[test]
    fn grok_mcp_rejects_wrong_binding_unknown_method_tool_and_authority_fields() {
        assert!(decode(CALL.as_bytes(), "other").is_err());
        assert!(decode(CALL.replace("ask_user", "write_file").as_bytes(), "srv_7").is_err());
        assert!(decode(CALL.replace("tools/call", "resources/read").as_bytes(), "srv_7").is_err());
        assert!(decode(CALL.replace("\"recommendedIndex\":0", "\"recommendedIndex\":\"0\"")
            .as_bytes(), "srv_7").is_err());
        assert!(decode(CALL.replace("\"header\":\"Format\"",
            "\"header\":\"Format\",\"domain\":\"root\"").as_bytes(), "srv_7").is_err());
        assert!(decode(CALL.replace("\"question\":\"Choose output format\"",
            "\"question\":\"\"").as_bytes(), "srv_7").is_err());
        assert!(decode(CALL.replace("\"label\":\"JSON\"",
            "\"label\":\"Markdown\"").as_bytes(), "srv_7").is_err());
        assert!(decode(CALL.replace("\"id\":7,\"method\"", "\"id\":null,\"method\"")
            .as_bytes(), "srv_7").is_err());
        assert!(decode(CALL.repeat(1000).as_bytes(), "srv_7").is_err());
        let request = decode(CALL.as_bytes(), "srv_7").unwrap().unwrap();
        assert!(commands::encode_grok_mcp_reply(&request,
            GrokMcpReply::Answer(&"a".repeat(4097))).is_err());
        let permission = br#"{"jsonrpc":"2.0","id":4,"method":"session/request_permission","params":{"sessionId":"s"}}"#;
        assert!(decode(permission, "srv_7").unwrap().is_none());
        assert!(matches!(acp::decode(permission, None).unwrap(),
            acp::Observation::PermissionRequest { .. }));
        assert!(matches!(acp::decode(CALL.as_bytes(), None).unwrap(),
            acp::Observation::Unhandled { .. }));
    }

    #[test]
    fn grok_mcp_rejects_fractional_wire_tokens_before_f64_normalization() {
        // Validate the measurement: this token really normalizes to integer 1,
        // while the auditor's 0.0000000000000001 does not normalize to zero.
        assert_eq!(Parser::parse("1.0000000000000001").unwrap().canonical(), "1");
        assert_ne!(Parser::parse("0.0000000000000001").unwrap().canonical(), "0");
        let outer = CALL.replace("\"id\":\"outer-7\"", "\"id\":1.0000000000000001");
        let inner = CALL.replace("\"id\":7,\"method\":\"tools/call\"",
            "\"id\":1.0000000000000001,\"method\":\"tools/call\"");
        let index = CALL.replace("\"recommendedIndex\":0",
            "\"recommendedIndex\":1.0000000000000001");
        for raw in [&outer, &inner, &index] {
            let error = acp::decode_grok_mcp(raw.as_bytes(), "srv_7").err().unwrap();
            assert_eq!(error.reason, "Grok MCP numeric wire token must be integer");
            assert_eq!(error.raw_frame, raw.as_bytes());
        }
        assert!(decode(CALL.replace("\"id\":7,\"method\"",
            "\"id\":1e0,\"method\"").as_bytes(), "srv_7").is_err());
        let content = CALL.replace("Choose output format",
            "Choose 1.0000000000000001 output format");
        assert!(decode(content.as_bytes(), "srv_7").unwrap().is_some());
    }

    #[test]
    fn grok_mcp_reply_keeps_original_redecode_failure() {
        let mut request = decode(CALL.as_bytes(), "srv_7").unwrap().unwrap();
        request.raw_frame = CALL.replace("ask_user", "write_file").into_bytes();
        let error = commands::encode_grok_mcp_reply(&request,
            GrokMcpReply::Answer("Markdown")).err().unwrap();
        match error {
            commands::EncodeError::SourceDecode(source) => {
                assert_eq!(source.reason, "unknown Grok MCP tool");
                assert_eq!(source.raw_frame, request.raw_frame);
            }
            other => panic!("expected original source decode error, got {other:?}"),
        }
    }
}
