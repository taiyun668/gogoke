//! Strict source data for pinned Grok Build 1.0.41 Write permission requests.
//! This parser never supplies host authority or a permission decision.
use super::acp::{self, Observation, RpcId};
use crate::store::atomic::{Json, JsonString};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub(crate) id: RpcId,
    pub(crate) session_id: String,
    pub(crate) tool_call_id: String,
    pub(crate) write_path: Option<String>,
    pub(crate) allow_once: Option<String>,
    pub(crate) reject_once: Option<String>,
}

fn get<'a>(fields: &'a BTreeMap<JsonString, Json>, name: &str) -> Option<&'a Json> {
    fields.get(&JsonString::from_str(name))
}
fn object(value: &Json) -> Option<&BTreeMap<JsonString, Json>> {
    if let Json::Object(fields) = value { Some(fields) } else { None }
}
fn text(value: &Json) -> Option<String> {
    if let Json::String(value) = value { value.to_well_formed_string() } else { None }
}
fn field_text(fields: &BTreeMap<JsonString, Json>, name: &str) -> Option<String> {
    get(fields, name).and_then(text).filter(|value| !value.is_empty())
}
fn names(fields: &BTreeMap<JsonString, Json>, expected: &[&str]) -> bool {
    fields.len() == expected.len() && expected.iter().all(|name| get(fields, name).is_some())
}

pub(crate) fn decode_grok(frame: &[u8]) -> Option<Request> {
    let Observation::PermissionRequest { id, params } = acp::decode(frame, None).ok()? else {
        return None;
    };
    let fields = object(&params)?;
    let session_id = field_text(fields, "sessionId")?;
    let tool = object(get(fields, "toolCall")?)?;
    let tool_call_id = field_text(tool, "toolCallId")?;
    let Json::Array(options) = get(fields, "options")? else { return None };
    let mut allow_once = None;
    let mut reject_once = None;
    let mut fixed_options = true;
    let mut option_ids = std::collections::BTreeSet::new();
    for value in options {
        let option = object(value)?;
        fixed_options &= names(option, &["optionId", "name", "kind"])
            && field_text(option, "name").is_some();
        let id = field_text(option, "optionId")?;
        let kind = field_text(option, "kind")?;
        if !option_ids.insert(id.clone()) { return None; }
        match kind.as_str() {
            "allow_once" if allow_once.is_none() => allow_once = Some(id),
            "reject_once" if reject_once.is_none() => reject_once = Some(id),
            "allow_once" | "reject_once" => return None,
            "allow_always" => {}, // Never selectable by this adapter.
            _ => fixed_options = false,
        }
    }
    // Only the source's fixed, mutually corroborating structured Write paths
    // can be considered for allow_once. All other tool shapes stay deny-only.
    let write_path = (|| {
        if !fixed_options
            || !names(fields, &["sessionId", "toolCall", "options", "_meta"])
            || !names(tool, &["toolCallId", "kind", "title", "rawInput", "_meta"])
            || field_text(tool, "kind")?.as_str() != "edit" { return None; }
        let input = object(get(tool, "rawInput")?)?;
        if !names(input, &["variant", "file_path", "content"])
            || field_text(input, "variant")?.as_str() != "Write"
            || text(get(input, "content")?).is_none() { return None; }
        let path = field_text(input, "file_path")?;
        if field_text(tool, "title")? != format!("Write `{path}`") { return None; }
        let meta = object(get(tool, "_meta")?)?;
        if !names(meta, &["x.ai/tool"]) { return None; }
        let vendor = object(get(meta, "x.ai/tool")?)?;
        if !names(vendor, &["version", "name", "kind", "namespace", "label", "read_only", "input"])
            || !matches!(get(vendor, "version"), Some(Json::Number(value)) if value == "1")
            || field_text(vendor, "name")?.as_str() != "write"
            || field_text(vendor, "kind")?.as_str() != "write"
            || field_text(vendor, "namespace")?.as_str() != "opencode"
            || field_text(vendor, "label")?.as_str() != "Write"
            || !matches!(get(vendor, "read_only"), Some(Json::Bool(false))) { return None; }
        let vendor_input = object(get(vendor, "input")?)?;
        if !names(vendor_input, &["path"]) || field_text(vendor_input, "path")? != path { return None; }
        Some(path)
    })();
    Some(Request { id, session_id, tool_call_id, write_path, allow_once, reject_once })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::session_transport::provider_evidence::commands;

    fn source(id:&str,path:&str,meta_path:&str,kind:&str,variant:&str)->Vec<u8> {
        r#"{"jsonrpc":"2.0","id":@ID@,"method":"session/request_permission","params":{"sessionId":"synthetic-thread","toolCall":{"toolCallId":"synthetic-call","kind":"@KIND@","title":"Write `@PATH@`","rawInput":{"variant":"@VARIANT@","file_path":"@PATH@","content":"synthetic edit"},"_meta":{"x.ai/tool":{"version":1,"name":"write","kind":"write","namespace":"opencode","label":"Write","read_only":false,"input":{"path":"@META_PATH@"}}}},"options":[{"optionId":"always","name":"Always","kind":"allow_always"},{"optionId":"once","name":"Once","kind":"allow_once"},{"optionId":"reject","name":"Reject","kind":"reject_once"}],"_meta":{}}}"#
            .replace("@ID@",id).replace("@PATH@",path)
            .replace("@META_PATH@",meta_path).replace("@KIND@",kind)
            .replace("@VARIANT@",variant).into_bytes()
    }

    #[test]
    fn grok_permission_fixed_write_requires_two_paths_and_typed_rpc_id() {
        let path="C:/synthetic/registered/main.ts";
        let numeric=decode_grok(&source("0",path,path,"edit","Write")).unwrap();
        assert_eq!(numeric.id,RpcId::Number(0));
        assert_eq!(numeric.write_path.as_deref(),Some(path));
        assert_eq!(numeric.allow_once.as_deref(),Some("once"));
        assert_eq!(numeric.reject_once.as_deref(),Some("reject"));
        let response=commands::encode_grok_permission_reply(&numeric.id,numeric.allow_once.as_deref()).unwrap();
        assert!(std::str::from_utf8(&response).unwrap().contains("\"id\":0"));
        assert!(!std::str::from_utf8(&response).unwrap().contains("always"));
        let string=decode_grok(&source("\"0\"",path,path,"edit","Write")).unwrap();
        assert_eq!(string.id,RpcId::String("0".into()));
        assert_ne!(numeric.id,string.id);
    }

    #[test]
    fn grok_permission_divergent_path_or_tool_shape_cannot_qualify_write() {
        let path="C:/synthetic/registered/main.ts";
        for (meta,kind,variant) in [
            ("C:/synthetic/outside/main.ts","edit","Write"),
            (path,"shell","Write"),
            (path,"edit","Shell"),
        ] {
            let request=decode_grok(&source("0",path,meta,kind,variant)).unwrap();
            assert!(request.write_path.is_none());
            assert_eq!(request.reject_once.as_deref(),Some("reject"));
        }
    }

    #[test]
    fn grok_permission_extra_path_and_ambiguous_options_never_select_allow_once() {
        let path="C:/synthetic/registered/main.ts";
        let original=String::from_utf8(source("0",path,path,"edit","Write")).unwrap();
        let multiple=original.replace("\"content\":\"synthetic edit\"",
            "\"content\":\"synthetic edit\",\"other_path\":\"C:/synthetic/outside\"");
        assert!(decode_grok(multiple.as_bytes()).unwrap().write_path.is_none());
        let duplicate=original.replace("\"optionId\":\"reject\",\"name\":\"Reject\",\"kind\":\"reject_once\"",
            "\"optionId\":\"once\",\"name\":\"Reject\",\"kind\":\"reject_once\"");
        assert!(decode_grok(duplicate.as_bytes()).is_none());
        let without_reject=original.replace(",{\"optionId\":\"reject\",\"name\":\"Reject\",\"kind\":\"reject_once\"}","");
        let request=decode_grok(without_reject.as_bytes()).unwrap();
        assert!(request.reject_once.is_none());
        let cancelled=commands::encode_grok_permission_reply(&request.id,None).unwrap();
        assert!(std::str::from_utf8(&cancelled).unwrap().contains("\"outcome\":\"cancelled\""));
    }
}
