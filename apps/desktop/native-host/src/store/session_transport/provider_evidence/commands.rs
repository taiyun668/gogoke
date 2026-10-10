//! Pinned vendor stdin encodings. This module is deliberately not wired to H.
//! H owns admission, process identity, request IDs, journal, session binding,
//! capability observations, and OriginBoundFrame capture before any decode.

use crate::store::atomic::{Json, JsonString};
use std::collections::BTreeMap;

use super::acp::RpcId;

const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_SAFE_ID: i64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Vendor {
    Claude,
    OpenCode,
    Grok,
    Antigravity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EncodeError {
    Invalid(&'static str),
    Unsupported(&'static str),
}

pub(crate) enum AcpCommand<'a> {
    Initialize {
        client_version: &'a str,
    },
    SessionNew {
        cwd: &'a str,
    },
    /// The caller must have observed loadSession=true on this exact process.
    SessionLoad {
        session_id: &'a str,
        cwd: &'a str,
        advertised: bool,
    },
    /// OpenCode's pinned adapter uses session/resume; Grok uses session/load.
    SessionResume {
        session_id: &'a str,
        cwd: &'a str,
        advertised: bool,
    },
    /// OpenCode 1.18.32 only. H verifies the original session and the
    /// response's currentValue before treating the setting as applied.
    SetConfigOption {
        session_id: &'a str,
        config_id: &'a str,
        value: &'a str,
    },
    Prompt {
        session_id: &'a str,
        text: &'a str,
    },
    /// A notification only. Its write cannot prove cancellation.
    Cancel {
        session_id: &'a str,
    },
    PermissionResponse,
    Steer,
}

pub(crate) enum ClaudeCommand<'a> {
    /// Published Agent SDK control handshake. It performs no model turn and
    /// its response does not claim a vendor session ID.
    Initialize { request_id: &'a str },
    /// UUID is derived once from original H User request bytes by journal.rs.
    User { uuid: &'a str, text: &'a str },
}

fn string(value: &str) -> Json {
    Json::String(JsonString::from_str(value))
}
fn object<const N: usize>(fields: [(&str, Json); N]) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(key, value)| (JsonString::from_str(key), value))
            .collect::<BTreeMap<_, _>>(),
    )
}
fn nonempty(value: &str, name: &'static str) -> Result<(), EncodeError> {
    if value.is_empty() || value.contains('\0') {
        Err(EncodeError::Invalid(name))
    } else {
        Ok(())
    }
}
fn rpc_id(id: &RpcId) -> Result<Json, EncodeError> {
    match id {
        RpcId::Number(value) if (-MAX_SAFE_ID..=MAX_SAFE_ID).contains(value) => {
            Ok(Json::Number(value.to_string()))
        }
        RpcId::String(value) => {
            nonempty(value, "rpc id")?;
            Ok(string(value))
        }
        _ => Err(EncodeError::Invalid("rpc id outside safe integer range")),
    }
}
fn jsonl(value: Json) -> Result<Vec<u8>, EncodeError> {
    let mut bytes = value.canonical().into_bytes();
    if bytes.len() + 1 > MAX_LINE_BYTES {
        return Err(EncodeError::Invalid("vendor line too large"));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// A reply to one H-bound original Grok server request. The caller must
/// derive `option_id` from that request's options, never from a label or tier.
pub(crate) fn encode_grok_permission_reply(
    id: &RpcId, option_id: Option<&str>,
) -> Result<Vec<u8>, EncodeError> {
    let outcome = if let Some(option) = option_id {
        nonempty(option, "permission option id")?;
        object([("outcome", string("selected")), ("optionId", string(option))])
    } else {
        object([("outcome", string("cancelled"))])
    };
    jsonl(object([
        ("jsonrpc", string("2.0")),
        ("id", rpc_id(id)?),
        ("result", object([("outcome", outcome)])),
    ]))
}

/// Encode one ACP JSON-RPC request or notification. `id` must be the original
/// H journal ID for requests and absent for cancel notifications. The returned
/// bytes say nothing about vendor receipt, authority, permission or delivery.
pub(crate) fn encode_acp(
    vendor: Vendor,
    id: Option<&RpcId>,
    command: AcpCommand<'_>,
) -> Result<Vec<u8>, EncodeError> {
    if !matches!(vendor, Vendor::OpenCode | Vendor::Grok) {
        return Err(EncodeError::Unsupported(
            "ACP is not this vendor's pinned transport",
        ));
    }
    let (method, params, notification) = match command {
        AcpCommand::Initialize { client_version } => {
            nonempty(client_version, "client version")?;
            let fs = object([
                ("readTextFile", Json::Bool(false)),
                ("writeTextFile", Json::Bool(false)),
            ]);
            let capabilities = match vendor {
                Vendor::Grok => object([("fs", fs), ("terminal", Json::Bool(false))]),
                Vendor::OpenCode => object([
                    ("fs", fs),
                    ("_meta", object([("terminal-auth", Json::Bool(true))])),
                ]),
                _ => unreachable!(),
            };
            let mut fields = BTreeMap::from([
                (
                    JsonString::from_str("protocolVersion"),
                    Json::Number("1".to_owned()),
                ),
                (JsonString::from_str("clientCapabilities"), capabilities),
            ]);
            if vendor == Vendor::Grok {
                fields.insert(
                    JsonString::from_str("_meta"),
                    object([
                        ("clientType", string("gogoke")),
                        ("clientVersion", string(client_version)),
                    ]),
                );
            } else {
                fields.insert(
                    JsonString::from_str("clientInfo"),
                    object([
                        ("name", string("gogoke")),
                        ("version", string(client_version)),
                    ]),
                );
            }
            ("initialize", Json::Object(fields), false)
        }
        AcpCommand::SessionNew { cwd } => {
            nonempty(cwd, "cwd")?;
            let mut fields = BTreeMap::from([
                (JsonString::from_str("cwd"), string(cwd)),
                (JsonString::from_str("mcpServers"), Json::Array(Vec::new())),
            ]);
            if vendor == Vendor::Grok {
                fields.insert(
                    JsonString::from_str("_meta"),
                    object([("yoloMode", Json::Bool(false))]),
                );
            }
            ("session/new", Json::Object(fields), false)
        }
        AcpCommand::SessionLoad {
            session_id,
            cwd,
            advertised,
        } => {
            if vendor != Vendor::Grok || !advertised {
                return Err(EncodeError::Unsupported(
                    "session/load not established for this process",
                ));
            }
            nonempty(session_id, "session id")?;
            nonempty(cwd, "cwd")?;
            (
                "session/load",
                object([
                    ("sessionId", string(session_id)),
                    ("cwd", string(cwd)),
                    ("mcpServers", Json::Array(Vec::new())),
                ]),
                false,
            )
        }
        AcpCommand::SessionResume {
            session_id,
            cwd,
            advertised,
        } => {
            if vendor != Vendor::OpenCode || !advertised {
                return Err(EncodeError::Unsupported(
                    "session/resume not established for this process",
                ));
            }
            nonempty(session_id, "session id")?;
            nonempty(cwd, "cwd")?;
            (
                "session/resume",
                object([
                    ("sessionId", string(session_id)),
                    ("cwd", string(cwd)),
                    ("mcpServers", Json::Array(Vec::new())),
                ]),
                false,
            )
        }
        AcpCommand::Prompt { session_id, text } => {
            nonempty(session_id, "session id")?;
            nonempty(text, "prompt text")?;
            (
                "session/prompt",
                object([
                    ("sessionId", string(session_id)),
                    (
                        "prompt",
                        Json::Array(vec![object([
                            ("type", string("text")),
                            ("text", string(text)),
                        ])]),
                    ),
                ]),
                false,
            )
        }
        AcpCommand::SetConfigOption { session_id, config_id, value } => {
            if vendor != Vendor::OpenCode {
                return Err(EncodeError::Unsupported("pinned vendor has no verified ACP config option"));
            }
            nonempty(session_id, "session id")?;
            nonempty(value, "config value")?;
            if !matches!(config_id, "model" | "effort") {
                return Err(EncodeError::Unsupported("only model and effort are verified ACP options"));
            }
            ("session/set_config_option", object([
                ("sessionId", string(session_id)),
                ("configId", string(config_id)),
                ("value", string(value)),
            ]), false)
        }
        AcpCommand::Cancel { session_id } => {
            nonempty(session_id, "session id")?;
            (
                "session/cancel",
                object([("sessionId", string(session_id))]),
                true,
            )
        }
        AcpCommand::PermissionResponse => {
            return Err(EncodeError::Unsupported(
                "no frozen ACP permission response encoder",
            ))
        }
        AcpCommand::Steer => return Err(EncodeError::Unsupported("no ACP in-turn steer")),
    };
    if notification {
        if id.is_some() {
            return Err(EncodeError::Invalid("notification must not have id"));
        }
        jsonl(object([
            ("jsonrpc", string("2.0")),
            ("method", string(method)),
            ("params", params),
        ]))
    } else {
        let id = id.ok_or(EncodeError::Invalid("request id missing"))?;
        jsonl(object([
            ("jsonrpc", string("2.0")),
            ("id", rpc_id(id)?),
            ("method", string(method)),
            ("params", params),
        ]))
    }
}

/// Only the user-input shape in the frozen Claude and agy adapters is encoded.
/// There is no supported CLI control-response, in-turn append, or steer here.
pub(crate) fn encode_user_input(vendor: Vendor, text: &str) -> Result<Vec<u8>, EncodeError> {
    nonempty(text, "user input")?;
    match vendor {
        Vendor::Claude => jsonl(object([
            ("type", string("user")),
            (
                "message",
                object([
                    ("role", string("user")),
                    (
                        "content",
                        Json::Array(vec![object([
                            ("type", string("text")),
                            ("text", string(text)),
                        ])]),
                    ),
                ]),
            ),
        ])),
        Vendor::Antigravity => jsonl(object([
            ("event", string("user")),
            ("message", object([("content", string(text))])),
        ])),
        _ => Err(EncodeError::Unsupported("vendor uses ACP session/prompt")),
    }
}

pub(crate) fn encode_claude(command: ClaudeCommand<'_>) -> Result<Vec<u8>, EncodeError> {
    match command {
        ClaudeCommand::Initialize { request_id } => {
            nonempty(request_id, "Claude initialize request id")?;
            jsonl(object([
                ("type", string("control_request")),
                ("request_id", string(request_id)),
                ("request", object([("subtype", string("initialize"))])),
            ]))
        }
        ClaudeCommand::User { uuid, text } => {
            nonempty(uuid, "Claude User UUID")?;
            nonempty(text, "Claude User text")?;
            jsonl(object([
                ("type", string("user")),
                ("uuid", string(uuid)),
                ("parent_tool_use_id", Json::Null),
                ("message", object([
                    ("role", string("user")),
                    ("content", Json::Array(vec![object([
                        ("type", string("text")), ("text", string(text)),
                    ])])),
                ])),
            ]))
        }
    }
}

/// Argument templates only; H supplies the pinned executable, environment,
/// sandbox, cwd, process custody and admission. No login command is launched.
pub(crate) fn launch_args(
    vendor: Vendor,
    resume_id: Option<&str>,
) -> Result<Vec<String>, EncodeError> {
    match vendor {
        Vendor::Claude => Err(EncodeError::Unsupported(
            "Claude launch requires bound model and effort")),
        Vendor::OpenCode if resume_id.is_none() => Ok(vec!["--pure".to_owned(), "acp".to_owned()]),
        Vendor::Grok if resume_id.is_none() => Err(EncodeError::Unsupported(
            "Grok model and effort must come from bound seat settings")),
        Vendor::Antigravity => {
            let mut args = Vec::new();
            if let Some(id) = resume_id {
                nonempty(id, "Antigravity conversation id")?;
                if id.trim() != id {
                    return Err(EncodeError::Invalid(
                        "Antigravity conversation id whitespace",
                    ));
                }
                args.extend(["--conversation".to_owned(), id.to_owned()]);
            }
            args.extend(
                [
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                ]
                .into_iter()
                .map(str::to_owned),
            );
            Ok(args)
        }
        _ => Err(EncodeError::Unsupported(
            "resume is an ACP request after launch",
        )),
    }
}

/// Fixed 2.1.196 help lists --model and --effort, plus --replay-user-messages
/// for streaming JSON and --safe-mode for disabling customizations. H alone
/// must supply these values from the current bound E seat at actual launch.
pub(crate) fn claude_launch_args(model: &str, effort: &str,
    resume_id: Option<&str>) -> Result<Vec<String>, EncodeError> {
    nonempty(model, "Claude model")?;
    if model.trim() != model || model.starts_with('-') {
        return Err(EncodeError::Invalid("Claude model is not a CLI value"));
    }
    if !matches!(effort, "low" | "medium" | "high" | "xhigh" | "max") {
        return Err(EncodeError::Unsupported("effort absent from fixed Claude help"));
    }
    let mut args = vec![
        "--print".to_owned(), "--input-format".to_owned(), "stream-json".to_owned(),
        "--output-format".to_owned(), "stream-json".to_owned(),
        "--verbose".to_owned(), "--replay-user-messages".to_owned(),
        "--safe-mode".to_owned(), "--disallowedTools".to_owned(),
        "Agent,EnterWorktree,ExitWorktree".to_owned(),
        "--model".to_owned(), model.to_owned(),
        "--effort".to_owned(), effort.to_owned(),
    ];
    if let Some(id) = resume_id {
        nonempty(id, "Claude session id")?;
        if id.contains('/') || id.contains('\\') {
            return Err(EncodeError::Invalid("Claude session id contains separator"));
        }
        args.push(format!("--resume={id}"));
    }
    Ok(args)
}

/// Fixed 1.0.41 `agent --help` places model and effort on the parent agent
/// command, before its `stdio` child command. These argv values request the
/// settings; only H's actual launch and provider readback can qualify them.
pub(crate) fn grok_launch_args(model: &str, effort: &str) -> Result<Vec<String>, EncodeError> {
    nonempty(model, "Grok model")?;
    nonempty(effort, "Grok effort")?;
    if model.trim() != model || effort.trim() != effort
        || model.starts_with('-') || effort.starts_with('-') {
        return Err(EncodeError::Invalid("Grok model or effort is not a CLI value"));
    }
    Ok(vec![
        "agent".to_owned(), "--model".to_owned(), model.to_owned(),
        "--reasoning-effort".to_owned(), effort.to_owned(),
        "--no-leader".to_owned(), "stdio".to_owned(),
    ])
}
