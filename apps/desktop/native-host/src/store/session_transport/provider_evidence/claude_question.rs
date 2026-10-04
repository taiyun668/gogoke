//! Claude Code 2.1.196 AskUserQuestion control framing only.
//!
//! The caller must bind `raw_frame` to the original A/H source and current
//! turn. Decoding grants no permission; encoding does not write stdin or prove
//! that Claude accepted an answer. The other request_user_dialog channel is
//! deliberately unsupported until its fixed result shape is observed.

use std::collections::{BTreeMap, BTreeSet};

use super::stream_json::{self, ClaudeData};
use crate::store::atomic::{AtomicError, Json, JsonString, Parser};

#[derive(Debug)]
pub(crate) enum QuestionError {
    Invalid(&'static str),
    Unsupported(&'static str),
    Stream(stream_json::StreamJsonError),
    Json(AtomicError),
    Utf8(std::str::Utf8Error),
}

pub(crate) struct ClaudeQuestion {
    raw_frame: Vec<u8>,
    request_id: String,
    tool_use_id: String,
    original_input_json: String,
    questions: Vec<QuestionItem>,
}

impl ClaudeQuestion {
    pub(crate) fn raw_frame(&self) -> &[u8] { &self.raw_frame }
    pub(crate) fn request_id(&self) -> &str { &self.request_id }
    pub(crate) fn tool_use_id(&self) -> &str { &self.tool_use_id }
    /// Complete original input, canonically rendered for readback. The exact
    /// vendor byte order remains available through `raw_frame`.
    pub(crate) fn original_input_json(&self) -> &str { &self.original_input_json }
    pub(crate) fn questions(&self) -> &[QuestionItem] { &self.questions }
}

pub(crate) struct QuestionItem {
    /// Product UI index derived from array order; Claude supplies no per-item ID.
    pub(crate) host_index: usize,
    pub(crate) question: String,
    pub(crate) header: String,
    pub(crate) options: Vec<QuestionOption>,
    pub(crate) multi_select: bool,
}

pub(crate) struct QuestionOption {
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) preview: Option<String>,
}

pub(crate) struct QuestionAnswer<'a> {
    /// Host-derived index into the exact original `questions` array.
    pub(crate) host_index: usize,
    /// An option label or free text, retained without trimming or rewriting.
    pub(crate) answer: &'a str,
}

fn key(value: &str) -> JsonString { JsonString::from_str(value) }
fn string(value: &str) -> Json { Json::String(key(value)) }

fn object<'a>(value: &'a Json, field: &'static str)
    -> Result<&'a BTreeMap<JsonString, Json>, QuestionError> {
    match value { Json::Object(value) => Ok(value), _ => Err(QuestionError::Invalid(field)) }
}

fn required<'a>(fields: &'a BTreeMap<JsonString, Json>, name: &'static str)
    -> Result<&'a Json, QuestionError> {
    fields.get(&key(name)).ok_or(QuestionError::Invalid(name))
}

fn text(fields: &BTreeMap<JsonString, Json>, name: &'static str)
    -> Result<String, QuestionError> {
    match required(fields, name)? {
        Json::String(value) => value.to_well_formed_string()
            .ok_or(QuestionError::Invalid(name)),
        _ => Err(QuestionError::Invalid(name)),
    }
}

fn positive_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value == value.trim() &&
        !value.chars().any(char::is_control)
}

fn visible_text(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains('\0')
}

fn questions(input: &BTreeMap<JsonString, Json>)
    -> Result<Vec<QuestionItem>, QuestionError> {
    if input.contains_key(&key("answers")) {
        // The answer must come from the User later, not a model-supplied field.
        return Err(QuestionError::Invalid("pre-filled answers"));
    }
    let Json::Array(entries) = required(input, "questions")? else {
        return Err(QuestionError::Invalid("questions"));
    };
    if !(1..=4).contains(&entries.len()) {
        return Err(QuestionError::Invalid("question count"));
    }
    let mut seen = BTreeSet::new();
    let mut items = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let fields = object(entry, "question item")?;
        let question = text(fields, "question")?;
        let header = text(fields, "header")?;
        if !visible_text(&question) || !visible_text(&header) ||
            !seen.insert(question.clone()) {
            return Err(QuestionError::Invalid("question text"));
        }
        let multi_select = match required(fields, "multiSelect")? {
            Json::Bool(value) => *value,
            _ => return Err(QuestionError::Invalid("multiSelect")),
        };
        let Json::Array(choices) = required(fields, "options")? else {
            return Err(QuestionError::Invalid("options"));
        };
        if !(2..=4).contains(&choices.len()) {
            return Err(QuestionError::Invalid("option count"));
        }
        let mut options = Vec::with_capacity(choices.len());
        for choice in choices {
            let fields = object(choice, "option")?;
            let label = text(fields, "label")?;
            let description = text(fields, "description")?;
            if !visible_text(&label) || description.contains('\0') {
                return Err(QuestionError::Invalid("option text"));
            }
            let preview = match fields.get(&key("preview")) {
                Some(Json::String(value)) => Some(value.to_well_formed_string()
                    .ok_or(QuestionError::Invalid("preview"))?),
                None => None,
                _ => return Err(QuestionError::Invalid("preview")),
            };
            options.push(QuestionOption { label, description, preview });
        }
        items.push(QuestionItem { host_index: index, question, header, options, multi_select });
    }
    Ok(items)
}

/// Decode one original stdout LF frame. All other `can_use_tool` approvals are
/// unrelated to a question. The fixed SDK also exposes request_user_dialog;
/// its AskUserQuestion result is not qualified for this codec.
pub(crate) fn decode(frame: &[u8]) -> Result<Option<ClaudeQuestion>, QuestionError> {
    if !frame.ends_with(b"\n") {
        return Err(QuestionError::Invalid("original LF frame"));
    }
    let preflight = stream_json::decode_claude_line(frame).map_err(QuestionError::Stream)?;
    let ClaudeData::ControlRequest { .. } = preflight else { return Ok(None); };
    let bytes = frame.strip_suffix(b"\n").ok_or(QuestionError::Invalid("LF"))?;
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    let parsed = Parser::parse(std::str::from_utf8(bytes).map_err(QuestionError::Utf8)?)
        .map_err(QuestionError::Json)?;
    let root = object(&parsed, "control request")?;
    let request = object(required(root, "request")?, "request")?;
    let subtype = text(request, "subtype")?;
    if subtype == "request_user_dialog" {
        if text(request, "dialog_kind")? == "permission_ask_user_question" {
            return Err(QuestionError::Unsupported("AskUserQuestion request_user_dialog result"));
        }
        return Ok(None);
    }
    if subtype != "can_use_tool" { return Ok(None); }
    if text(request, "tool_name")? != "AskUserQuestion" { return Ok(None); }
    let request_id = text(root, "request_id")?;
    let tool_use_id = text(request, "tool_use_id")?;
    if !positive_id(&request_id) || !positive_id(&tool_use_id) {
        return Err(QuestionError::Invalid("request/tool-use identity"));
    }
    let input = required(request, "input")?;
    let input_fields = object(input, "input")?;
    let items = questions(input_fields)?;
    Ok(Some(ClaudeQuestion { raw_frame: frame.to_vec(), request_id,
        tool_use_id, original_input_json: input.canonical(), questions: items }))
}

/// Build the SDK 0.3.196 control response for the exact original request.
/// No provider write, tool completion, or receipt is performed here.
pub(crate) fn encode_answer(question: &ClaudeQuestion,
    answers: &[QuestionAnswer<'_>]) -> Result<Vec<u8>, QuestionError> {
    if answers.len() != question.questions.len() {
        return Err(QuestionError::Invalid("answer count"));
    }
    let mut by_index = vec![None; question.questions.len()];
    for item in answers {
        if item.host_index >= by_index.len() || by_index[item.host_index].is_some() ||
            item.answer.is_empty() || item.answer.contains('\0') {
            return Err(QuestionError::Invalid("answer index/value"));
        }
        by_index[item.host_index] = Some(item.answer);
    }
    let mut answer_fields = BTreeMap::new();
    for (item, answer) in question.questions.iter().zip(by_index) {
        let answer = answer.ok_or(QuestionError::Invalid("missing answer"))?;
        answer_fields.insert(key(&item.question), string(answer));
    }
    if answer_fields.len() != question.questions.len() {
        return Err(QuestionError::Invalid("ambiguous question text"));
    }
    let mut original_input = match Parser::parse(&question.original_input_json)
        .map_err(QuestionError::Json)? {
        Json::Object(fields) => fields,
        _ => return Err(QuestionError::Invalid("original input")),
    };
    if original_input.insert(key("answers"), Json::Object(answer_fields)).is_some() {
        return Err(QuestionError::Invalid("pre-filled answers"));
    }
    let result = Json::Object(BTreeMap::from([
        (key("behavior"), string("allow")),
        (key("updatedInput"), Json::Object(original_input)),
        // The fixed SDK adds this original tool-use identity after invoking
        // canUseTool. The CLI must not associate the answer with another tool.
        (key("toolUseID"), string(&question.tool_use_id)),
    ]));
    let response = Json::Object(BTreeMap::from([
        (key("subtype"), string("success")),
        (key("request_id"), string(&question.request_id)),
        (key("response"), result),
    ]));
    let mut bytes = Json::Object(BTreeMap::from([
        (key("type"), string("control_response")),
        (key("response"), response),
    ])).canonical().into_bytes();
    bytes.push(b'\n');
    Ok(bytes)
}
