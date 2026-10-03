//! Fixed Codex 0.149 app-server JSONL codec, not a v37 authority or receipt.
//! H supplies a live ProcessCustodian frame and its native thread/cwd/model
//! evidence. This module only encodes commands and correlates protocol bytes.

use crate::store::atomic::{AtomicError, Json, JsonString, Parser};
use std::collections::{BTreeMap, BTreeSet};

const MAX_FRAME: usize = 1024 * 1024;
const MAX_DEPTH: usize = 128;
const MAX_SAFE_ID: i64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RpcId {
    Number(i64),
    String(String),
}
impl RpcId {
    pub(crate) fn client(number: u64) -> Result<Self, RpcError> {
        if number == 0 || number > MAX_SAFE_ID as u64 {
            return Err(RpcError::Invalid("client id"));
        }
        Ok(Self::Number(number as i64))
    }
    fn json(&self) -> Result<Json, RpcError> {
        match self {
            Self::Number(value) if (-MAX_SAFE_ID..=MAX_SAFE_ID).contains(value) => {
                Ok(Json::Number(value.to_string()))
            }
            Self::String(value) if nonempty(value) => Ok(s(value)),
            _ => Err(RpcError::Invalid("rpc id")),
        }
    }
}

#[derive(Debug)]
pub(crate) enum RpcError {
    Invalid(&'static str),
    FrameTooLarge,
    PartialFrame,
    WrongId,
    RemoteResponse(Vec<u8>),
    Json(AtomicError),
    Utf8(std::str::Utf8Error),
}
impl From<AtomicError> for RpcError {
    fn from(error: AtomicError) -> Self {
        Self::Json(error)
    }
}
impl From<std::str::Utf8Error> for RpcError {
    fn from(error: std::str::Utf8Error) -> Self {
        Self::Utf8(error)
    }
}

/// The caller must obtain cwd, model, and effort from native H evidence.
/// These are protocol parameters only: no grant, permission tier, or path
/// authority is inferred from successful encoding or a Codex ACK.
#[derive(Debug)]
pub(crate) enum Command {
    Initialize {
        client_version: String,
    },
    InitializeHostTools {
        client_version: String,
    },
    Initialized,
    ConfigRead {
        cwd: String,
    },
    FeatureList {
        thread_id: String,
        cursor: Option<String>,
    },
    ThreadStart {
        cwd: String,
        model: String,
    },
    ThreadStartHostTools {
        cwd: String,
        model: String,
    },
    ThreadResume {
        thread_id: String,
        cwd: String,
        model: String,
    },
    TurnStart {
        thread_id: String,
        cwd: String,
        model: String,
        effort: String,
        text: String,
        // Some comes only from H's verified LPAC tier. None retains an exact
        // historical command for recovery; new model writes never use None.
        network_access: Option<bool>,
    },
    TurnSteer {
        thread_id: String,
        expected_turn_id: String,
        text: String,
    },
    TurnInterrupt {
        thread_id: String,
        turn_id: String,
    },
    /// Appends one user-role message to model-visible history without a turn.
    AppendWithoutTurn {
        thread_id: String,
        text: String,
    },
    /// The empty RPC response means submitted; completion is a later item event.
    ThreadCompactStart {
        thread_id: String,
    },
    QuestionAnswer {
        request_id: RpcId,
        answers: BTreeMap<String, Vec<String>>,
    },
    DynamicToolResponse {
        request_id: RpcId,
        text: String,
        success: bool,
    },
}

impl Command {
    fn method(&self) -> Option<&'static str> {
        match self {
            Self::Initialize { .. } | Self::InitializeHostTools { .. } => Some("initialize"),
            Self::Initialized => Some("initialized"),
            Self::ConfigRead { .. } => Some("config/read"),
            Self::FeatureList { .. } => Some("experimentalFeature/list"),
            Self::ThreadStart { .. } | Self::ThreadStartHostTools { .. } => Some("thread/start"),
            Self::ThreadResume { .. } => Some("thread/resume"),
            Self::TurnStart { .. } => Some("turn/start"),
            Self::TurnSteer { .. } => Some("turn/steer"),
            Self::TurnInterrupt { .. } => Some("turn/interrupt"),
            Self::AppendWithoutTurn { .. } => Some("thread/inject_items"),
            Self::ThreadCompactStart { .. } => Some("thread/compact/start"),
            Self::QuestionAnswer { .. } | Self::DynamicToolResponse { .. } => None,
        }
    }

    pub(crate) fn encode(&self, id: Option<&RpcId>) -> Result<Vec<u8>, RpcError> {
        let value = match self {
            Self::DynamicToolResponse { request_id, text, success } => {
                if id.is_some() || text.is_empty() {
                    return Err(RpcError::Invalid("dynamic tool response"));
                }
                obj([("id", request_id.json()?), ("result", obj([
                    ("contentItems", Json::Array(vec![obj([
                        ("type", s("inputText")), ("text", s(text)),
                    ])])),
                    ("success", Json::Bool(*success)),
                ]))])
            }
            Self::QuestionAnswer {
                request_id,
                answers,
            } => {
                if id.is_some() || answers.is_empty() {
                    return Err(RpcError::Invalid("question answer"));
                }
                let mut entries = BTreeMap::new();
                for (question, values) in answers {
                    if !nonempty(question)
                        || values.is_empty()
                        || values.iter().any(|value| !nonempty(value))
                    {
                        return Err(RpcError::Invalid("question answers"));
                    }
                    entries.insert(
                        k(question),
                        obj([(
                            "answers",
                            Json::Array(values.iter().map(|v| s(v)).collect()),
                        )]),
                    );
                }
                obj([
                    ("id", request_id.json()?),
                    (
                        "result",
                        Json::Object(BTreeMap::from([(k("answers"), Json::Object(entries))])),
                    ),
                ])
            }
            other => {
                let params = other.params()?;
                let method = other.method().ok_or(RpcError::Invalid("method"))?;
                let mut fields = BTreeMap::from([(k("method"), s(method)), (k("params"), params)]);
                match (other, id) {
                    (Self::Initialized, None) => {}
                    (Self::Initialized, Some(_)) | (_, None) => {
                        return Err(RpcError::Invalid("request id"))
                    }
                    (_, Some(id)) => {
                        fields.insert(k("id"), id.json()?);
                    }
                }
                Json::Object(fields)
            }
        };
        let mut bytes = value.canonical().into_bytes();
        bytes.push(b'\n');
        if bytes.len() > MAX_FRAME {
            return Err(RpcError::FrameTooLarge);
        }
        Ok(bytes)
    }

    fn params(&self) -> Result<Json, RpcError> {
        match self {
            Self::Initialize { client_version } | Self::InitializeHostTools { client_version } => {
                required(client_version, "client version")?;
                return Ok(obj([
                    (
                        "clientInfo",
                        obj([("name", s("gogoke")), ("version", s(client_version))]),
                    ),
                    ("capabilities", if matches!(self, Self::InitializeHostTools { .. }) {
                        obj([("experimentalApi", Json::Bool(true))])
                    } else { obj([]) }),
                ]));
            }
            Self::Initialized => return Ok(obj([])),
            Self::ConfigRead { cwd } => {
                required(cwd, "cwd")?;
                return Ok(obj([("cwd", s(cwd)), ("includeLayers", Json::Bool(true))]));
            }
            Self::FeatureList {thread_id,cursor} => {
                required(thread_id,"feature thread id")?;
                if let Some(cursor)=cursor {required(cursor,"feature cursor")?;}
                return Ok(obj([
                    ("threadId",s(thread_id)),
                    ("cursor",cursor.as_deref().map_or(Json::Null,s)),
                    ("limit",Json::Number("32".into())),
                ]));
            }
            Self::ThreadStart { cwd, model } => {
                required(cwd, "cwd")?;
                required(model, "model")?;
                return Ok(obj([
                    ("cwd", s(cwd)),
                    ("model", s(model)),
                    ("ephemeral", Json::Bool(false)),
                    ("config", memory_off()),
                ]));
            }
            Self::ThreadStartHostTools { cwd, model } => {
                let Json::Object(mut fields) = (Self::ThreadStart {
                    cwd: cwd.clone(), model: model.clone(),
                }).params()? else { return Err(RpcError::Invalid("host tool thread params")); };
                fields.insert(k("dynamicTools"), host_tools());
                return Ok(Json::Object(fields));
            }
            Self::ThreadResume {
                thread_id,
                cwd,
                model,
            } => {
                required(thread_id, "thread id")?;
                required(cwd, "cwd")?;
                required(model, "model")?;
                return Ok(obj([
                    ("threadId", s(thread_id)),
                    ("cwd", s(cwd)),
                    ("model", s(model)),
                    ("config", memory_off()),
                ]));
            }
            Self::TurnStart {
                thread_id,
                cwd,
                model,
                effort,
                text,
                network_access,
            } => {
                for (value, field) in [
                    (thread_id, "thread id"),
                    (cwd, "cwd"),
                    (model, "model"),
                    (effort, "effort"),
                    (text, "text"),
                ] {
                    required(value, field)?;
                }
                let mut params = BTreeMap::from([
                    (k("threadId"), s(thread_id)),
                    (k("cwd"), s(cwd)),
                    (k("model"), s(model)),
                    (k("effort"), s(effort)),
                    (k("input"), text_input(text)),
                ]);
                if let Some(network_access) = network_access {
                    params.insert(k("approvalPolicy"), s("never"));
                    params.insert(k("sandboxPolicy"), obj([
                        ("type", s("externalSandbox")),
                        ("networkAccess", s(if *network_access { "enabled" } else { "restricted" })),
                    ]));
                }
                return Ok(Json::Object(params));
            }
            Self::TurnSteer {
                thread_id,
                expected_turn_id,
                text,
            } => {
                required(thread_id, "thread id")?;
                required(expected_turn_id, "turn id")?;
                required(text, "text")?;
                return Ok(obj([
                    ("threadId", s(thread_id)),
                    ("expectedTurnId", s(expected_turn_id)),
                    ("input", text_input(text)),
                ]));
            }
            Self::TurnInterrupt { thread_id, turn_id } => {
                required(thread_id, "thread id")?;
                required(turn_id, "turn id")?;
                return Ok(obj([("threadId", s(thread_id)), ("turnId", s(turn_id))]));
            }
            Self::AppendWithoutTurn { thread_id, text } => {
                required(thread_id, "thread id")?;
                required(text, "text")?;
                return Ok(obj([
                    ("threadId", s(thread_id)),
                    ("items", Json::Array(vec![obj([
                        ("type", s("message")),
                        ("role", s("user")),
                        ("content", Json::Array(vec![obj([
                            ("type", s("input_text")),
                            ("text", s(text)),
                        ])])),
                    ])])),
                ]));
            }
            Self::ThreadCompactStart { thread_id } => {
                required(thread_id, "thread id")?;
                return Ok(obj([("threadId", s(thread_id))]));
            }
            Self::QuestionAnswer { .. } | Self::DynamicToolResponse { .. } => return Err(RpcError::Invalid("response params")),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum TurnStatus {
    InProgress,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug)]
pub(crate) struct Question {
    pub(crate) id: String,
    pub(crate) header: String,
    pub(crate) question: String,
    pub(crate) is_other: bool,
    pub(crate) is_secret: bool,
    pub(crate) options: Option<Vec<(String, String)>>,
}

#[derive(Debug)]
pub(crate) struct QuestionCard {
    pub(crate) request_id: RpcId,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) item_id: String,
    pub(crate) auto_resolution_ms: Option<u64>,
    pub(crate) is_blocking: Option<bool>,
    pub(crate) questions: Vec<Question>,
}
impl QuestionCard {
    pub(crate) fn answer(
        &self,
        answers: BTreeMap<String, Vec<String>>,
    ) -> Result<Command, RpcError> {
        let expected: BTreeSet<_> = self
            .questions
            .iter()
            .map(|question| question.id.as_str())
            .collect();
        if expected.len() != answers.len()
            || answers.keys().any(|id| !expected.contains(id.as_str()))
            || answers
                .values()
                .any(|values| values.is_empty() || values.iter().any(|value| !nonempty(value)))
        {
            return Err(RpcError::Invalid("question answer ids"));
        }
        Ok(Command::QuestionAnswer {
            request_id: self.request_id.clone(),
            answers,
        })
    }
}

/// Responses are correlated against the one H-journaled in-flight command.
/// Notifications and unknown server requests are raw events, never authority.
#[derive(Debug)]
pub(crate) enum Reply {
    Initialized {
        id: RpcId,
    },
    MemoryOff {
        id: RpcId,
        cwd: String,
    },
    Thread {
        id: RpcId,
        thread_id: String,
        cwd: String,
    },
    Turn {
        id: RpcId,
        turn_id: String,
        status: TurnStatus,
    },
    Ack {
        id: RpcId,
    },
    /// Parsed provider observation. H must bind it to current native custody.
    TurnNotification {
        thread_id: String,
        turn_id: String,
        status: TurnStatus,
        raw_frame: Vec<u8>,
    },
    CompactionItem {
        thread_id: String,
        item_id: String,
        raw_frame: Vec<u8>,
    },
    RemoteError {
        id: RpcId,
        raw_frame: Vec<u8>,
    },
    Event {
        method: String,
        raw_frame: Vec<u8>,
    },
    ServerRequest {
        id: RpcId,
        method: String,
        raw_frame: Vec<u8>,
    },
    Question(QuestionCard),
    FeaturePage {
        id: RpcId,
        features: Vec<(String,bool)>,
        next_cursor: Option<String>,
    },
}

/// The fixed CLI's actual server request, distinct from an item lifecycle
/// notification. Its arguments are selections; they never identify a caller.
#[derive(Debug)]
pub(crate) struct DynamicToolCall {
    pub(crate) request_id: RpcId,
    pub(crate) call_id: String,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) tool: String,
    pub(crate) namespace: Option<String>,
    pub(crate) arguments: Json,
}

pub(crate) fn decode_dynamic_tool_call(frame: &[u8]) -> Result<Option<DynamicToolCall>, RpcError> {
    let Json::Object(fields)=Parser::parse(std::str::from_utf8(frame_body(frame)?)?)? else {
        return Err(RpcError::Invalid("dynamic tool request object"));
    };
    let Some(method)=fields.get(&k("method")) else {return Ok(None);};
    if string(method,"method")?!="item/tool/call" {return Ok(None);}
    if fields.contains_key(&k("result")) || fields.contains_key(&k("error"))
        || fields.keys().any(|key| ![k("id"),k("method"),k("params"),k("jsonrpc")].contains(key)) {
        return Err(RpcError::Invalid("dynamic tool request fields"));
    }
    if let Some(version)=fields.get(&k("jsonrpc")) {
        if string(version,"jsonrpc")?!="2.0" {return Err(RpcError::Invalid("jsonrpc"));}
    }
    let request_id=parse_id(fields.get(&k("id")).ok_or(RpcError::Invalid("dynamic tool id"))?)?;
    let Some(Json::Object(params))=fields.get(&k("params")) else {
        return Err(RpcError::Invalid("dynamic tool params"));
    };
    if params.keys().any(|key| ![k("arguments"),k("callId"),k("threadId"),k("turnId"),k("tool"),k("namespace")].contains(key)) {
        return Err(RpcError::Invalid("dynamic tool params fields"));
    }
    let value=|name| params.get(&k(name)).ok_or(RpcError::Invalid("dynamic tool field"));
    let required_string=|name| -> Result<String,RpcError> {
        let value=string(value(name)?,"dynamic tool string")?;
        required(&value,"dynamic tool string")?;
        Ok(value)
    };
    let namespace=match params.get(&k("namespace")) {
        None|Some(Json::Null)=>None,
        Some(value)=>Some(string(value,"dynamic tool namespace")?),
    };
    Ok(Some(DynamicToolCall {request_id,
        call_id:required_string("callId")?,thread_id:required_string("threadId")?,
        turn_id:required_string("turnId")?,tool:required_string("tool")?,namespace,
        arguments:Parser::parse(&value("arguments")?.canonical())?,
    }))
}

pub(crate) fn decode(frame: &[u8], pending: Option<(&RpcId, &Command)>) -> Result<Reply, RpcError> {
    let body = frame_body(frame)?;
    let Json::Object(fields) = Parser::parse(std::str::from_utf8(body)?)? else {
        return Err(RpcError::Invalid("top-level object"));
    };
    if let Some(method_value) = fields.get(&k("method")) {
        if fields.contains_key(&k("result")) || fields.contains_key(&k("error")) {
            return Err(RpcError::Invalid("method/result overlap"));
        }
        let method = string(method_value, "method")?;
        if let Some(id_value) = fields.get(&k("id")) {
            let id = parse_id(id_value)?;
            if method == "item/tool/requestUserInput" {
                return Ok(Reply::Question(parse_question(
                    id,
                    fields.get(&k("params")),
                )?));
            }
            return Ok(Reply::ServerRequest {
                id,
                method,
                raw_frame: frame.to_vec(),
            });
        }
        return notification(method, fields.get(&k("params")), frame);
    }
    let id = parse_id(
        fields
            .get(&k("id"))
            .ok_or(RpcError::Invalid("response id"))?,
    )?;
    let Some((expected, command)) = pending else {
        return Err(RpcError::WrongId);
    };
    if &id != expected {
        return Err(RpcError::WrongId);
    }
    let result = fields.get(&k("result"));
    let error = fields.get(&k("error"));
    if result.is_some() == error.is_some() {
        return Err(RpcError::Invalid("result/error exclusivity"));
    }
    if error.is_some() {
        return Ok(Reply::RemoteError {
            id,
            raw_frame: frame.to_vec(),
        });
    }
    let result = result.expect("exclusive result");
    match command {
        Command::Initialize { .. } | Command::InitializeHostTools { .. } => {
            object(result, "initialize result")?;
            Ok(Reply::Initialized { id })
        }
        Command::ConfigRead { cwd } => {
            let config = object(
                field(object(result, "config/read result")?, "config")?,
                "config",
            )?;
            let features = object(field(config, "features")?, "features")?;
            let memories = object(field(config, "memories")?, "memories")?;
            if !matches!(features.get(&k("memories")), Some(Json::Bool(false)))
                || !matches!(
                    memories.get(&k("generate_memories")),
                    Some(Json::Bool(false))
                )
                || !matches!(memories.get(&k("use_memories")), Some(Json::Bool(false)))
            {
                return Err(RpcError::Invalid("effective memory not disabled"));
            }
            Ok(Reply::MemoryOff {
                id,
                cwd: cwd.clone(),
            })
        }
        Command::ThreadStart { .. } | Command::ThreadStartHostTools { .. } | Command::ThreadResume { .. } => {
            let thread = object(field(object(result, "thread result")?, "thread")?, "thread")?;
            let found = string(field(thread, "id")?, "thread id")?;
            let actual_cwd = string(field(thread, "cwd")?, "thread cwd")?;
            // A path string is a provider observation, not physical identity.
            // The native boundary verifies it against F's held directory.
            if let Command::ThreadResume { thread_id, .. } = command {
                if &found != thread_id {
                    return Err(RpcError::Invalid("thread id mismatch"));
                }
            }
            Ok(Reply::Thread {
                id,
                thread_id: found,
                cwd: actual_cwd,
            })
        }
        Command::TurnStart { .. } => {
            let turn = object(field(object(result, "turn result")?, "turn")?, "turn")?;
            let turn_id = string(field(turn, "id")?, "turn id")?;
            let status = match string(field(turn, "status")?, "turn status")?.as_str() {
                "inProgress" => TurnStatus::InProgress,
                "completed" => TurnStatus::Completed,
                "interrupted" => TurnStatus::Interrupted,
                "failed" => TurnStatus::Failed,
                _ => return Err(RpcError::Invalid("turn status")),
            };
            Ok(Reply::Turn {
                id,
                turn_id,
                status,
            })
        }
        Command::TurnSteer {
            expected_turn_id, ..
        } => {
            let found = string(
                field(object(result, "turn steer result")?, "turnId")?,
                "turn id",
            )?;
            if &found != expected_turn_id {
                return Err(RpcError::Invalid("turn id mismatch"));
            }
            Ok(Reply::Ack { id })
        }
        Command::TurnInterrupt { .. } => {
            if !object(result, "interrupt result")?.is_empty() {
                return Err(RpcError::Invalid("interrupt result"));
            }
            Ok(Reply::Ack { id })
        }
        Command::AppendWithoutTurn { .. } | Command::ThreadCompactStart { .. } => {
            if !object(result, "empty command result")?.is_empty() {
                return Err(RpcError::Invalid("empty command result"));
            }
            Ok(Reply::Ack { id })
        }
        Command::FeatureList {..} => {
            let result=object(result,"feature result")?;
            let Json::Array(data)=field(result,"data")? else {return Err(RpcError::Invalid("feature data"));};
            let mut features=Vec::new();let mut seen=BTreeSet::new();
            for feature in data {
                let feature=object(feature,"feature")?;
                let name=string(field(feature,"name")?,"feature name")?;
                if !seen.insert(name.clone()) {return Err(RpcError::Invalid("duplicate feature name"));}
                let Json::Bool(enabled)=field(feature,"enabled")? else {return Err(RpcError::Invalid("feature enabled"));};
                features.push((name,*enabled));
            }
            let next_cursor=match field(result,"nextCursor")? {
                Json::Null=>None,
                value=>Some(string(value,"feature next cursor")?),
            };
            Ok(Reply::FeaturePage {id,features,next_cursor})
        }
        Command::Initialized | Command::QuestionAnswer { .. } | Command::DynamicToolResponse { .. } => {
            Err(RpcError::Invalid("unexpected response"))
        }
    }
}

/// Reconstruct a thread ID only from the exact native RPC command and the A
/// source frame persisted for its observed response. The command must be our
/// canonical, persistent Work, memory-off thread/start encoding.
pub(crate) fn decode_stored_thread_start(
    command_frame: &[u8],
    response_frame: &[u8],
) -> Result<String, RpcError> {
    let body = frame_body(command_frame)?;
    let Json::Object(fields) = Parser::parse(std::str::from_utf8(body)?)? else {
        return Err(RpcError::Invalid("stored thread command"));
    };
    if fields.len() != 3 || string(field(&fields, "method")?, "method")? != "thread/start" {
        return Err(RpcError::Invalid("stored thread method"));
    }
    let id = parse_id(field(&fields, "id")?)?;
    let params = object(field(&fields, "params")?, "thread params")?;
    let cwd = string(field(params, "cwd")?, "thread cwd")?;
    let model = string(field(params, "model")?, "thread model")?;
    let command = if params.contains_key(&k("dynamicTools")) {
        Command::ThreadStartHostTools { cwd, model }
    } else { Command::ThreadStart { cwd, model } };
    if !stored_thread_command_matches(&command,&id,command_frame)? {
        return Err(RpcError::Invalid("stored thread command mismatch"));
    }
    match decode(response_frame, Some((&id, &command)))? {
        Reply::Thread { thread_id, .. } => Ok(thread_id),
        _ => Err(RpcError::Invalid("stored thread response")),
    }
}

/// Resume proof is the original canonical command bytes plus its matching
/// native response. Recreating a command from later strings is not proof.
pub(crate) fn decode_stored_thread_resume(
    command_frame: &[u8],
    response_frame: &[u8],
) -> Result<String, RpcError> {
    let Json::Object(fields) = Parser::parse(std::str::from_utf8(frame_body(command_frame)?)?)? else {
        return Err(RpcError::Invalid("stored resume command"));
    };
    if fields.len() != 3 || string(field(&fields, "method")?, "method")? != "thread/resume" {
        return Err(RpcError::Invalid("stored resume method"));
    }
    let id = parse_id(field(&fields, "id")?)?;
    let params = object(field(&fields, "params")?, "resume params")?;
    let command = Command::ThreadResume {
        thread_id: string(field(params, "threadId")?, "thread id")?,
        cwd: string(field(params, "cwd")?, "thread cwd")?,
        model: string(field(params, "model")?, "thread model")?,
    };
    if !stored_thread_command_matches(&command,&id,command_frame)? {
        return Err(RpcError::Invalid("stored resume command mismatch"));
    }
    match decode(response_frame, Some((&id, &command)))? {
        Reply::Thread { thread_id, .. } => Ok(thread_id),
        Reply::RemoteError { raw_frame, .. } => Err(RpcError::RemoteResponse(raw_frame)),
        _ => Err(RpcError::Invalid("stored resume response")),
    }
}

/// Historical records keep their exact bytes. Accept only the two encodings
/// this product actually emitted, without rewriting their original history or
/// allowing callers to select a legacy configuration for a new native write.
fn stored_thread_command_matches(command:&Command,id:&RpcId,frame:&[u8])->Result<bool,RpcError> {
    let current=command.encode(Some(id))?;
    if current==frame {return Ok(true);}
    if !matches!(command,Command::ThreadStart{..}|Command::ThreadResume{..}) {return Ok(false);}
    let Json::Object(mut fields)=Parser::parse(std::str::from_utf8(frame_body(&current)?)?)? else {return Err(RpcError::Invalid("native thread command"));};
    let Some(Json::Object(params))=fields.get_mut(&k("params")) else {return Err(RpcError::Invalid("native thread params"));};
    params.insert(k("config"),legacy_memory_off());
    let mut original=Json::Object(fields).canonical().into_bytes();original.push(b'\n');
    Ok(original==frame)
}

/// Recovery decodes the command actually persisted before its native write.
/// Exact re-encoding rejects added fields or another method/input shape.
pub(crate) fn decode_stored_turn_start(frame: &[u8]) -> Result<(RpcId, Command), RpcError> {
    let Json::Object(fields) = Parser::parse(std::str::from_utf8(frame_body(frame)?)?)? else {
        return Err(RpcError::Invalid("stored turn command"));
    };
    if fields.len() != 3 || string(field(&fields, "method")?, "method")? != "turn/start" {
        return Err(RpcError::Invalid("stored turn method"));
    }
    let id = parse_id(field(&fields, "id")?)?;
    let params = object(field(&fields, "params")?, "turn params")?;
    let Json::Array(inputs) = field(params, "input")? else {
        return Err(RpcError::Invalid("stored turn input"));
    };
    if inputs.len() != 1 { return Err(RpcError::Invalid("stored turn input count")); }
    let network_access = match params.get(&k("sandboxPolicy")) {
        None => {
            if params.contains_key(&k("approvalPolicy")) {
                return Err(RpcError::Invalid("stored turn partial permission policy"));
            }
            None
        }
        Some(policy) => {
            if string(field(params, "approvalPolicy")?, "approval policy")? != "never" {
                return Err(RpcError::Invalid("stored turn approval policy"));
            }
            let policy = object(policy, "sandbox policy")?;
            if string(field(policy, "type")?, "sandbox type")? != "externalSandbox" {
                return Err(RpcError::Invalid("stored turn external sandbox"));
            }
            Some(match string(field(policy, "networkAccess")?, "network access")?.as_str() {
                "enabled" => true,
                "restricted" => false,
                _ => return Err(RpcError::Invalid("stored turn network access")),
            })
        }
    };
    let command = Command::TurnStart {
        thread_id: string(field(params, "threadId")?, "thread id")?,
        cwd: string(field(params, "cwd")?, "cwd")?,
        model: string(field(params, "model")?, "model")?,
        effort: string(field(params, "effort")?, "effort")?,
        text: string(field(object(&inputs[0], "text input")?, "text")?, "text")?,
        network_access,
    };
    if command.encode(Some(&id))? != frame {
        return Err(RpcError::Invalid("stored turn command mismatch"));
    }
    Ok((id, command))
}

/// Decode the exact command retained for an append; reject extra fields and
/// a changed role/input shape by re-encoding the production command.
pub(crate) fn decode_stored_append(frame:&[u8])->Result<(RpcId,Command),RpcError> {
    let Json::Object(fields)=Parser::parse(std::str::from_utf8(frame_body(frame)?)?)? else {return Err(RpcError::Invalid("stored append command"));};
    if fields.len()!=3 || string(field(&fields,"method")?,"method")?!="thread/inject_items" {return Err(RpcError::Invalid("stored append method"));}
    let id=parse_id(field(&fields,"id")?)?;
    let params=object(field(&fields,"params")?,"append params")?;
    let Json::Array(items)=field(params,"items")? else {return Err(RpcError::Invalid("stored append items"));};
    if items.len()!=1 {return Err(RpcError::Invalid("stored append item count"));}
    let item=object(&items[0],"append item")?;
    let Json::Array(content)=field(item,"content")? else {return Err(RpcError::Invalid("stored append content"));};
    if content.len()!=1 {return Err(RpcError::Invalid("stored append content count"));}
    let command=Command::AppendWithoutTurn {thread_id:string(field(params,"threadId")?,"thread id")?,
        text:string(field(object(&content[0],"append text")?,"text")?,"text")?};
    if command.encode(Some(&id))?!=frame {return Err(RpcError::Invalid("stored append command mismatch"));}
    Ok((id,command))
}

fn parse_question(id: RpcId, params: Option<&Json>) -> Result<QuestionCard, RpcError> {
    let params = object(
        params.ok_or(RpcError::Invalid("question params"))?,
        "question params",
    )?;
    let thread_id = string(field(params, "threadId")?, "thread id")?;
    let turn_id = string(field(params, "turnId")?, "turn id")?;
    let item_id = string(field(params, "itemId")?, "item id")?;
    let is_blocking=match params.get(&k("isBlocking")) {
        None|Some(Json::Null)=>None,
        Some(Json::Bool(value))=>Some(*value),
        _=>return Err(RpcError::Invalid("isBlocking")),
    };
    let auto_resolution_ms = match params.get(&k("autoResolutionMs")) {
        None | Some(Json::Null) => None,
        Some(Json::Number(value)) => {
            let value = value
                .parse::<u64>()
                .map_err(|_| RpcError::Invalid("autoResolutionMs"))?;
            if value > MAX_SAFE_ID as u64 {
                return Err(RpcError::Invalid("autoResolutionMs"));
            }
            Some(value)
        }
        _ => return Err(RpcError::Invalid("autoResolutionMs")),
    };
    let Json::Array(entries) = field(params, "questions")? else {
        return Err(RpcError::Invalid("questions"));
    };
    if entries.is_empty() {
        return Err(RpcError::Invalid("questions"));
    }
    let mut seen = BTreeSet::new();
    let mut questions = Vec::new();
    for entry in entries {
        let entry = object(entry, "question")?;
        let id = string(field(entry, "id")?, "question id")?;
        if !seen.insert(id.clone()) {
            return Err(RpcError::Invalid("duplicate question id"));
        }
        let header = string(field(entry, "header")?, "question header")?;
        let question = string(field(entry, "question")?, "question text")?;
        let is_other = optional_bool(entry, "isOther")?;
        let is_secret = optional_bool(entry, "isSecret")?;
        let options = match entry.get(&k("options")) {
            None | Some(Json::Null) => None,
            Some(Json::Array(options)) => Some(
                options
                    .iter()
                    .map(|option| {
                        let option = object(option, "option")?;
                        Ok((
                            string(field(option, "label")?, "option label")?,
                            string(field(option, "description")?, "option description")?,
                        ))
                    })
                    .collect::<Result<Vec<_>, RpcError>>()?,
            ),
            _ => return Err(RpcError::Invalid("options")),
        };
        questions.push(Question {
            id,
            header,
            question,
            is_other,
            is_secret,
            options,
        });
    }
    Ok(QuestionCard {
        request_id: id,
        thread_id,
        turn_id,
        item_id,
        auto_resolution_ms,
        is_blocking,
        questions,
    })
}

fn notification(method: String, params: Option<&Json>, raw: &[u8]) -> Result<Reply, RpcError> {
    if method == "turn/started" || method == "turn/completed" {
        let params = object(
            params.ok_or(RpcError::Invalid("turn notification params"))?,
            "turn notification",
        )?;
        let thread_id = string(field(params, "threadId")?, "notification thread id")?;
        let turn = object(field(params, "turn")?, "notification turn")?;
        let turn_id = string(field(turn, "id")?, "notification turn id")?;
        let status = if method == "turn/started" {
            TurnStatus::InProgress
        } else {
            match string(field(turn, "status")?, "terminal status")?.as_str() {
                "completed" => TurnStatus::Completed,
                "interrupted" => TurnStatus::Interrupted,
                "failed" => TurnStatus::Failed,
                _ => return Err(RpcError::Invalid("terminal status")),
            }
        };
        return Ok(Reply::TurnNotification {
            thread_id,
            turn_id,
            status,
            raw_frame: raw.to_vec(),
        });
    }
    if method == "item/completed" {
        let params = object(
            params.ok_or(RpcError::Invalid("item notification params"))?,
            "item notification",
        )?;
        let item = object(field(params, "item")?, "item")?;
        if matches!(item.get(&k("type")),Some(Json::String(value))
            if value.to_well_formed_string().as_deref()==Some("contextCompaction"))
        {
            return Ok(Reply::CompactionItem {
                thread_id: string(field(params, "threadId")?, "item thread id")?,
                item_id: string(field(item, "id")?, "item id")?,
                raw_frame: raw.to_vec(),
            });
        }
    }
    Ok(Reply::Event {
        method,
        raw_frame: raw.to_vec(),
    })
}

fn frame_body(frame: &[u8]) -> Result<&[u8], RpcError> {
    if frame.len() > MAX_FRAME {
        return Err(RpcError::FrameTooLarge);
    }
    if frame.len() < 2 || frame.last() != Some(&b'\n') || frame[..frame.len() - 1].contains(&b'\n')
    {
        return Err(RpcError::PartialFrame);
    }
    let body = if frame.get(frame.len() - 2) == Some(&b'\r') {
        &frame[..frame.len() - 2]
    } else {
        &frame[..frame.len() - 1]
    };
    if body.is_empty() || body.contains(&b'\r') || !depth_ok(body) {
        return Err(RpcError::Invalid("frame shape"));
    }
    Ok(body)
}
fn depth_ok(bytes: &[u8]) -> bool {
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for &byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else if byte == b'"' {
            quoted = true;
        } else if byte == b'{' || byte == b'[' {
            depth += 1;
            if depth > MAX_DEPTH {
                return false;
            }
        } else if byte == b'}' || byte == b']' {
            depth = depth.saturating_sub(1);
        }
    }
    !quoted && depth == 0
}
fn parse_id(value: &Json) -> Result<RpcId, RpcError> {
    match value {
        Json::String(value) => {
            let value = value
                .to_well_formed_string()
                .ok_or(RpcError::Invalid("rpc id Unicode"))?;
            if !nonempty(&value) {
                return Err(RpcError::Invalid("rpc id"));
            }
            Ok(RpcId::String(value))
        }
        Json::Number(value) => {
            let parsed = value
                .parse::<i64>()
                .map_err(|_| RpcError::Invalid("rpc numeric id"))?;
            if !(-MAX_SAFE_ID..=MAX_SAFE_ID).contains(&parsed) {
                return Err(RpcError::Invalid("rpc numeric id"));
            }
            Ok(RpcId::Number(parsed))
        }
        _ => Err(RpcError::Invalid("rpc id")),
    }
}
fn k(value: &str) -> JsonString {
    JsonString::from_str(value)
}
fn s(value: &str) -> Json {
    Json::String(k(value))
}
fn obj<const N: usize>(fields: [(&str, Json); N]) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(key, value)| (k(key), value))
            .collect(),
    )
}
fn nonempty(value: &str) -> bool {
    !value.is_empty() && !value.contains('\0')
}
fn required(value: &str, field: &'static str) -> Result<(), RpcError> {
    if nonempty(value) {
        Ok(())
    } else {
        Err(RpcError::Invalid(field))
    }
}
/// Fixed CLI function-tool shape. The native gateway derives the domain,
/// request identity and caller; none is a model-supplied argument.
fn host_tools() -> Json {
    let schema = obj([
        ("type", s("object")),
        ("additionalProperties", Json::Bool(false)),
        ("required", Json::Array(["operation", "targetId", "expectedRevision", "payload"]
            .into_iter().map(s).collect())),
        ("properties", obj([
            ("operation", obj([("type", s("string"))])),
            ("targetId", obj([("type", s("string"))])),
            ("expectedRevision", obj([("type", Json::Array(vec![s("string"), s("null")]))])),
            ("payload", obj([("type", s("object"))])),
        ])),
    ]);
    Json::Array([
        ("gogoke_seat", "Create, dispatch, tune or read a subordinate seat within the native parent scope."),
        ("gogoke_policy", "Read native permission facts and submit or decide an authorized stage gate."),
        ("gogoke_worktree", "Read, register or merge a host-created worktree within native permission facts."),
        ("gogoke_takeover", "Read a takeover card or consume its already written native answer."),
    ].into_iter().map(|(name, description)| obj([
        ("type", s("function")), ("name", s(name)),
        ("description", s(description)), ("inputSchema", schema.clone()),
    ])).collect())
}

fn memory_off() -> Json {
    // ConfigManager appends these to the process CLI override list. Dotted
    // leaves retain unrelated flags; a whole features table replaces them.
    obj([
        ("features.memories",Json::Bool(false)),
        ("memories.generate_memories",Json::Bool(false)),
        ("memories.use_memories",Json::Bool(false)),
    ])
}
fn legacy_memory_off() -> Json {
    obj([
        ("features", obj([("memories", Json::Bool(false))])),
        (
            "memories",
            obj([
                ("generate_memories", Json::Bool(false)),
                ("use_memories", Json::Bool(false)),
            ]),
        ),
    ])
}
fn text_input(text: &str) -> Json {
    Json::Array(vec![obj([("type", s("text")), ("text", s(text))])])
}
fn object<'a>(
    value: &'a Json,
    field: &'static str,
) -> Result<&'a BTreeMap<JsonString, Json>, RpcError> {
    let Json::Object(fields) = value else {
        return Err(RpcError::Invalid(field));
    };
    Ok(fields)
}
fn field<'a>(
    fields: &'a BTreeMap<JsonString, Json>,
    name: &'static str,
) -> Result<&'a Json, RpcError> {
    fields.get(&k(name)).ok_or(RpcError::Invalid(name))
}
fn string(value: &Json, field: &'static str) -> Result<String, RpcError> {
    let Json::String(value) = value else {
        return Err(RpcError::Invalid(field));
    };
    let value = value
        .to_well_formed_string()
        .ok_or(RpcError::Invalid(field))?;
    required(&value, field)?;
    Ok(value)
}
fn optional_bool(
    fields: &BTreeMap<JsonString, Json>,
    name: &'static str,
) -> Result<bool, RpcError> {
    match fields.get(&k(name)) {
        None => Ok(false),
        Some(Json::Bool(value)) => Ok(*value),
        _ => Err(RpcError::Invalid(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thread_overrides_preserve_process_features_and_exact_old_history() {
        let id=RpcId::Number(2);
        let command=Command::ThreadStart{cwd:"D:/sealed-tree".into(),model:"m".into()};
        let current=command.encode(Some(&id)).unwrap();
        let encoded=std::str::from_utf8(&current).unwrap();
        assert!(encoded.contains("\"features.memories\":false"));
        assert!(encoded.contains("\"memories.generate_memories\":false"));
        assert!(encoded.contains("\"memories.use_memories\":false"));
        assert!(!encoded.contains("\"features\":{"),"a table override must not erase the process's other feature flags");
        let response=b"{\"id\":2,\"result\":{\"thread\":{\"id\":\"thread-a\",\"cwd\":\"D:/sealed-tree\"}}}\n";
        assert_eq!(decode_stored_thread_start(&current,response).unwrap(),"thread-a");
        let original=b"{\"id\":2,\"method\":\"thread/start\",\"params\":{\"config\":{\"features\":{\"memories\":false},\"memories\":{\"generate_memories\":false,\"use_memories\":false}},\"cwd\":\"D:/sealed-tree\",\"ephemeral\":false,\"model\":\"m\"}}\n";
        assert_eq!(decode_stored_thread_start(original,response).unwrap(),"thread-a","original history remains readable byte for byte");
        let wrong=std::str::from_utf8(original).unwrap().replace("\"memories\":false","\"memories\":true");
        assert!(decode_stored_thread_start(wrong.as_bytes(),response).is_err());
        let extra=std::str::from_utf8(original).unwrap().replace("\"features\":{","\"features\":{\"default_mode_request_user_input\":true,");
        assert!(decode_stored_thread_start(extra.as_bytes(),response).is_err());
        let resume=b"{\"id\":2,\"method\":\"thread/resume\",\"params\":{\"config\":{\"features\":{\"memories\":false},\"memories\":{\"generate_memories\":false,\"use_memories\":false}},\"cwd\":\"D:/sealed-tree\",\"model\":\"m\",\"threadId\":\"thread-a\"}}\n";
        assert_eq!(decode_stored_thread_resume(resume,response).unwrap(),"thread-a");
    }

    #[test]
    fn loaded_thread_feature_pages_keep_the_original_rpc_and_opaque_cursor() {
        let id=RpcId::client(51).unwrap();
        let command=Command::FeatureList {thread_id:"threadA".into(),cursor:Some("opaque.page:two".into())};
        assert_eq!(command.encode(Some(&id)).unwrap(),b"{\"id\":51,\"method\":\"experimentalFeature/list\",\"params\":{\"cursor\":\"opaque.page:two\",\"limit\":32,\"threadId\":\"threadA\"}}\n");
        let response=b"{\"id\":51,\"result\":{\"data\":[{\"name\":\"multi_agent_v2\",\"enabled\":false}],\"nextCursor\":\"opaque.page:three\"}}\n";
        match decode(response,Some((&id,&command))).unwrap() {
            Reply::FeaturePage {id:RpcId::Number(51),features,next_cursor}=>{
                assert_eq!(features,vec![("multi_agent_v2".into(),false)]);
                assert_eq!(next_cursor.as_deref(),Some("opaque.page:three"));
            },
            other=>panic!("unexpected feature response: {other:?}"),
        }
        let wrong=RpcId::client(52).unwrap();
        assert!(matches!(decode(response,Some((&wrong,&command))),Err(RpcError::WrongId)));
        assert!(matches!(decode(b"{\"id\":51,\"result\":{\"data\":[{\"name\":\"multi_agent_v2\",\"enabled\":0}],\"nextCursor\":null}}\n",Some((&id,&command))),Err(RpcError::Invalid("feature enabled"))));
        assert!(matches!(decode(b"{\"id\":51,\"result\":{\"data\":[{\"name\":\"a\",\"enabled\":false},{\"name\":\"a\",\"enabled\":true}],\"nextCursor\":null}}\n",Some((&id,&command))),Err(RpcError::Invalid("duplicate feature name"))));
    }

    #[test]
    fn native_question_blocking_is_reported_not_inferred_from_mode() {
        let frame="{\"id\":19,\"method\":\"item/tool/requestUserInput\",\"params\":{\"threadId\":\"threadA\",\"turnId\":\"turnA\",\"itemId\":\"itemA\",\"isBlocking\":false,\"questions\":[{\"id\":\"q\",\"header\":\"Choose\",\"question\":\"Which?\",\"isOther\":true,\"isSecret\":false,\"options\":null}]}}\n";
        for (actual,expected) in [(frame.to_owned(),Some(false)),(frame.replace("\"isBlocking\":false","\"isBlocking\":true"),Some(true)),(frame.replace("\"isBlocking\":false,",""),None)] {
            let Reply::Question(card)=decode(actual.as_bytes(),None).unwrap() else {panic!("original question");};
            assert_eq!(card.is_blocking,expected);
        }
        assert!(matches!(decode(frame.replace("\"isBlocking\":false","\"isBlocking\":0").as_bytes(),None),Err(RpcError::Invalid("isBlocking"))));
    }

    #[test]
    fn stored_resume_requires_original_canonical_command_and_matching_native_response() {
        let id = RpcId::client(19).unwrap();
        let command = Command::ThreadResume {
            thread_id: "thread-a".into(),
            cwd: "D:/sealed-tree".into(),
            model: "m".into(),
        };
        let stored = command.encode(Some(&id)).unwrap();
        let response = b"{\"id\":19,\"result\":{\"thread\":{\"id\":\"thread-a\",\"cwd\":\"D:/sealed-tree\"}}}\n";
        assert_eq!(decode_stored_thread_resume(&stored, response).unwrap(), "thread-a");
        assert!(matches!(decode_stored_thread_resume(&stored,
            b"{\"id\":20,\"result\":{\"thread\":{\"id\":\"thread-a\",\"cwd\":\"D:/sealed-tree\"}}}\n"),
            Err(RpcError::WrongId)));
        assert!(matches!(decode_stored_thread_resume(&stored,
            b"{\"id\":19,\"result\":{\"thread\":{\"id\":\"thread-b\",\"cwd\":\"D:/sealed-tree\"}}}\n"),
            Err(RpcError::Invalid("thread id mismatch"))));
        let altered = String::from_utf8(stored.clone()).unwrap()
            .replace("\"memories.use_memories\":false", "\"memories.use_memories\":true");
        assert_ne!(altered.as_bytes(),stored.as_slice(),"the negative control must change the actual retained command");
        assert!(matches!(decode_stored_thread_resume(altered.as_bytes(), response),
            Err(RpcError::Invalid("stored resume command mismatch"))));
        let mut noncanonical = stored.clone();
        noncanonical.insert(1, b' ');
        assert!(matches!(decode_stored_thread_resume(&noncanonical, response),
            Err(RpcError::Invalid("stored resume command mismatch"))));
        let error = b"{\"id\":19,\"error\":{\"code\":-32600,\"message\":\"original provider detail\"}}\n";
        assert!(matches!(decode_stored_thread_resume(&stored, error),
            Err(RpcError::RemoteResponse(raw)) if raw == error.to_vec()));
    }

    #[test]
    fn compact_and_append_use_fixed_native_shape_and_only_empty_submission_response() {
        let id = RpcId::client(41).unwrap();
        let append = Command::AppendWithoutTurn {
            thread_id: "thread-a".into(),
            text: "later context".into(),
        };
        let original = append.encode(Some(&id)).unwrap();
        let (stored_id,stored)=decode_stored_append(&original).unwrap();
        assert_eq!(stored_id,id);
        assert_eq!(stored.encode(Some(&stored_id)).unwrap(),original);
        let changed=String::from_utf8(original.clone()).unwrap().replace("\"role\":\"user\"","\"role\":\"assistant\"");
        assert!(decode_stored_append(changed.as_bytes()).is_err(),"a different item role cannot use the original append completion");
        assert_eq!(original, b"{\"id\":41,\"method\":\"thread/inject_items\",\"params\":{\"items\":[{\"content\":[{\"text\":\"later context\",\"type\":\"input_text\"}],\"role\":\"user\",\"type\":\"message\"}],\"threadId\":\"thread-a\"}}\n".to_vec());
        assert!(matches!(decode(b"{\"id\":41,\"result\":{}}\n", Some((&id, &append))),
            Ok(Reply::Ack { id: RpcId::Number(41) })));
        assert!(matches!(decode(b"{\"id\":42,\"result\":{}}\n", Some((&id, &append))),
            Err(RpcError::WrongId)));
        assert!(matches!(decode(b"{\"id\":41,\"result\":{\"threadId\":\"thread-b\"}}\n", Some((&id, &append))),
            Err(RpcError::Invalid("empty command result"))));
        assert!(matches!(Command::AppendWithoutTurn { thread_id: "".into(), text: "x".into() }.encode(Some(&id)),
            Err(RpcError::Invalid("thread id"))));
        assert!(matches!(Command::AppendWithoutTurn { thread_id: "thread-a".into(), text: "".into() }.encode(Some(&id)),
            Err(RpcError::Invalid("text"))));
        let remote_error = b"{\"id\":41,\"error\":{\"code\":-32600,\"message\":\"original provider detail\"}}\n";
        match decode(remote_error, Some((&id, &append))).unwrap() {
            Reply::RemoteError { raw_frame, .. } => assert_eq!(raw_frame, remote_error.to_vec()),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(append.encode(Some(&id)).unwrap(), original);

        let compact = Command::ThreadCompactStart { thread_id: "thread-a".into() };
        assert_eq!(compact.encode(Some(&id)).unwrap(),
            b"{\"id\":41,\"method\":\"thread/compact/start\",\"params\":{\"threadId\":\"thread-a\"}}\n".to_vec());
        assert!(matches!(decode(b"{\"id\":41,\"result\":{}}\n", Some((&id, &compact))),
            Ok(Reply::Ack { id: RpcId::Number(41) })));
        assert!(matches!(decode(b"{\"id\":41,\"result\":{\"completed\":true}}\n", Some((&id, &compact))),
            Err(RpcError::Invalid("empty command result"))));
        assert!(matches!(Command::ThreadCompactStart { thread_id: "".into() }.encode(Some(&id)),
            Err(RpcError::Invalid("thread id"))));
    }

    #[test]
    fn fixed_initialize_and_native_thread_turn_fields() {
        let id = RpcId::client(1).unwrap();
        let init = Command::Initialize {
            client_version: "0.1.0".into(),
        };
        let bytes = init.encode(Some(&id)).unwrap();
        assert_eq!(bytes,b"{\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{},\"clientInfo\":{\"name\":\"gogoke\",\"version\":\"0.1.0\"}}}\n".to_vec());
        assert!(matches!(
            decode(
                br#"{"id":1,"result":{"userAgent":"codex/0.149"}}
"#,
                Some((&id, &init))
            )
            .unwrap(),
            Reply::Initialized { .. }
        ));
        assert_eq!(
            Command::Initialized.encode(None).unwrap(),
            b"{\"method\":\"initialized\",\"params\":{}}\n".to_vec()
        );
        let start = Command::ThreadStart {
            cwd: "D:/sealed-tree".into(),
            model: "gpt-6-sol".into(),
        };
        let encoded =
            String::from_utf8(start.encode(Some(&RpcId::client(2).unwrap())).unwrap()).unwrap();
        assert!(encoded.contains("\"ephemeral\":false"));
        assert!(encoded.contains("\"model\":\"gpt-6-sol\""));
        assert!(encoded.contains("\"memories.use_memories\":false"));
        let turn = Command::TurnStart {
            thread_id: "thread-a".into(),
            cwd: "D:/sealed-tree".into(),
            model: "gpt-6-sol".into(),
            effort: "high".into(),
            text: "hello".into(),
            network_access: Some(true),
        };
        let encoded =
            String::from_utf8(turn.encode(Some(&RpcId::client(3).unwrap())).unwrap()).unwrap();
        assert!(encoded.contains("\"effort\":\"high\""));
        assert!(encoded.contains("\"input\":[{\"text\":\"hello\",\"type\":\"text\"}]"));
        assert!(encoded.contains("\"approvalPolicy\":\"never\""));
        assert!(encoded.contains("\"sandboxPolicy\":{\"networkAccess\":\"enabled\",\"type\":\"externalSandbox\"}"));
    }

    #[test]
    fn stored_turn_permission_policy_preserves_exact_history_and_rejects_changes() {
        // Recovery retains a pre-policy command's bytes rather than upgrading
        // an already written vendor command to the current permission policy.
        let legacy = b"{\"id\":9,\"method\":\"turn/start\",\"params\":{\"cwd\":\"sealed-tree\",\"effort\":\"high\",\"input\":[{\"text\":\"go\",\"type\":\"text\"}],\"model\":\"m\",\"threadId\":\"t\"}}\n";
        let (id, command) = decode_stored_turn_start(legacy).unwrap();
        assert!(matches!(&command, Command::TurnStart { network_access: None, .. }));
        assert_eq!(command.encode(Some(&id)).unwrap(), legacy);
        for network_access in [false, true] {
            let command = Command::TurnStart { thread_id: "t".into(), cwd: "sealed-tree".into(),
                model: "m".into(), effort: "high".into(), text: "go".into(),
                network_access: Some(network_access) };
            let frame = command.encode(Some(&id)).unwrap();
            let (recovered_id, recovered) = decode_stored_turn_start(&frame).unwrap();
            assert!(matches!(&recovered, Command::TurnStart { network_access: Some(value), .. }
                if *value == network_access));
            assert_eq!(recovered.encode(Some(&recovered_id)).unwrap(), frame);
            let text = String::from_utf8(frame).unwrap();
            for changed in [
                text.replace("\"approvalPolicy\":\"never\",", ""),
                text.replace("\"approvalPolicy\":\"never\"", "\"approvalPolicy\":\"on-request\""),
                text.replace("\"type\":\"externalSandbox\"", "\"type\":\"dangerFullAccess\""),
                text.replace("\"networkAccess\":", "\"unexpected\":false,\"networkAccess\":"),
                text.replace("\"threadId\":\"t\"", "\"permissions\":\":workspace\",\"threadId\":\"t\""),
            ] {
                assert!(decode_stored_turn_start(changed.as_bytes()).is_err(),
                    "partial, mixed or altered stored policy must not be recovered");
            }
        }
    }

    #[test]
    fn effective_memory_off_requires_three_direct_config_facts() {
        let id = RpcId::client(4).unwrap();
        let read = Command::ConfigRead {
            cwd: "D:/sealed-tree".into(),
        };
        let request = String::from_utf8(read.encode(Some(&id)).unwrap()).unwrap();
        assert!(request.contains("\"includeLayers\":true"));
        let good=b"{\"id\":4,\"result\":{\"config\":{\"features\":{\"memories\":false},\"memories\":{\"generate_memories\":false,\"use_memories\":false}}}}\n";
        assert!(
            matches!(decode(good,Some((&id,&read))).unwrap(),Reply::MemoryOff{cwd,..} if cwd=="D:/sealed-tree")
        );
        let bad=b"{\"id\":4,\"result\":{\"config\":{\"features\":{\"memories\":false},\"memories\":{\"generate_memories\":true,\"use_memories\":false}}}}\n";
        assert!(matches!(
            decode(bad, Some((&id, &read))),
            Err(RpcError::Invalid("effective memory not disabled"))
        ));
    }

    #[test]
    fn response_id_shape_error_and_duplicate_key_fail_closed() {
        let id = RpcId::client(7).unwrap();
        let command = Command::TurnSteer {
            thread_id: "t".into(),
            expected_turn_id: "turn-a".into(),
            text: "x".into(),
        };
        assert!(matches!(
            decode(
                b"{\"id\":8,\"result\":{\"turnId\":\"turn-a\"}}\n",
                Some((&id, &command))
            ),
            Err(RpcError::WrongId)
        ));
        assert!(matches!(
            decode(
                b"{\"id\":7,\"result\":{},\"error\":{}}\n",
                Some((&id, &command))
            ),
            Err(RpcError::Invalid("result/error exclusivity"))
        ));
        assert!(matches!(
            decode(
                b"{\"id\":7,\"id\":7,\"result\":{}}\n",
                Some((&id, &command))
            ),
            Err(RpcError::Json(_))
        ));
        assert!(matches!(
            decode(
                b"{\"id\":7,\"result\":{\"turnId\":\"a\",\"turnId\":\"b\"}}\n",
                Some((&id, &command))
            ),
            Err(RpcError::Json(_))
        ));
        assert!(matches!(
            decode(b"{\"id\":7,\"result\":{}}", Some((&id, &command))),
            Err(RpcError::PartialFrame)
        ));
        let mut oversized = vec![b' '; MAX_FRAME];
        oversized.push(b'\n');
        assert!(matches!(
            decode(&oversized, Some((&id, &command))),
            Err(RpcError::FrameTooLarge)
        ));
        assert!(matches!(
            decode(
                b"{\"id\":7,\"result\":{}}\n{\"id\":7}\n",
                Some((&id, &command))
            ),
            Err(RpcError::PartialFrame)
        ));
        let error = b"{\"id\":7,\"error\":{\"code\":-32001,\"message\":\"original detail\"}}\n";
        match decode(error, Some((&id, &command))).unwrap() {
            Reply::RemoteError { raw_frame, .. } => assert_eq!(raw_frame, error.to_vec()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn thread_turn_and_question_are_correlated_without_authority() {
        let id = RpcId::client(2).unwrap();
        let start = Command::ThreadStart {
            cwd: "D:/sealed-tree".into(),
            model: "m".into(),
        };
        let frame=b"{\"id\":2,\"result\":{\"thread\":{\"id\":\"thread-a\",\"cwd\":\"D:/sealed-tree\"}}}\n";
        let stored=start.encode(Some(&id)).unwrap();
        assert_eq!(decode_stored_thread_start(&stored,frame).unwrap(),"thread-a");
        let changed=String::from_utf8(stored.clone()).unwrap().replace("\"ephemeral\":false","\"ephemeral\":true");
        assert!(matches!(decode_stored_thread_start(changed.as_bytes(),frame),
            Err(RpcError::Invalid("stored thread command mismatch"))));
        assert!(matches!(decode_stored_thread_start(&stored,
            b"{\"id\":3,\"result\":{\"thread\":{\"id\":\"thread-a\",\"cwd\":\"D:/sealed-tree\"}}}\n"),
            Err(RpcError::WrongId)));
        assert!(
            matches!(decode(frame,Some((&id,&start))).unwrap(),Reply::Thread{thread_id,..} if thread_id=="thread-a")
        );
        let turn = Command::TurnStart {
            thread_id: "thread-a".into(),
            cwd: "D:/sealed-tree".into(),
            model: "m".into(),
            effort: "high".into(),
            text: "go".into(),
            network_access: Some(true),
        };
        assert!(matches!(
            decode(
                b"{\"id\":2,\"result\":{\"turn\":{\"id\":\"turn-a\",\"status\":\"inProgress\"}}}\n",
                Some((&id, &turn))
            )
            .unwrap(),
            Reply::Turn {
                status: TurnStatus::InProgress,
                ..
            }
        ));
        let question=b"{\"id\":\"ask-1\",\"method\":\"item/tool/requestUserInput\",\"params\":{\"threadId\":\"thread-a\",\"turnId\":\"turn-a\",\"itemId\":\"item-a\",\"questions\":[{\"id\":\"q1\",\"header\":\"Choice\",\"question\":\"Proceed?\",\"options\":[{\"label\":\"Yes\",\"description\":\"Proceed\"}]}]}}\n";
        let Reply::Question(card) = decode(question, Some((&id, &turn))).unwrap() else {
            panic!("question");
        };
        assert_eq!(card.thread_id, "thread-a");
        assert!(card.answer(BTreeMap::new()).is_err());
        let answer = card
            .answer(BTreeMap::from([("q1".into(), vec!["Yes".into()])]))
            .unwrap();
        let encoded = String::from_utf8(answer.encode(None).unwrap()).unwrap();
        assert!(encoded.contains("\"id\":\"ask-1\""));
        assert!(encoded.contains("\"answers\":{\"q1\":{\"answers\":[\"Yes\"]}}"));
        assert!(
            matches!(decode(b"{\"method\":\"future/new\",\"params\":{\"ok\":true}}\n",None).unwrap(),Reply::Event{method,..} if method=="future/new")
        );
        assert!(
            matches!(decode(b"{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"thread-a\",\"turn\":{\"id\":\"turn-a\",\"status\":\"completed\"}}}\n",None).unwrap(),
            Reply::TurnNotification {thread_id,turn_id,status:TurnStatus::Completed,..}
                if thread_id=="thread-a" && turn_id=="turn-a")
        );
        assert!(
            matches!(decode(b"{\"method\":\"item/completed\",\"params\":{\"threadId\":\"thread-a\",\"item\":{\"type\":\"contextCompaction\",\"id\":\"item-a\"}}}\n",None).unwrap(),
            Reply::CompactionItem {item_id,..} if item_id=="item-a")
        );
    }
}
