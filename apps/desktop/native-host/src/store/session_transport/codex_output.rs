//! Pure Codex 0.149 JSONL notification to canonical ACP update conversion.
//!
//! Source field shapes are the fixed `rust-v0.149.0` app-server-protocol v2
//! generated types (AgentMessageDeltaNotification, Reasoning*DeltaNotification,
//! Item{Started,Completed}Notification, ThreadItem, Turn*Notification,
//! ThreadStatusChangedNotification, ThreadTokenUsageUpdatedNotification).
//! The caller captures the exact OriginBoundFrame in A before calling this
//! converter, then supplies native event labels/cursors to ledger::EventInput.
//! No frame, request, grant, or second provider history is manufactured here.

use super::codex_rpc::{self, QuestionCard, Reply, TurnStatus};
use crate::store::atomic::{AtomicError, Json, JsonString, Parser};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) enum OutputError {
    Invalid(&'static str),
    WrongThread,
    Codec(codex_rpc::RpcError),
    Json(AtomicError),
    Utf8(std::str::Utf8Error),
}
impl From<codex_rpc::RpcError> for OutputError {
    fn from(value: codex_rpc::RpcError) -> Self { Self::Codec(value) }
}
impl From<AtomicError> for OutputError {
    fn from(value: AtomicError) -> Self { Self::Json(value) }
}
impl From<std::str::Utf8Error> for OutputError {
    fn from(value: std::str::Utf8Error) -> Self { Self::Utf8(value) }
}
type Result<T> = std::result::Result<T, OutputError>;

/// `update_json` is a canonical ACP session/update object ready for the
/// existing ledger EventInput. H supplies event ID, labels, time, and cursor.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct NormalizedUpdate {
    pub(crate) method: String,
    pub(crate) update_json: String,
}

#[derive(Debug)]
pub(crate) enum Output {
    Update(NormalizedUpdate),
    TurnStarted { update: NormalizedUpdate, turn_id: String },
    TurnTerminal { update: NormalizedUpdate, turn_id: String, status: TurnStatus },
    CompactionCompleted {update:NormalizedUpdate,turn_id:String,item_id:String},
    Question(QuestionCard),
    /// Leave A's exact source row pending for explicit native recovery/routing.
    Unhandled { method: String },
}

fn key(value: &str) -> JsonString { JsonString::from_str(value) }
fn text(value: &str) -> Json { Json::String(key(value)) }
fn copy_json(value: &Json) -> Json {
    match value {
        Json::Null => Json::Null,
        Json::Bool(value) => Json::Bool(*value),
        Json::Number(value) => Json::Number(value.clone()),
        Json::String(value) => Json::String(value.clone()),
        Json::Array(values) => Json::Array(values.iter().map(copy_json).collect()),
        Json::Object(fields) => Json::Object(fields.iter().map(|(key,value)|
            (key.clone(),copy_json(value))).collect()),
    }
}
fn object<'a>(value: &'a Json, name: &'static str) -> Result<&'a BTreeMap<JsonString, Json>> {
    match value { Json::Object(fields) => Ok(fields), _ => Err(OutputError::Invalid(name)) }
}
fn field<'a>(fields: &'a BTreeMap<JsonString, Json>, name: &'static str) -> Result<&'a Json> {
    fields.get(&key(name)).ok_or(OutputError::Invalid(name))
}
fn string(fields: &BTreeMap<JsonString, Json>, name: &'static str) -> Result<String> {
    let Json::String(value) = field(fields,name)? else { return Err(OutputError::Invalid(name)); };
    value.to_well_formed_string().filter(|text| !text.is_empty() && !text.contains('\0'))
        .ok_or(OutputError::Invalid(name))
}
fn metadata(method: &str, thread_id: &str, turn_id: Option<&str>, item_id: Option<&str>)
    -> BTreeMap<JsonString, Json> {
    let mut meta=BTreeMap::from([(key("provider"),text("codex")),
        (key("codexMethod"),text(method)),(key("threadId"),text(thread_id))]);
    if let Some(turn_id)=turn_id {meta.insert(key("turnId"),text(turn_id));}
    if let Some(item_id)=item_id {meta.insert(key("itemId"),text(item_id));}
    meta
}
fn update(method: &str, kind: &str, thread_id: &str, turn_id: Option<&str>,
    item_id: Option<&str>, extra: impl IntoIterator<Item=(&'static str,Json)>) -> Output {
    let mut fields=BTreeMap::from([(key("sessionUpdate"),text(kind)),
        (key("_meta"),Json::Object(metadata(method,thread_id,turn_id,item_id)))]);
    for (name,value) in extra {fields.insert(key(name),value);}
    Output::Update(NormalizedUpdate {method:method.to_owned(),
        update_json:Json::Object(fields).canonical()})
}
fn with_meta_field(output: Output, name: &str, value: Json) -> Result<Output> {
    let Output::Update(mut result)=output else {return Err(OutputError::Invalid("update"));};
    let Json::Object(mut fields)=Parser::parse(&result.update_json)? else {
        return Err(OutputError::Invalid("canonical update"));
    };
    let Some(Json::Object(meta))=fields.get_mut(&key("_meta")) else {
        return Err(OutputError::Invalid("update metadata"));
    };
    meta.insert(key(name),value);
    result.update_json=Json::Object(fields).canonical();
    Ok(Output::Update(result))
}
fn text_chunk(method: &str, kind: &str, params: &BTreeMap<JsonString, Json>, thread_id: &str)
    -> Result<Output> {
    let turn_id=string(params,"turnId")?;
    let item_id=string(params,"itemId")?;
    let delta=string_allow_empty(params,"delta")?;
    if method=="item/reasoning/summaryTextDelta" {nonnegative_number(params,"summaryIndex")?;}
    if method=="item/reasoning/textDelta" {nonnegative_number(params,"contentIndex")?;}
    let content=Json::Object(BTreeMap::from([(key("type"),text("text")),
        (key("text"),text(&delta))]));
    Ok(update(method,kind,thread_id,Some(&turn_id),Some(&item_id),[("content",content)]))
}
fn string_allow_empty(fields: &BTreeMap<JsonString, Json>, name: &'static str) -> Result<String> {
    let Json::String(value)=field(fields,name)? else {return Err(OutputError::Invalid(name));};
    value.to_well_formed_string().filter(|text| !text.contains('\0'))
        .ok_or(OutputError::Invalid(name))
}
fn status(value: &str) -> Result<&'static str> {
    match value {
        "inProgress" => Ok("in_progress"),
        "completed" => Ok("completed"),
        "failed" | "declined" => Ok("failed"),
        _ => Err(OutputError::Invalid("tool status")),
    }
}
fn tool_item(method: &str, params: &BTreeMap<JsonString, Json>, thread_id: &str,
    started: bool) -> Result<Output> {
    nonnegative_number(params,if started {"startedAtMs"} else {"completedAtMs"})?;
    let turn_id=string(params,"turnId")?;
    let item=object(field(params,"item")?,"item")?;
    let item_id=string(item,"id")?;
    let item_type=string(item,"type")?;
    if item_type=="contextCompaction" {
        let output=update(method,"session_info_update",thread_id,Some(&turn_id),Some(&item_id),[]);
        let Output::Update(update)=with_meta_field(output,"codexItemType",text(&item_type))? else {return Err(OutputError::Invalid("compaction update"));};
        return Ok(if started {Output::Update(update)} else {Output::CompactionCompleted {update,turn_id,item_id}});
    }
    let (kind,title)=match item_type.as_str() {
        "commandExecution" => {string(item,"command")?; ("execute","Command execution")},
        "fileChange" => {
            if !matches!(field(item,"changes")?,Json::Array(_)) {return Err(OutputError::Invalid("file changes"));}
            ("edit","File change")
        },
        "mcpToolCall" => {string(item,"server")?; string(item,"tool")?;
            field(item,"arguments")?; ("other","MCP tool call")},
        "dynamicToolCall" => {string(item,"tool")?; field(item,"arguments")?;
            ("other","Tool call")},
        _ => return Ok(Output::Unhandled {method:method.to_owned()}),
    };
    let native_status=string(item,"status")?;
    let acp_status=status(&native_status)?;
    if started && acp_status != "in_progress" || !started && acp_status == "in_progress" {
        return Err(OutputError::Invalid("tool lifecycle status"));
    }
    let output=if started {
        update(method,"tool_call",thread_id,Some(&turn_id),Some(&item_id),[
            ("toolCallId",text(&item_id)),("kind",text(kind)),("title",text(title)),
            ("status",text(acp_status))])
    } else {
        let mut extra=vec![("toolCallId",text(&item_id)),("status",text(acp_status))];
        if let Some(raw)=["aggregatedOutput","result","error","contentItems"].iter()
            .filter_map(|name| item.get(&key(name))).find(|raw| !matches!(raw,Json::Null)) {
            extra.push(("rawOutput",copy_json(raw)));
        }
        update(method,"tool_call_update",thread_id,Some(&turn_id),Some(&item_id),extra)
    };
    with_meta_field(output,"codexItemType",text(&item_type))
}
fn tool_output(method: &str, params: &BTreeMap<JsonString, Json>, thread_id: &str,
    field_name: &'static str) -> Result<Output> {
    let turn_id=string(params,"turnId")?;
    let item_id=string(params,"itemId")?;
    let delta=string_allow_empty(params,field_name)?;
    Ok(update(method,"tool_call_update",thread_id,Some(&turn_id),Some(&item_id),[
        ("toolCallId",text(&item_id)),("rawOutput",text(&delta))]))
}
fn nonnegative_number(fields: &BTreeMap<JsonString, Json>, name: &'static str) -> Result<u64> {
    let Json::Number(value)=field(fields,name)? else {return Err(OutputError::Invalid(name));};
    value.parse::<u64>().map_err(|_|OutputError::Invalid(name))
}
fn token_usage(method: &str, params: &BTreeMap<JsonString, Json>, thread_id: &str)
    -> Result<Output> {
    let turn_id=string(params,"turnId")?;
    let usage=object(field(params,"tokenUsage")?,"tokenUsage")?;
    for name in ["total","last"] {
        let breakdown=object(field(usage,name)?,"token usage breakdown")?;
        for metric in ["totalTokens","inputTokens","cachedInputTokens","cacheWriteInputTokens",
            "outputTokens","reasoningOutputTokens"] {nonnegative_number(breakdown,metric)?;}
    }
    match field(usage,"modelContextWindow")? {
        Json::Null => {},
        Json::Number(number) if number.parse::<u64>().is_ok() => {},
        _ => return Err(OutputError::Invalid("modelContextWindow")),
    }
    // ACP usage_update.used means context occupancy. Codex exposes cumulative
    // and last-turn counts, not that quantity; preserve their source meaning.
    let output=update(method,"session_info_update",thread_id,Some(&turn_id),None,[]);
    with_meta_field(output,"codexTokenUsage",copy_json(field(params,"tokenUsage")?))
}
fn turn_event(method: &str, params: &BTreeMap<JsonString, Json>, thread_id: &str,
    reply: Reply) -> Result<Output> {
    let Reply::TurnNotification {turn_id,status,..}=reply else {
        return Err(OutputError::Invalid("turn notification"));
    };
    let turn=object(field(params,"turn")?,"turn")?;
    let actual_status=string(turn,"status")?;
    let expected_status=match &status {
        TurnStatus::InProgress => "inProgress",TurnStatus::Completed => "completed",
        TurnStatus::Interrupted => "interrupted",TurnStatus::Failed => "failed",
    };
    if actual_status != expected_status {return Err(OutputError::Invalid("turn status"));}
    let state=match &status {
        TurnStatus::InProgress => "inProgress",
        TurnStatus::Completed => "completed",
        TurnStatus::Interrupted => "interrupted",
        TurnStatus::Failed => "failed",
    };
    let output=update(method,"session_info_update",thread_id,Some(&turn_id),None,[]);
    let Output::Update(update)=with_meta_field(output,"turnStatus",text(state))? else {
        return Err(OutputError::Invalid("turn update"));
    };
    if method=="turn/completed" {
        if status==TurnStatus::InProgress {return Err(OutputError::Invalid("terminal status"));}
        Ok(Output::TurnTerminal {update,turn_id,status})
    } else if status==TurnStatus::InProgress {
        Ok(Output::TurnStarted {update,turn_id})
    } else {Err(OutputError::Invalid("started status"))}
}

/// Convert one exact provider LF frame. Unknown methods and supported-but-
/// unmapped item types are returned explicitly, so H leaves A pending.
pub(crate) fn normalize(frame: &[u8], expected_thread_id: &str) -> Result<Output> {
    if expected_thread_id.is_empty() || expected_thread_id.contains('\0') {
        return Err(OutputError::Invalid("expected thread"));
    }
    let reply=codex_rpc::decode(frame,None)?;
    if let Reply::Question(card)=reply {
        if card.thread_id != expected_thread_id {return Err(OutputError::WrongThread);}
        return Ok(Output::Question(card));
    }
    let body=frame.strip_suffix(b"\n").ok_or(OutputError::Invalid("LF frame"))?;
    let body=body.strip_suffix(b"\r").unwrap_or(body);
    let Json::Object(envelope)=Parser::parse(std::str::from_utf8(body)?)? else {
        return Err(OutputError::Invalid("notification envelope"));
    };
    let method=string(&envelope,"method")?;
    let mapped=matches!(method.as_str(),
        "item/agentMessage/delta" | "item/reasoning/summaryTextDelta" |
        "item/reasoning/textDelta" | "item/commandExecution/outputDelta" |
        "item/fileChange/outputDelta" | "item/mcpToolCall/progress" |
        "item/started" | "item/completed" | "turn/started" | "turn/completed" |
        "thread/tokenUsage/updated" | "thread/status/changed" | "thread/started");
    let params=match envelope.get(&key("params")) {
        Some(Json::Object(fields)) => fields,
        _ if !mapped => return Ok(Output::Unhandled {method}),
        _ => return Err(OutputError::Invalid("params")),
    };
    let thread_id=if method=="thread/started" {
        string(object(field(params,"thread")?,"thread")?,"id")?
    } else if let Some(value)=params.get(&key("threadId")) {
        let Json::String(value)=value else {return Err(OutputError::Invalid("threadId"));};
        value.to_well_formed_string().ok_or(OutputError::Invalid("threadId"))?
    } else {
        return Ok(Output::Unhandled {method});
    };
    if thread_id != expected_thread_id {return Err(OutputError::WrongThread);}
    match method.as_str() {
        "item/agentMessage/delta" => text_chunk(&method,"agent_message_chunk",params,&thread_id),
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" =>
            text_chunk(&method,"agent_thought_chunk",params,&thread_id),
        "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" =>
            tool_output(&method,params,&thread_id,"delta"),
        "item/mcpToolCall/progress" => tool_output(&method,params,&thread_id,"message"),
        "item/started" => tool_item(&method,params,&thread_id,true),
        "item/completed" => tool_item(&method,params,&thread_id,false),
        "turn/started" | "turn/completed" => turn_event(&method,params,&thread_id,reply),
        "thread/tokenUsage/updated" => token_usage(&method,params,&thread_id),
        "thread/status/changed" => {
            let status=object(field(params,"status")?,"thread status")?;
            let state=string(status,"type")?;
            if !matches!(state.as_str(),"notLoaded"|"idle"|"systemError"|"active") {
                return Err(OutputError::Invalid("thread status"));
            }
            if state=="active" {
                let Json::Array(flags)=field(status,"activeFlags")? else {
                    return Err(OutputError::Invalid("activeFlags"));
                };
                for flag in flags {
                    let Json::String(flag)=flag else {return Err(OutputError::Invalid("active flag"));};
                    if !matches!(flag.to_well_formed_string().as_deref(),
                        Some("waitingOnApproval"|"waitingOnUserInput")) {
                        return Err(OutputError::Invalid("active flag"));
                    }
                }
            }
            let output=update(&method,"session_info_update",&thread_id,None,None,[]);
            with_meta_field(output,"threadStatus",copy_json(field(params,"status")?))
        },
        "thread/started" => {
            let output=update(&method,"session_info_update",&thread_id,None,None,[]);
            with_meta_field(output,"threadStatus",text("started"))
        },
        _ => Ok(Output::Unhandled {method}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_message_thought_and_tool_updates_are_canonical() {
        let message=b"{\"method\":\"item/agentMessage/delta\",\"params\":{\"threadId\":\"t\",\"turnId\":\"v\",\"itemId\":\"i\",\"delta\":\"hello\"}}\n";
        let Output::Update(message)=normalize(message,"t").unwrap() else {panic!("message");};
        assert_eq!(message.update_json,
            "{\"_meta\":{\"codexMethod\":\"item/agentMessage/delta\",\"itemId\":\"i\",\"provider\":\"codex\",\"threadId\":\"t\",\"turnId\":\"v\"},\"content\":{\"text\":\"hello\",\"type\":\"text\"},\"sessionUpdate\":\"agent_message_chunk\"}");
        let thought=b"{\"method\":\"item/reasoning/summaryTextDelta\",\"params\":{\"threadId\":\"t\",\"turnId\":\"v\",\"itemId\":\"r\",\"delta\":\"why\",\"summaryIndex\":0}}\n";
        let Output::Update(thought)=normalize(thought,"t").unwrap() else {panic!("thought");};
        assert!(thought.update_json.contains("agent_thought_chunk"));
        let output=b"{\"method\":\"item/commandExecution/outputDelta\",\"params\":{\"threadId\":\"t\",\"turnId\":\"v\",\"itemId\":\"cmd\",\"delta\":\"done\"}}\n";
        let Output::Update(output)=normalize(output,"t").unwrap() else {panic!("tool output");};
        assert!(output.update_json.contains("tool_call_update"));
        assert!(output.update_json.contains("\"rawOutput\":\"done\""));
        let started=b"{\"method\":\"item/started\",\"params\":{\"threadId\":\"t\",\"turnId\":\"v\",\"startedAtMs\":10,\"item\":{\"type\":\"commandExecution\",\"id\":\"cmd\",\"command\":\"echo hi\",\"status\":\"inProgress\"}}}\n";
        let Output::Update(started)=normalize(started,"t").unwrap() else {panic!("tool start");};
        assert!(started.update_json.contains("\"sessionUpdate\":\"tool_call\""));
        let completed=b"{\"method\":\"item/completed\",\"params\":{\"threadId\":\"t\",\"turnId\":\"v\",\"completedAtMs\":11,\"item\":{\"type\":\"commandExecution\",\"id\":\"cmd\",\"command\":\"echo hi\",\"status\":\"completed\",\"aggregatedOutput\":\"hi\"}}}\n";
        let Output::Update(completed)=normalize(completed,"t").unwrap() else {panic!("tool complete");};
        assert!(completed.update_json.contains("\"status\":\"completed\""));
        assert!(completed.update_json.contains("\"rawOutput\":\"hi\""));
    }
    #[test]
    fn wrong_thread_malformed_and_unknown_never_become_ledger_success() {
        let wrong=b"{\"method\":\"item/agentMessage/delta\",\"params\":{\"threadId\":\"other\",\"turnId\":\"v\",\"itemId\":\"i\",\"delta\":\"x\"}}\n";
        assert!(matches!(normalize(wrong,"t"),Err(OutputError::WrongThread)));
        assert!(normalize(b"{\"method\":\"item/agentMessage/delta\",\"params\":{}}", "t").is_err());
        assert!(normalize(b"{\"method\":\"item/agentMessage/delta\",\"method\":\"warning\",\"params\":{}}\n", "t").is_err());
        let unknown=b"{\"method\":\"future/new\",\"params\":{\"threadId\":\"t\"}}\n";
        assert!(matches!(normalize(unknown,"t").unwrap(),Output::Unhandled{method} if method=="future/new"));
        let unknown_other=b"{\"method\":\"future/new\",\"params\":{\"threadId\":\"other\"}}\n";
        assert!(matches!(normalize(unknown_other,"t"),Err(OutputError::WrongThread)));
    }
    #[test]
    fn turn_terminal_and_question_keep_separate_control_meanings() {
        let started=b"{\"method\":\"turn/started\",\"params\":{\"threadId\":\"t\",\"turn\":{\"id\":\"v\",\"status\":\"inProgress\"}}}\n";
        assert!(matches!(normalize(started,"t").unwrap(),Output::TurnStarted{turn_id,..} if turn_id=="v"));
        let terminal=b"{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"t\",\"turn\":{\"id\":\"v\",\"status\":\"completed\"}}}\n";
        assert!(matches!(normalize(terminal,"t").unwrap(),Output::TurnTerminal{turn_id,status:TurnStatus::Completed,..} if turn_id=="v"));
        let question=b"{\"id\":\"ask\",\"method\":\"item/tool/requestUserInput\",\"params\":{\"threadId\":\"t\",\"turnId\":\"v\",\"itemId\":\"i\",\"questions\":[{\"id\":\"q\",\"header\":\"Choose\",\"question\":\"Proceed?\"}]}}\n";
        assert!(matches!(normalize(question,"t").unwrap(),Output::Question(card) if card.thread_id=="t"));
        assert!(matches!(normalize(question,"other"),Err(OutputError::WrongThread)));
    }
    #[test]
    fn compaction_completion_requires_its_native_item_and_thread() {
        let completed=b"{\"method\":\"item/completed\",\"params\":{\"threadId\":\"t\",\"turnId\":\"v\",\"completedAtMs\":12,\"item\":{\"type\":\"contextCompaction\",\"id\":\"compact-item\"}}}\n";
        let Output::CompactionCompleted {update,turn_id,item_id}=normalize(completed,"t").unwrap() else {panic!("original completion required");};
        assert_eq!(turn_id,"v");assert_eq!(item_id,"compact-item");
        assert!(update.update_json.contains("\"codexItemType\":\"contextCompaction\""));
        assert!(matches!(normalize(completed,"other"),Err(OutputError::WrongThread)));
        let started=String::from_utf8(completed.to_vec()).unwrap().replace("item/completed","item/started").replace("completedAtMs","startedAtMs");
        assert!(matches!(normalize(started.as_bytes(),"t").unwrap(),Output::Update(_)),"submission/start is not completion");
        let invalid=String::from_utf8(completed.to_vec()).unwrap().replace("\"completedAtMs\":12","\"completedAtMs\":-1");
        assert!(normalize(invalid.as_bytes(),"t").is_err());
        assert!(normalize(b"{\"id\":7,\"result\":{}}\n","t").is_err(),"a command ACK cannot normalize into a completed compaction");
    }
    #[test]
    fn usage_preserves_codex_counts_without_inventing_context_occupancy() {
        let usage=b"{\"method\":\"thread/tokenUsage/updated\",\"params\":{\"threadId\":\"t\",\"turnId\":\"v\",\"tokenUsage\":{\"total\":{\"totalTokens\":150,\"inputTokens\":100,\"cachedInputTokens\":10,\"cacheWriteInputTokens\":0,\"outputTokens\":50,\"reasoningOutputTokens\":5},\"last\":{\"totalTokens\":20,\"inputTokens\":12,\"cachedInputTokens\":2,\"cacheWriteInputTokens\":0,\"outputTokens\":8,\"reasoningOutputTokens\":1},\"modelContextWindow\":200000}}}\n";
        let Output::Update(update)=normalize(usage,"t").unwrap() else {panic!("usage");};
        assert!(update.update_json.contains("\"codexTokenUsage\""));
        assert!(update.update_json.contains("\"modelContextWindow\":200000"));
        assert!(update.update_json.contains("\"sessionUpdate\":\"session_info_update\""));
        assert!(!update.update_json.contains("\"used\""));
        let bad=String::from_utf8(usage.to_vec()).unwrap().replace("\"totalTokens\":20","\"totalTokens\":-1");
        assert!(normalize(bad.as_bytes(),"t").is_err());
    }
}
