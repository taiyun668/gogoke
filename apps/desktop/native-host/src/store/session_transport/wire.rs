use crate::store::atomic::{AtomicError, Json, JsonString, Parser};
use std::collections::BTreeMap;

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_JSON_DEPTH: usize = 128;
const SCHEMA: &str = "gogoke.37.operations.v1";

// Parser::parse builds nested JSON recursively. Check the raw frame before
// entering it; braces inside strings (including escaped quotes) are data.
fn bounded_json_depth(bytes: &[u8]) -> Result<(), V37WireError> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_JSON_DEPTH {
                        return Err(V37WireError::Invalid("JSON depth"));
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum V37WireError {
    Invalid(&'static str),
    Json(AtomicError),
    Utf8(std::str::Utf8Error),
    Revision(std::num::ParseIntError),
}

pub(crate) struct V37Request {
    /// Exact received frame, including field order and whitespace, for the
    /// native request journal's replay comparison.
    pub(crate) raw_bytes: Vec<u8>,
    pub(crate) family: String,
    pub(crate) operation: String,
    pub(crate) request_id: String,
    pub(crate) target_id: String,
    pub(crate) domain_id: String,
    pub(crate) expected_revision: u64,
    pub(crate) payload: BTreeMap<JsonString, Json>,
}

pub(crate) struct V37Receipt {
    /// Exact bytes read from the native adapter. The journal retains these
    /// bytes unchanged; the parsed fields below are only for correlation.
    pub(crate) raw_bytes: Vec<u8>,
    pub(crate) family: String,
    pub(crate) operation: String,
    pub(crate) request_id: String,
    pub(crate) target_id: String,
    pub(crate) status: V37Status,
    pub(crate) previous_revision: u64,
    pub(crate) revision: u64,
    result: BTreeMap<JsonString, Json>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum V37Status {
    Applied,
    Replayed,
    Denied,
    Stale,
    Conflict,
    Unsupported,
    Unknown,
    Failed,
}

impl V37Status {
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Applied => "APPLIED",
            Self::Replayed => "REPLAYED",
            Self::Denied => "DENIED",
            Self::Stale => "STALE",
            Self::Conflict => "CONFLICT",
            Self::Unsupported => "UNSUPPORTED",
            Self::Unknown => "UNKNOWN",
            Self::Failed => "FAILED",
        }
    }
}

pub(crate) fn encode_receipt(
    request: &V37Request,
    status: V37Status,
    previous_revision: u64,
    revision: u64,
    result: BTreeMap<JsonString, Json>,
) -> Vec<u8> {
    let fields = BTreeMap::from([
        (
            JsonString::from_str("schema"),
            Json::String(JsonString::from_str(SCHEMA)),
        ),
        (
            JsonString::from_str("family"),
            Json::String(JsonString::from_str(&request.family)),
        ),
        (
            JsonString::from_str("operation"),
            Json::String(JsonString::from_str(&request.operation)),
        ),
        (
            JsonString::from_str("requestId"),
            Json::String(JsonString::from_str(&request.request_id)),
        ),
        (
            JsonString::from_str("targetId"),
            Json::String(JsonString::from_str(&request.target_id)),
        ),
        (
            JsonString::from_str("status"),
            Json::String(JsonString::from_str(status.wire())),
        ),
        (
            JsonString::from_str("previousRevision"),
            Json::String(JsonString::from_str(&previous_revision.to_string())),
        ),
        (
            JsonString::from_str("revision"),
            Json::String(JsonString::from_str(&revision.to_string())),
        ),
        (JsonString::from_str("result"), Json::Object(result)),
    ]);
    Json::Object(fields).canonical().into_bytes()
}

fn field(
    fields: &mut BTreeMap<JsonString, Json>,
    name: &'static str,
) -> Result<Json, V37WireError> {
    fields
        .remove(&JsonString::from_str(name))
        .ok_or(V37WireError::Invalid(name))
}

fn string(
    fields: &mut BTreeMap<JsonString, Json>,
    name: &'static str,
) -> Result<String, V37WireError> {
    match field(fields, name)? {
        Json::String(value) => value
            .to_well_formed_string()
            .ok_or(V37WireError::Invalid(name)),
        _ => Err(V37WireError::Invalid(name)),
    }
}

fn identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphabetic()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
}

fn operation_admitted(family: &str, operation: &str) -> bool {
    let allowed: &[&str] = match family {
        "K-SESSION" => &[
            "open",
            "resume",
            "stop",
            "send",
            "output-stream",
            "capability-probe",
            "append-without-turn",
            "exit-and-stop-receipt",
            "reconnect",
            "compact",
            "renew-session",
            "admission-reserve",
            "admission-commit",
            "admission-release",
        ],
        "K-LEDGER" => &[
            "record",
            "scoped-query",
            "subscribe",
            "resume-subscription",
            "end-subscription",
        ],
        "K-INBOX" => &[
            "enqueue",
            "edit",
            "cancel",
            "steer",
            "deliver",
            "check-unknown",
            "requeue",
        ],
        "K-QCARD" => &["raise", "answer", "expire", "recover"],
        "K-SIDE" => &[
            "create",
            "resume",
            "archive",
            "restore",
            "delete",
            "pending-delta",
            "read-thread",
        ],
        "K-SEAT" => &[
            "create-from-template",
            "tune",
            "bind-instance",
            "change-instance",
            "reclaim",
            "short-to-long",
            "state-card",
            "takeover-answers",
        ],
        "K-POLICY" => &[
            "call-permission-table",
            "gate-submit",
            "gate-decide",
            "stage-transition",
            "escalate",
            "trigger-register",
            "trigger-recover",
            "trigger-cancel",
        ],
        "K-INSTANCE" => &[
            "register",
            "install-state",
            "login-state",
            "version-and-new-version",
            "repin-after-manual-upgrade",
            "concurrency-input",
            "home-lifecycle",
        ],
        "K-WORKTREE" => &[
            "create",
            "register",
            "classify-single-or-mixed",
            "graph-query",
            "merge",
            "cleanup",
        ],
        "K-UI" => &["read-models", "actions"],
        _ => return false,
    };
    allowed.contains(&operation)
}

/// Decodes the same closed envelope as the TS v37 codec without accepting a
/// caller or grant on the wire. The raw bytes must be retained by the caller
/// for exact replay checks; this parsed view is never a request identity.
pub(crate) fn decode_request(bytes: &[u8]) -> Result<V37Request, V37WireError> {
    if bytes.len() > MAX_BYTES {
        return Err(V37WireError::Invalid("frame size"));
    }
    bounded_json_depth(bytes)?;
    let text = std::str::from_utf8(bytes).map_err(V37WireError::Utf8)?;
    let Json::Object(mut fields) = Parser::parse(text).map_err(V37WireError::Json)? else {
        return Err(V37WireError::Invalid("object"));
    };
    if fields.len() != 8 || string(&mut fields, "schema")? != SCHEMA {
        return Err(V37WireError::Invalid("schema"));
    }
    let family = string(&mut fields, "family")?;
    let operation = string(&mut fields, "operation")?;
    if !operation_admitted(&family, &operation) {
        return Err(V37WireError::Invalid("operation"));
    }
    let request_id = string(&mut fields, "requestId")?;
    let target_id = string(&mut fields, "targetId")?;
    let domain_id = string(&mut fields, "domainId")?;
    if !identifier(&request_id) || !identifier(&target_id) || !identifier(&domain_id) {
        return Err(V37WireError::Invalid("identity"));
    }
    let revision = string(&mut fields, "expectedRevision")?;
    if revision.is_empty()
        || (revision.len() > 1 && revision.starts_with('0'))
        || !revision.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(V37WireError::Invalid("revision"));
    }
    let expected_revision = revision.parse::<u64>().map_err(V37WireError::Revision)?;
    let Json::Object(payload) = field(&mut fields, "payload")? else {
        return Err(V37WireError::Invalid("payload"));
    };
    if !fields.is_empty() {
        return Err(V37WireError::Invalid("extra fields"));
    }
    Ok(V37Request {
        raw_bytes: bytes.to_vec(),
        family,
        operation,
        request_id,
        target_id,
        domain_id,
        expected_revision,
        payload,
    })
}

fn decode_revision(value: String, name: &'static str) -> Result<u64, V37WireError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(V37WireError::Invalid(name));
    }
    value.parse::<u64>().map_err(V37WireError::Revision)
}

fn decode_status(value: String) -> Result<V37Status, V37WireError> {
    match value.as_str() {
        "APPLIED" => Ok(V37Status::Applied),
        "REPLAYED" => Ok(V37Status::Replayed),
        "DENIED" => Ok(V37Status::Denied),
        "STALE" => Ok(V37Status::Stale),
        "CONFLICT" => Ok(V37Status::Conflict),
        "UNSUPPORTED" => Ok(V37Status::Unsupported),
        "UNKNOWN" => Ok(V37Status::Unknown),
        "FAILED" => Ok(V37Status::Failed),
        _ => Err(V37WireError::Invalid("status")),
    }
}

/// Decodes the frozen receipt envelope emitted by the adapter. This parser is
/// separate from request decoding so a request-shaped frame, output event, or
/// model value cannot be used as a completion.
pub(crate) fn decode_receipt(bytes: &[u8]) -> Result<V37Receipt, V37WireError> {
    if bytes.len() > MAX_BYTES {
        return Err(V37WireError::Invalid("frame size"));
    }
    bounded_json_depth(bytes)?;
    let text = std::str::from_utf8(bytes).map_err(V37WireError::Utf8)?;
    let Json::Object(mut fields) = Parser::parse(text).map_err(V37WireError::Json)? else {
        return Err(V37WireError::Invalid("object"));
    };
    if fields.len() != 9 || string(&mut fields, "schema")? != SCHEMA {
        return Err(V37WireError::Invalid("schema"));
    }
    let family = string(&mut fields, "family")?;
    let operation = string(&mut fields, "operation")?;
    if !operation_admitted(&family, &operation) {
        return Err(V37WireError::Invalid("operation"));
    }
    let request_id = string(&mut fields, "requestId")?;
    let target_id = string(&mut fields, "targetId")?;
    if !identifier(&request_id) || !identifier(&target_id) {
        return Err(V37WireError::Invalid("identity"));
    }
    let status = decode_status(string(&mut fields, "status")?)?;
    let previous_revision =
        decode_revision(string(&mut fields, "previousRevision")?, "previousRevision")?;
    let revision = decode_revision(string(&mut fields, "revision")?, "revision")?;
    let Json::Object(result) = field(&mut fields, "result")? else {
        return Err(V37WireError::Invalid("result"));
    };
    if !fields.is_empty() {
        return Err(V37WireError::Invalid("extra fields"));
    }
    Ok(V37Receipt {
        raw_bytes: bytes.to_vec(),
        family,
        operation,
        request_id,
        target_id,
        status,
        previous_revision,
        revision,
        result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"schema":"gogoke.37.operations.v1","family":"K-INSTANCE","operation":"register","requestId":"registerA","targetId":"instanceA","domainId":"global","expectedRevision":"0","payload":{"driverId":"codex"}}"#;

    #[test]
    fn exact_envelope_and_closed_operation_are_decoded() {
        let request = decode_request(GOOD.as_bytes()).expect("decode");
        assert_eq!(request.family, "K-INSTANCE");
        assert_eq!(request.operation, "register");
        assert_eq!(request.request_id, "registerA");
        assert_eq!(request.domain_id, "global");
        assert_eq!(request.expected_revision, 0);
        assert_eq!(request.raw_bytes, GOOD.as_bytes());
        assert_eq!(request.payload.len(), 1);
        let receipt = encode_receipt(&request, V37Status::Applied, 0, 1, BTreeMap::new());
        assert_eq!(
            std::str::from_utf8(&receipt).unwrap(),
            r#"{"family":"K-INSTANCE","operation":"register","previousRevision":"0","requestId":"registerA","result":{},"revision":"1","schema":"gogoke.37.operations.v1","status":"APPLIED","targetId":"instanceA"}"#
        );
    }

    #[test]
    fn caller_forgery_unknown_operation_and_duplicate_keys_fail() {
        for raw in [
            GOOD.replace("\"payload\":", "\"caller\":\"owner\",\"payload\":"),
            GOOD.replace("\"register\"", "\"unlisted\""),
            GOOD.replace(
                "\"requestId\":\"registerA\"",
                "\"requestId\":\"registerA\",\"requestId\":\"registerB\"",
            ),
            GOOD.replace("\"expectedRevision\":\"0\"", "\"expectedRevision\":\"00\""),
            GOOD.replace("\"domainId\":\"global\"", "\"domainId\":\"projectA\""),
        ] {
            if raw.contains("\"projectA\"") {
                // A project domain is syntactically valid. F's trusted global
                // authority gate, rather than the wire parser, rejects it.
                assert!(decode_request(raw.as_bytes()).is_ok());
            } else {
                assert!(decode_request(raw.as_bytes()).is_err());
            }
        }
    }

    #[test]
    fn receipt_decoder_retains_exact_bytes_and_closed_status() {
        let raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"sendA","targetId":"sessionA","status":"APPLIED","previousRevision":"3","revision":"4","result":{}}
"#;
        let receipt = decode_receipt(raw).expect("receipt");
        assert_eq!(receipt.raw_bytes, raw);
        assert_eq!(receipt.family, "K-SESSION");
        assert_eq!(receipt.operation, "send");
        assert_eq!(receipt.status, V37Status::Applied);
        assert_eq!(receipt.previous_revision, 3);
        assert_eq!(receipt.revision, 4);
        let unknown = String::from_utf8_lossy(raw).replace("APPLIED", "NOT_A_STATUS");
        assert!(decode_receipt(unknown.as_bytes()).is_err());
    }

    #[test]
    fn recursive_json_is_bounded_before_receipt_parse() {
        let deep = format!(
            "{{\"schema\":\"{SCHEMA}\",\"family\":\"K-SESSION\",\"operation\":\"send\",\"requestId\":\"sendA\",\"targetId\":\"sessionA\",\"status\":\"APPLIED\",\"previousRevision\":\"3\",\"revision\":\"4\",\"result\":{{\"nested\":{} }} }}",
            format!("{}null{}", "[".repeat(MAX_JSON_DEPTH), "]".repeat(MAX_JSON_DEPTH))
        );
        assert!(matches!(
            decode_receipt(deep.as_bytes()),
            Err(V37WireError::Invalid("JSON depth"))
        ));
        let ordinary = deep.replace(
            &format!(
                "{}null{}",
                "[".repeat(MAX_JSON_DEPTH),
                "]".repeat(MAX_JSON_DEPTH)
            ),
            "[null]",
        );
        assert!(decode_receipt(ordinary.as_bytes()).is_ok());
        let quoted = ordinary.replace("[null]", "\"[{\\\"depth\\\":42}]\"");
        assert!(decode_receipt(quoted.as_bytes()).is_ok());
    }
}
