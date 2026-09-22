//! B2 CommitDomainRecord: one native immediate transaction for the core
//! object/event/receipt group. This is not a SQL transport.

use super::digest::content_hash;
use super::same_open::{SameOpenError, VerifiedDatabaseConnection};
use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};

#[path = "product_core.rs"]
mod product_core;
pub(super) use product_core::{initialize_product_core_schema, validate_product_core_schema};

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;

fn sqlite_transient() -> Option<unsafe extern "C" fn(*mut c_void)> {
    // SQLITE_TRANSIENT is the documented sentinel -1, not a callable function.
    unsafe { std::mem::transmute(-1isize as *const ()) }
}

const COUNTER_CHECK: &str = "length(counter) BETWEEN 1 AND 20 AND counter NOT GLOB '*[^0-9]*' AND (counter = '0' OR substr(counter, 1, 1) <> '0')";
const STREAM_COUNTER_CHECK: &str = "length(stream_counter) BETWEEN 1 AND 20 AND stream_counter NOT GLOB '*[^0-9]*' AND (stream_counter = '0' OR substr(stream_counter, 1, 1) <> '0')";
pub(super) const JS_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

unsafe extern "C" {
    fn sqlite3_prepare_v2(
        database: *mut c_void,
        sql: *const c_char,
        bytes: c_int,
        statement: *mut *mut c_void,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_bind_text(
        statement: *mut c_void,
        index: c_int,
        value: *const c_char,
        bytes: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_bind_blob(
        statement: *mut c_void,
        index: c_int,
        value: *const c_void,
        bytes: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_bind_int64(statement: *mut c_void, index: c_int, value: i64) -> c_int;
    fn sqlite3_step(statement: *mut c_void) -> c_int;
    fn sqlite3_column_text(statement: *mut c_void, column: c_int) -> *const u8;
    fn sqlite3_column_bytes(statement: *mut c_void, column: c_int) -> c_int;
    fn sqlite3_changes(database: *mut c_void) -> c_int;
    fn sqlite3_finalize(statement: *mut c_void) -> c_int;
    fn sqlite3_errmsg(database: *mut c_void) -> *const c_char;
}

#[derive(Debug)]
pub enum AtomicError {
    InvalidRecord(&'static str),
    NonCanonicalJson(&'static str),
    DurabilityContractFailed(String),
    CounterConflict,
    WriteConflict(&'static str),
    OperationConflict,
    CommitUnknown,
    Sqlite { code: c_int, message: String },
    SameOpen(SameOpenError),
}

impl std::fmt::Display for AtomicError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for AtomicError {}

impl From<SameOpenError> for AtomicError {
    fn from(error: SameOpenError) -> Self {
        Self::SameOpen(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeIdentity {
    pub runtime_instance_id: String,
    pub native_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainRecordInput {
    pub domain_id: String,
    pub object_type: String,
    pub object_id: String,
    pub object_version: String,
    pub object_bytes: Vec<u8>,
    pub native_identity: Option<NativeIdentity>,
    pub event_id: String,
    pub stream_id: String,
    pub expected_previous_counter: Option<String>,
    pub counter: String,
    pub event_type: String,
    pub occurred_at: String,
    pub event_bytes: Vec<u8>,
    pub receipt_id: String,
    pub operation_id: String,
    pub receipt_type: String,
    pub recorded_at: String,
    pub receipt_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainRecordReceipt {
    pub disposition: &'static str,
    pub domain_id: String,
    pub receipt_id: String,
    pub operation_id: String,
    pub event_id: String,
    pub object_hash: String,
    pub event_hash: String,
    pub receipt_hash: String,
    pub operation_fingerprint: String,
}

#[derive(Clone)]
struct PreparedRecord {
    domain_id: String,
    object_type: String,
    object_id: String,
    object_version: String,
    object_bytes: Vec<u8>,
    object_hash: String,
    native_identity: Option<NativeIdentity>,
    event_id: String,
    stream_id: String,
    expected_previous_counter: Option<String>,
    counter: String,
    event_type: String,
    occurred_at: String,
    event_bytes: Vec<u8>,
    event_hash: String,
    receipt_id: String,
    operation_id: String,
    receipt_type: String,
    recorded_at: String,
    receipt_bytes: Vec<u8>,
    receipt_hash: String,
    operation_fingerprint: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct JsonString(Vec<u16>);

impl JsonString {
    pub(super) fn from_str(value: &str) -> Self {
        Self(value.encode_utf16().collect())
    }

    pub(super) fn from_units(value: Vec<u16>) -> Self {
        Self(value)
    }

    pub(super) fn units(&self) -> &[u16] {
        &self.0
    }

    pub(super) fn to_well_formed_string(&self) -> Option<String> {
        char::decode_utf16(self.0.iter().copied())
            .map(|value| value.ok())
            .collect::<Option<String>>()
    }
}

impl From<&str> for JsonString {
    fn from(value: &str) -> Self {
        Self::from_str(value)
    }
}

impl From<String> for JsonString {
    fn from(value: String) -> Self {
        Self::from_str(&value)
    }
}

pub(super) enum Json {
    Null,
    Bool(bool),
    Number(String),
    String(JsonString),
    Array(Vec<Json>),
    Object(BTreeMap<JsonString, Json>),
}

impl Json {
    pub(super) fn canonical(&self) -> String {
        match self {
            Json::Null => "null".to_owned(),
            Json::Bool(true) => "true".to_owned(),
            Json::Bool(false) => "false".to_owned(),
            Json::Number(value) => value.clone(),
            Json::String(value) => json_string_utf16(value),
            Json::Array(values) => {
                let mut output = String::from("[");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push_str(&value.canonical());
                }
                output.push(']');
                output
            }
            Json::Object(fields) => {
                let mut output = String::from("{");
                let mut entries = fields.iter().collect::<Vec<_>>();
                entries.sort_by(|(left, _), (right, _)| left.units().cmp(right.units()));
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push_str(&json_string_utf16(key));
                    output.push(':');
                    output.push_str(&value.canonical());
                }
                output.push('}');
                output
            }
        }
    }
}

pub(super) fn json_string(value: &str) -> String {
    json_string_utf16(&JsonString::from_str(value))
}

pub(super) fn json_string_utf16(value: &JsonString) -> String {
    let mut output = String::from("\"");
    let mut index = 0;
    while index < value.units().len() {
        let unit = value.units()[index];
        if (0xd800..=0xdbff).contains(&unit) {
            if let Some(low) = value.units().get(index + 1).copied() {
                if (0xdc00..=0xdfff).contains(&low) {
                    let scalar =
                        0x10000 + (((u32::from(unit) - 0xd800) << 10) | (u32::from(low) - 0xdc00));
                    output.push(char::from_u32(scalar).expect("valid UTF-16 pair"));
                    index += 2;
                    continue;
                }
            }
            output.push_str(&format!("\\u{unit:04x}"));
            index += 1;
            continue;
        }
        if (0xdc00..=0xdfff).contains(&unit) {
            output.push_str(&format!("\\u{unit:04x}"));
            index += 1;
            continue;
        }
        let character = char::from_u32(u32::from(unit)).expect("non-surrogate UTF-16 unit");
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if (ch as u32) < 0x20 => output.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => output.push(ch),
        }
        index += 1;
    }
    output.push('"');
    output
}

/// Render a finite JavaScript Number using the JSON.stringify decimal form.
/// Rust's Display representation supplies shortest-roundtrip significant digits;
/// this applies ECMAScript's fixed/scientific notation thresholds and exponent
/// spelling to those digits.
pub(super) fn canonical_js_number(value: f64) -> String {
    debug_assert!(value.is_finite());
    let mut buffer = ryu_js::Buffer::new();
    buffer.format_finite(value).to_owned()
}

pub(super) fn valid_js_number(value: f64) -> bool {
    value.is_finite() && (value.fract() != 0.0 || value.abs() <= JS_MAX_SAFE_INTEGER as f64)
}

pub(super) struct Parser<'a> {
    text: &'a str,
    offset: usize,
}

impl<'a> Parser<'a> {
    pub(super) fn parse(text: &'a str) -> Result<Json, AtomicError> {
        let mut parser = Self { text, offset: 0 };
        parser.skip_whitespace();
        let value = parser.parse_value()?;
        parser.skip_whitespace();
        if parser.offset != parser.text.len() {
            return Err(AtomicError::NonCanonicalJson("trailing content"));
        }
        Ok(value)
    }

    fn skip_whitespace(&mut self) {
        while matches!(
            self.text.as_bytes().get(self.offset),
            Some(b' ' | b'\n' | b'\r' | b'\t')
        ) {
            self.offset += 1;
        }
    }

    fn parse_value(&mut self) -> Result<Json, AtomicError> {
        match self.text.as_bytes().get(self.offset) {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(Json::String(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", Json::Bool(true)),
            Some(b'f') => self.parse_literal("false", Json::Bool(false)),
            Some(b'n') => self.parse_literal("null", Json::Null),
            Some(b'0'..=b'9' | b'-') => self.parse_number(),
            _ => Err(AtomicError::NonCanonicalJson("expected a JSON value")),
        }
    }

    // Match the finite JavaScript Number surface used by Gogoke JsonValue.
    // require_canonical_json below rejects tokens that do not equal the
    // ECMAScript JSON.stringify rendering of the parsed number.
    fn parse_number(&mut self) -> Result<Json, AtomicError> {
        let start = self.offset;
        let bytes = self.text.as_bytes();
        if bytes.get(self.offset) == Some(&b'-') {
            self.offset += 1;
        }
        match bytes.get(self.offset) {
            Some(b'0') => {
                self.offset += 1;
                if bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                    return Err(AtomicError::NonCanonicalJson("leading-zero JSON number"));
                }
            }
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                    self.offset += 1;
                }
            }
            _ => return Err(AtomicError::NonCanonicalJson("invalid JSON integer")),
        }
        if bytes.get(self.offset) == Some(&b'.') {
            self.offset += 1;
            let fraction_start = self.offset;
            while bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                self.offset += 1;
            }
            if self.offset == fraction_start {
                return Err(AtomicError::NonCanonicalJson("invalid JSON fraction"));
            }
        }
        if matches!(bytes.get(self.offset), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(bytes.get(self.offset), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            let exponent_start = self.offset;
            while bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                self.offset += 1;
            }
            if self.offset == exponent_start {
                return Err(AtomicError::NonCanonicalJson("invalid JSON exponent"));
            }
        }
        let token = &self.text[start..self.offset];
        let value = token
            .parse::<f64>()
            .map_err(|_| AtomicError::NonCanonicalJson("invalid JSON number"))?;
        if !value.is_finite() {
            return Err(AtomicError::NonCanonicalJson("non-finite JSON number"));
        }
        if !valid_js_number(value) {
            return Err(AtomicError::NonCanonicalJson(
                "JSON integer exceeds JS safe range",
            ));
        }
        Ok(Json::Number(canonical_js_number(value)))
    }

    fn parse_literal(&mut self, literal: &str, value: Json) -> Result<Json, AtomicError> {
        if self.text[self.offset..].starts_with(literal) {
            self.offset += literal.len();
            Ok(value)
        } else {
            Err(AtomicError::NonCanonicalJson("invalid literal"))
        }
    }

    fn parse_object(&mut self) -> Result<Json, AtomicError> {
        self.offset += 1;
        self.skip_whitespace();
        let mut fields = BTreeMap::new();
        if self.text.as_bytes().get(self.offset) == Some(&b'}') {
            self.offset += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            if self.text.as_bytes().get(self.offset) != Some(&b'"') {
                return Err(AtomicError::NonCanonicalJson("expected object key"));
            }
            let key = self.parse_string()?;
            if fields.contains_key(&key) {
                return Err(AtomicError::NonCanonicalJson("duplicate JSON key"));
            }
            self.skip_whitespace();
            if self.text.as_bytes().get(self.offset) != Some(&b':') {
                return Err(AtomicError::NonCanonicalJson("expected colon"));
            }
            self.offset += 1;
            self.skip_whitespace();
            let value = self.parse_value()?;
            fields.insert(key, value);
            self.skip_whitespace();
            match self.text.as_bytes().get(self.offset) {
                Some(b',') => self.offset += 1,
                Some(b'}') => {
                    self.offset += 1;
                    break;
                }
                _ => {
                    return Err(AtomicError::NonCanonicalJson(
                        "expected comma or object end",
                    ))
                }
            }
        }
        Ok(Json::Object(fields))
    }

    fn parse_array(&mut self) -> Result<Json, AtomicError> {
        self.offset += 1;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.text.as_bytes().get(self.offset) == Some(&b']') {
            self.offset += 1;
            return Ok(Json::Array(values));
        }
        loop {
            self.skip_whitespace();
            values.push(self.parse_value()?);
            self.skip_whitespace();
            match self.text.as_bytes().get(self.offset) {
                Some(b',') => self.offset += 1,
                Some(b']') => {
                    self.offset += 1;
                    break;
                }
                _ => return Err(AtomicError::NonCanonicalJson("expected comma or array end")),
            }
        }
        Ok(Json::Array(values))
    }

    fn parse_string(&mut self) -> Result<JsonString, AtomicError> {
        self.offset += 1;
        let mut output = Vec::new();
        let bytes = self.text.as_bytes();
        while let Some(&byte) = bytes.get(self.offset) {
            match byte {
                b'"' => {
                    self.offset += 1;
                    return Ok(JsonString::from_units(output));
                }
                b'\\' => {
                    self.offset += 1;
                    match bytes.get(self.offset) {
                        Some(b'"') => {
                            output.push('"' as u16);
                            self.offset += 1;
                        }
                        Some(b'\\') => {
                            output.push('\\' as u16);
                            self.offset += 1;
                        }
                        Some(b'/') => {
                            output.push('/' as u16);
                            self.offset += 1;
                        }
                        Some(b'b') => {
                            output.push(0x0008);
                            self.offset += 1;
                        }
                        Some(b'f') => {
                            output.push(0x000c);
                            self.offset += 1;
                        }
                        Some(b'n') => {
                            output.push('\n' as u16);
                            self.offset += 1;
                        }
                        Some(b'r') => {
                            output.push('\r' as u16);
                            self.offset += 1;
                        }
                        Some(b't') => {
                            output.push('\t' as u16);
                            self.offset += 1;
                        }
                        Some(b'u') => {
                            self.offset += 1;
                            output.push(parse_hex_quad(bytes, &mut self.offset)?);
                        }
                        _ => return Err(AtomicError::NonCanonicalJson("invalid string escape")),
                    }
                }
                b if b < 0x20 => return Err(AtomicError::NonCanonicalJson("unescaped control")),
                _ => {
                    let ch = self.text[self.offset..]
                        .chars()
                        .next()
                        .ok_or(AtomicError::NonCanonicalJson("truncated string"))?;
                    let mut encoded = [0; 2];
                    output.extend_from_slice(ch.encode_utf16(&mut encoded));
                    self.offset += ch.len_utf8();
                }
            }
        }
        Err(AtomicError::NonCanonicalJson("unterminated string"))
    }
}

fn parse_hex_quad(bytes: &[u8], offset: &mut usize) -> Result<u16, AtomicError> {
    if bytes.len().saturating_sub(*offset) < 4 {
        return Err(AtomicError::NonCanonicalJson("truncated unicode escape"));
    }
    let mut value = 0u16;
    for _ in 0..4 {
        let digit = match bytes[*offset] {
            b'0'..=b'9' => bytes[*offset] - b'0',
            b'a'..=b'f' => bytes[*offset] - b'a' + 10,
            b'A'..=b'F' => bytes[*offset] - b'A' + 10,
            _ => return Err(AtomicError::NonCanonicalJson("invalid unicode escape")),
        };
        value = (value << 4) | u16::from(digit);
        *offset += 1;
    }
    Ok(value)
}

pub(super) fn require_canonical_json(bytes: &[u8], path: &'static str) -> Result<(), AtomicError> {
    let text = std::str::from_utf8(bytes).map_err(|_| AtomicError::NonCanonicalJson(path))?;
    let parsed = Parser::parse(text)?;
    if parsed.canonical().as_bytes() != bytes {
        return Err(AtomicError::NonCanonicalJson(path));
    }
    Ok(())
}

pub(super) fn canonical_object_without_string_field(
    bytes: &[u8],
    field: &str,
    path: &'static str,
) -> Result<(Vec<u8>, String), AtomicError> {
    let text = std::str::from_utf8(bytes).map_err(|_| AtomicError::NonCanonicalJson(path))?;
    let parsed = Parser::parse(text)?;
    if parsed.canonical().as_bytes() != bytes {
        return Err(AtomicError::NonCanonicalJson(path));
    }
    let Json::Object(mut fields) = parsed else {
        return Err(AtomicError::NonCanonicalJson(path));
    };
    let Some(value) = fields.remove(&JsonString::from_str(field)) else {
        return Err(AtomicError::InvalidRecord(path));
    };
    let Json::String(value) = value else {
        return Err(AtomicError::InvalidRecord(path));
    };
    let value = value
        .to_well_formed_string()
        .ok_or(AtomicError::InvalidRecord(path))?;
    Ok((Json::Object(fields).canonical().into_bytes(), value))
}

fn require_id(value: &str, path: &'static str) -> Result<String, AtomicError> {
    if value.is_empty() || value.trim() != value || value.contains('\0') {
        return Err(AtomicError::InvalidRecord(path));
    }
    Ok(value.to_owned())
}

fn require_counter(value: &str, path: &'static str) -> Result<u64, AtomicError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
        || value.len() > 20
    {
        return Err(AtomicError::InvalidRecord(path));
    }
    value
        .parse::<u64>()
        .map_err(|_| AtomicError::InvalidRecord(path))
}

fn require_timestamp(value: &str, path: &'static str) -> Result<String, AtomicError> {
    require_id(value, path)?;
    if value.len() < 20 || !value.ends_with('Z') {
        return Err(AtomicError::InvalidRecord(path));
    }
    Ok(value.to_owned())
}

fn js(value: Json) -> Json {
    value
}

fn fingerprint(record: &PreparedRecord) -> String {
    let mut event = BTreeMap::new();
    event.insert(
        "counter".into(),
        Json::String(record.counter.clone().into()),
    );
    event.insert(
        "eventHash".into(),
        Json::String(record.event_hash.clone().into()),
    );
    event.insert(
        "eventId".into(),
        Json::String(record.event_id.clone().into()),
    );
    event.insert(
        "eventType".into(),
        Json::String(record.event_type.clone().into()),
    );
    event.insert(
        "expectedPreviousCounter".into(),
        record
            .expected_previous_counter
            .as_ref()
            .map(|value| Json::String(value.clone().into()))
            .unwrap_or(Json::Null),
    );
    event.insert(
        "occurredAt".into(),
        Json::String(record.occurred_at.clone().into()),
    );
    event.insert(
        "streamId".into(),
        Json::String(record.stream_id.clone().into()),
    );
    let mut object = BTreeMap::new();
    object.insert(
        "objectHash".into(),
        Json::String(record.object_hash.clone().into()),
    );
    object.insert(
        "objectId".into(),
        Json::String(record.object_id.clone().into()),
    );
    object.insert(
        "objectType".into(),
        Json::String(record.object_type.clone().into()),
    );
    object.insert(
        "objectVersion".into(),
        Json::String(record.object_version.clone().into()),
    );
    let mut receipt = BTreeMap::new();
    receipt.insert(
        "receiptHash".into(),
        Json::String(record.receipt_hash.clone().into()),
    );
    receipt.insert(
        "receiptId".into(),
        Json::String(record.receipt_id.clone().into()),
    );
    receipt.insert(
        "receiptType".into(),
        Json::String(record.receipt_type.clone().into()),
    );
    receipt.insert(
        "recordedAt".into(),
        Json::String(record.recorded_at.clone().into()),
    );
    let native = match &record.native_identity {
        None => Json::Null,
        Some(identity) => {
            let mut fields = BTreeMap::new();
            fields.insert(
                "nativeId".into(),
                Json::String(identity.native_id.clone().into()),
            );
            fields.insert(
                "runtimeInstanceId".into(),
                Json::String(identity.runtime_instance_id.clone().into()),
            );
            Json::Object(fields)
        }
    };
    let mut root = BTreeMap::new();
    root.insert(
        "domainId".into(),
        Json::String(record.domain_id.clone().into()),
    );
    root.insert("event".into(), Json::Object(event));
    root.insert("nativeIdentity".into(), native);
    root.insert("object".into(), Json::Object(object));
    root.insert("receipt".into(), Json::Object(receipt));
    content_hash(js(Json::Object(root)).canonical().as_bytes())
}

fn prepare(input: DomainRecordInput) -> Result<PreparedRecord, AtomicError> {
    require_canonical_json(&input.object_bytes, "object.canonicalBytes")?;
    require_canonical_json(&input.event_bytes, "event.canonicalBytes")?;
    require_canonical_json(&input.receipt_bytes, "receipt.canonicalBytes")?;
    let counter = require_counter(&input.counter, "event.counter")?;
    match &input.expected_previous_counter {
        None if counter != 0 => return Err(AtomicError::InvalidRecord("first counter must be 0")),
        Some(previous) => {
            let previous = require_counter(previous, "event.expectedPreviousCounter")?;
            if counter
                != previous
                    .checked_add(1)
                    .ok_or(AtomicError::InvalidRecord("counter"))?
            {
                return Err(AtomicError::InvalidRecord(
                    "counter does not follow previous",
                ));
            }
        }
        None => {}
    }
    if input.native_identity.is_some() {
        return Err(AtomicError::InvalidRecord(
            "nativeIdentity is not admitted in this slice except NativeBinding",
        ));
    }
    let mut record = PreparedRecord {
        domain_id: require_id(&input.domain_id, "domainId")?,
        object_type: require_id(&input.object_type, "objectType")?,
        object_id: require_id(&input.object_id, "objectId")?,
        object_version: require_id(&input.object_version, "objectVersion")?,
        object_hash: content_hash(&input.object_bytes),
        object_bytes: input.object_bytes,
        native_identity: None,
        event_id: require_id(&input.event_id, "eventId")?,
        stream_id: require_id(&input.stream_id, "streamId")?,
        expected_previous_counter: input.expected_previous_counter,
        counter: input.counter,
        event_type: require_id(&input.event_type, "eventType")?,
        occurred_at: require_timestamp(&input.occurred_at, "occurredAt")?,
        event_hash: content_hash(&input.event_bytes),
        event_bytes: input.event_bytes,
        receipt_id: require_id(&input.receipt_id, "receiptId")?,
        operation_id: require_id(&input.operation_id, "operationId")?,
        receipt_type: require_id(&input.receipt_type, "receiptType")?,
        recorded_at: require_timestamp(&input.recorded_at, "recordedAt")?,
        receipt_hash: content_hash(&input.receipt_bytes),
        receipt_bytes: input.receipt_bytes,
        operation_fingerprint: String::new(),
    };
    record.operation_fingerprint = fingerprint(&record);
    Ok(record)
}

fn sqlite_message(database: *mut c_void) -> String {
    let pointer = unsafe { sqlite3_errmsg(database) };
    if pointer.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }
}

fn fail(database: *mut c_void, code: c_int) -> AtomicError {
    AtomicError::Sqlite {
        code,
        message: sqlite_message(database),
    }
}

pub(crate) struct Statement {
    database: *mut c_void,
    raw: *mut c_void,
}

impl Statement {
    pub(crate) fn prepare(database: *mut c_void, sql: &str) -> Result<Self, AtomicError> {
        let sql = CString::new(sql).map_err(|_| AtomicError::InvalidRecord("sql"))?;
        let mut raw = std::ptr::null_mut();
        let rc = unsafe {
            sqlite3_prepare_v2(database, sql.as_ptr(), -1, &mut raw, std::ptr::null_mut())
        };
        if rc != SQLITE_OK {
            if !raw.is_null() {
                unsafe {
                    let _ = sqlite3_finalize(raw);
                }
            }
            return Err(fail(database, rc));
        }
        Ok(Self { database, raw })
    }

    pub(crate) fn bind_text(&self, index: c_int, value: &str) -> Result<(), AtomicError> {
        let rc = unsafe {
            sqlite3_bind_text(
                self.raw,
                index,
                value.as_ptr().cast(),
                value.len() as c_int,
                sqlite_transient(),
            )
        };
        if rc == SQLITE_OK {
            Ok(())
        } else {
            Err(fail(self.database, rc))
        }
    }

    pub(crate) fn bind_blob(&self, index: c_int, value: &[u8]) -> Result<(), AtomicError> {
        let rc = unsafe {
            sqlite3_bind_blob(
                self.raw,
                index,
                value.as_ptr().cast(),
                value.len() as c_int,
                sqlite_transient(),
            )
        };
        if rc == SQLITE_OK {
            Ok(())
        } else {
            Err(fail(self.database, rc))
        }
    }

    pub(crate) fn bind_i64(&self, index: c_int, value: i64) -> Result<(), AtomicError> {
        let rc = unsafe { sqlite3_bind_int64(self.raw, index, value) };
        if rc == SQLITE_OK {
            Ok(())
        } else {
            Err(fail(self.database, rc))
        }
    }

    pub(crate) fn step_done(&self) -> Result<(), AtomicError> {
        let rc = unsafe { sqlite3_step(self.raw) };
        if rc == SQLITE_DONE {
            Ok(())
        } else {
            Err(fail(self.database, rc))
        }
    }

    pub(crate) fn step_row(&self) -> Result<bool, AtomicError> {
        match unsafe { sqlite3_step(self.raw) } {
            SQLITE_ROW => Ok(true),
            SQLITE_DONE => Ok(false),
            code => Err(fail(self.database, code)),
        }
    }

    pub(crate) fn column_text(&self, column: c_int) -> Result<String, AtomicError> {
        let pointer = unsafe { sqlite3_column_text(self.raw, column) };
        if pointer.is_null() {
            return Err(AtomicError::InvalidRecord("null column"));
        }
        let bytes = unsafe { sqlite3_column_bytes(self.raw, column) } as usize;
        let slice = unsafe { std::slice::from_raw_parts(pointer, bytes) };
        String::from_utf8(slice.to_vec()).map_err(|_| AtomicError::InvalidRecord("column utf8"))
    }
}

impl Drop for Statement {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                let _ = sqlite3_finalize(self.raw);
            }
            self.raw = std::ptr::null_mut();
        }
    }
}

pub(crate) fn exec(
    connection: &mut VerifiedDatabaseConnection<'_>,
    sql: &str,
) -> Result<(), AtomicError> {
    connection.execute(sql).map_err(AtomicError::from)
}

fn changes(connection: &VerifiedDatabaseConnection<'_>) -> i32 {
    unsafe { sqlite3_changes(connection.as_ptr()) }
}

fn require_one_change(
    connection: &VerifiedDatabaseConnection<'_>,
    stage: &'static str,
) -> Result<(), AtomicError> {
    if changes(connection) == 1 {
        Ok(())
    } else {
        Err(AtomicError::WriteConflict(stage))
    }
}

const ADMITTED_TABLES: &[&str] = &[
    "gogoke_objects",
    "gogoke_native_identities",
    "gogoke_stream_heads",
    "gogoke_events",
    "gogoke_receipts",
];

pub fn admit_core_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let mut found = Vec::new();
    while statement.step_row()? {
        found.push(statement.column_text(0)?);
    }
    drop(statement);
    for name in &found {
        if !ADMITTED_TABLES.contains(&name.as_str()) {
            return Err(AtomicError::InvalidRecord("unknown schema object"));
        }
    }
    for required in ADMITTED_TABLES {
        if !found.iter().any(|name| name == required) {
            return Err(AtomicError::InvalidRecord("missing core table"));
        }
    }
    Ok(())
}

fn core_schema_statements() -> [String; 7] {
    [
        format!(
            "CREATE TABLE IF NOT EXISTS gogoke_objects (
                schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version = 1),
                domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
                object_type TEXT NOT NULL CHECK (length(object_type) > 0),
                object_id TEXT NOT NULL CHECK (length(object_id) > 0),
                object_version TEXT NOT NULL CHECK (length(object_version) > 0),
                canonical_json BLOB NOT NULL CHECK (length(canonical_json) > 0),
                content_hash TEXT NOT NULL CHECK (length(content_hash) = 71),
                created_at TEXT NOT NULL CHECK (length(created_at) > 0),
                PRIMARY KEY (domain_id, object_type, object_id, object_version)
            ) STRICT"
        ),
        "CREATE TABLE IF NOT EXISTS gogoke_native_identities (
                schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version = 1),
                domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
                runtime_instance_id TEXT NOT NULL CHECK (length(runtime_instance_id) > 0),
                native_id TEXT NOT NULL CHECK (length(native_id) > 0),
                object_type TEXT NOT NULL,
                object_id TEXT NOT NULL,
                object_version TEXT NOT NULL,
                PRIMARY KEY (domain_id, runtime_instance_id, native_id),
                FOREIGN KEY (domain_id, object_type, object_id, object_version)
                  REFERENCES gogoke_objects (domain_id, object_type, object_id, object_version)
                  ON DELETE RESTRICT ON UPDATE RESTRICT
            ) STRICT"
            .to_owned(),
        format!(
            "CREATE TABLE IF NOT EXISTS gogoke_stream_heads (
                schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version = 1),
                domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
                stream_id TEXT NOT NULL CHECK (length(stream_id) > 0),
                counter TEXT NOT NULL CHECK ({COUNTER_CHECK}),
                PRIMARY KEY (domain_id, stream_id)
            ) STRICT"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS gogoke_events (
                schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version = 1),
                domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
                event_id TEXT NOT NULL CHECK (length(event_id) > 0),
                stream_id TEXT NOT NULL CHECK (length(stream_id) > 0),
                stream_counter TEXT NOT NULL CHECK ({STREAM_COUNTER_CHECK}),
                event_type TEXT NOT NULL CHECK (length(event_type) > 0),
                occurred_at TEXT NOT NULL CHECK (length(occurred_at) > 0),
                object_type TEXT NOT NULL,
                object_id TEXT NOT NULL,
                object_version TEXT NOT NULL,
                canonical_json BLOB NOT NULL CHECK (length(canonical_json) > 0),
                content_hash TEXT NOT NULL CHECK (length(content_hash) = 71),
                PRIMARY KEY (domain_id, event_id),
                UNIQUE (domain_id, stream_id, stream_counter),
                FOREIGN KEY (domain_id, stream_id)
                  REFERENCES gogoke_stream_heads (domain_id, stream_id)
                  ON DELETE RESTRICT ON UPDATE RESTRICT,
                FOREIGN KEY (domain_id, object_type, object_id, object_version)
                  REFERENCES gogoke_objects (domain_id, object_type, object_id, object_version)
                  ON DELETE RESTRICT ON UPDATE RESTRICT
            ) STRICT"
        ),
        "CREATE TABLE IF NOT EXISTS gogoke_receipts (
                schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version = 1),
                domain_id TEXT NOT NULL CHECK (length(domain_id) > 0),
                receipt_id TEXT NOT NULL CHECK (length(receipt_id) > 0),
                operation_id TEXT NOT NULL CHECK (length(operation_id) > 0),
                event_id TEXT NOT NULL,
                object_type TEXT NOT NULL,
                object_id TEXT NOT NULL,
                object_version TEXT NOT NULL,
                receipt_type TEXT NOT NULL CHECK (length(receipt_type) > 0),
                recorded_at TEXT NOT NULL CHECK (length(recorded_at) > 0),
                operation_fingerprint TEXT NOT NULL CHECK (length(operation_fingerprint) = 71),
                canonical_json BLOB NOT NULL CHECK (length(canonical_json) > 0),
                content_hash TEXT NOT NULL CHECK (length(content_hash) = 71),
                PRIMARY KEY (domain_id, receipt_id),
                UNIQUE (domain_id, operation_id),
                FOREIGN KEY (domain_id, event_id)
                  REFERENCES gogoke_events (domain_id, event_id)
                  ON DELETE RESTRICT ON UPDATE RESTRICT,
                FOREIGN KEY (domain_id, object_type, object_id, object_version)
                  REFERENCES gogoke_objects (domain_id, object_type, object_id, object_version)
                  ON DELETE RESTRICT ON UPDATE RESTRICT
            ) STRICT"
            .to_owned(),
        "CREATE INDEX IF NOT EXISTS idx_gogoke_events_stream ON gogoke_events (domain_id, stream_id, stream_counter)".to_owned(),
        "CREATE INDEX IF NOT EXISTS idx_gogoke_receipts_event ON gogoke_receipts (domain_id, event_id)".to_owned(),
    ]
}

pub fn apply_core_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), AtomicError> {
    for statement in core_schema_statements() {
        exec(connection, &statement)?;
    }
    exec(connection, "PRAGMA foreign_keys = ON")?;
    exec(connection, "PRAGMA journal_mode = WAL")?;
    exec(connection, "PRAGMA synchronous = FULL")?;
    assert_durability(connection)?;
    admit_core_schema(connection)
}

fn pragma_text(
    connection: &mut VerifiedDatabaseConnection<'_>,
    sql: &str,
) -> Result<String, AtomicError> {
    let statement = Statement::prepare(connection.as_ptr(), sql)?;
    if !statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(sql.to_owned()));
    }
    statement.column_text(0)
}

fn assert_durability(connection: &mut VerifiedDatabaseConnection<'_>) -> Result<(), AtomicError> {
    let foreign_keys = pragma_text(connection, "PRAGMA foreign_keys")?;
    if foreign_keys != "1" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "foreign_keys={foreign_keys}"
        )));
    }
    let journal = pragma_text(connection, "PRAGMA journal_mode")?;
    if !journal.eq_ignore_ascii_case("wal") {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "journal_mode={journal}"
        )));
    }
    let synchronous = pragma_text(connection, "PRAGMA synchronous")?;
    if synchronous != "2" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "synchronous={synchronous}"
        )));
    }
    Ok(())
}

struct ExistingReceipt {
    domain_id: String,
    receipt_id: String,
    operation_id: String,
    event_id: String,
    object_type: String,
    object_id: String,
    object_version: String,
    object_hash: String,
    event_hash: String,
    receipt_hash: String,
    operation_fingerprint: String,
}

fn query_receipt(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    operation_id: &str,
) -> Result<Option<ExistingReceipt>, AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT r.domain_id, r.receipt_id, r.operation_id, r.event_id, r.object_type,
                r.object_id, r.object_version, o.content_hash, e.content_hash,
                r.content_hash, r.operation_fingerprint
         FROM gogoke_receipts r
         JOIN gogoke_objects o
           ON o.domain_id = r.domain_id AND o.object_type = r.object_type
          AND o.object_id = r.object_id AND o.object_version = r.object_version
         JOIN gogoke_events e
           ON e.domain_id = r.domain_id AND e.event_id = r.event_id
         WHERE r.domain_id = ? AND r.operation_id = ?",
    )?;
    statement.bind_text(1, domain_id)?;
    statement.bind_text(2, operation_id)?;
    if !statement.step_row()? {
        return Ok(None);
    }
    Ok(Some(ExistingReceipt {
        domain_id: statement.column_text(0)?,
        receipt_id: statement.column_text(1)?,
        operation_id: statement.column_text(2)?,
        event_id: statement.column_text(3)?,
        object_type: statement.column_text(4)?,
        object_id: statement.column_text(5)?,
        object_version: statement.column_text(6)?,
        object_hash: statement.column_text(7)?,
        event_hash: statement.column_text(8)?,
        receipt_hash: statement.column_text(9)?,
        operation_fingerprint: statement.column_text(10)?,
    }))
}

fn matches(existing: &ExistingReceipt, record: &PreparedRecord) -> bool {
    existing.domain_id == record.domain_id
        && existing.receipt_id == record.receipt_id
        && existing.operation_id == record.operation_id
        && existing.event_id == record.event_id
        && existing.object_type == record.object_type
        && existing.object_id == record.object_id
        && existing.object_version == record.object_version
        && existing.object_hash == record.object_hash
        && existing.event_hash == record.event_hash
        && existing.receipt_hash == record.receipt_hash
        && existing.operation_fingerprint == record.operation_fingerprint
}

fn receipt_from(record: &PreparedRecord, disposition: &'static str) -> DomainRecordReceipt {
    DomainRecordReceipt {
        disposition,
        domain_id: record.domain_id.clone(),
        receipt_id: record.receipt_id.clone(),
        operation_id: record.operation_id.clone(),
        event_id: record.event_id.clone(),
        object_hash: record.object_hash.clone(),
        event_hash: record.event_hash.clone(),
        receipt_hash: record.receipt_hash.clone(),
        operation_fingerprint: record.operation_fingerprint.clone(),
    }
}

fn rollback(connection: &mut VerifiedDatabaseConnection<'_>) {
    let _ = connection.execute("ROLLBACK");
}

pub fn get_receipt(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    operation_id: &str,
) -> Result<Option<DomainRecordReceipt>, AtomicError> {
    let domain_id = require_id(domain_id, "domainId")?;
    let operation_id = require_id(operation_id, "operationId")?;
    Ok(
        query_receipt(connection, &domain_id, &operation_id)?.map(|existing| DomainRecordReceipt {
            disposition: "STORED",
            domain_id: existing.domain_id,
            receipt_id: existing.receipt_id,
            operation_id: existing.operation_id,
            event_id: existing.event_id,
            object_hash: existing.object_hash,
            event_hash: existing.event_hash,
            receipt_hash: existing.receipt_hash,
            operation_fingerprint: existing.operation_fingerprint,
        }),
    )
}

pub fn commit_domain_record(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: DomainRecordInput,
) -> Result<DomainRecordReceipt, AtomicError> {
    let record = prepare(input)?;
    assert_durability(connection)?;
    exec(connection, "BEGIN IMMEDIATE")?;
    let outcome = apply_prepared_domain_record_in_transaction(connection, record);
    match outcome {
        Ok(receipt) => {
            if let Err(error) = exec(connection, "COMMIT") {
                rollback(connection);
                return Err(match error {
                    AtomicError::SameOpen(SameOpenError::SqliteExec { .. }) => {
                        AtomicError::CommitUnknown
                    }
                    other => other,
                });
            }
            Ok(receipt)
        }
        Err(error) => {
            rollback(connection);
            Err(error)
        }
    }
}

/// Internal storage composition on the existing verified connection. This is
/// NOT actor admission or a Decision/Outcome permission. The owning authority
/// must revalidate current rights and references before this group, including
/// replay, and must abort its whole transaction on an error from this function.
/// No callback, BEGIN, COMMIT, second database, or external I/O occurs here.
pub(super) fn apply_domain_record_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: DomainRecordInput,
) -> Result<DomainRecordReceipt, AtomicError> {
    let record = prepare(input)?;
    assert_durability(connection)?;
    apply_prepared_domain_record_in_transaction(connection, record)
}

fn apply_prepared_domain_record_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    record: PreparedRecord,
) -> Result<DomainRecordReceipt, AtomicError> {
    unsafe extern "C" {
        fn sqlite3_get_autocommit(database: *mut c_void) -> c_int;
    }
    // SAFETY: the caller exclusively owns the same live verified connection.
    if unsafe { sqlite3_get_autocommit(connection.as_ptr()) } != 0 {
        return Err(AtomicError::InvalidRecord(
            "record write group requires owning transaction",
        ));
    }
    (|| {
        if let Some(existing) = query_receipt(connection, &record.domain_id, &record.operation_id)?
        {
            if !matches(&existing, &record) {
                return Err(AtomicError::OperationConflict);
            }
            return Ok(receipt_from(&record, "RECONCILED"));
        }

        if record.expected_previous_counter.is_none() {
            let statement = Statement::prepare(
                connection.as_ptr(),
                "INSERT INTO gogoke_stream_heads (domain_id, stream_id, counter)
                 VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
            )?;
            statement.bind_text(1, &record.domain_id)?;
            statement.bind_text(2, &record.stream_id)?;
            statement.bind_text(3, &record.counter)?;
            statement.step_done()?;
        } else {
            let statement = Statement::prepare(
                connection.as_ptr(),
                "UPDATE gogoke_stream_heads SET counter = ?
                 WHERE domain_id = ? AND stream_id = ? AND counter = ?",
            )?;
            statement.bind_text(1, &record.counter)?;
            statement.bind_text(2, &record.domain_id)?;
            statement.bind_text(3, &record.stream_id)?;
            statement.bind_text(
                4,
                record
                    .expected_previous_counter
                    .as_deref()
                    .expect("previous"),
            )?;
            statement.step_done()?;
        }
        if changes(connection) != 1 {
            return Err(AtomicError::CounterConflict);
        }

        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO gogoke_objects (
                domain_id, object_type, object_id, object_version,
                canonical_json, content_hash, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )?;
        statement.bind_text(1, &record.domain_id)?;
        statement.bind_text(2, &record.object_type)?;
        statement.bind_text(3, &record.object_id)?;
        statement.bind_text(4, &record.object_version)?;
        statement.bind_blob(5, &record.object_bytes)?;
        statement.bind_text(6, &record.object_hash)?;
        statement.bind_text(7, &record.occurred_at)?;
        statement.step_done()?;
        require_one_change(connection, "object")?;

        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO gogoke_events (
                domain_id, event_id, stream_id, stream_counter, event_type, occurred_at,
                object_type, object_id, object_version, canonical_json, content_hash
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )?;
        statement.bind_text(1, &record.domain_id)?;
        statement.bind_text(2, &record.event_id)?;
        statement.bind_text(3, &record.stream_id)?;
        statement.bind_text(4, &record.counter)?;
        statement.bind_text(5, &record.event_type)?;
        statement.bind_text(6, &record.occurred_at)?;
        statement.bind_text(7, &record.object_type)?;
        statement.bind_text(8, &record.object_id)?;
        statement.bind_text(9, &record.object_version)?;
        statement.bind_blob(10, &record.event_bytes)?;
        statement.bind_text(11, &record.event_hash)?;
        statement.step_done()?;
        require_one_change(connection, "event")?;

        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO gogoke_receipts (
                domain_id, receipt_id, operation_id, event_id,
                object_type, object_id, object_version, receipt_type, recorded_at,
                operation_fingerprint, canonical_json, content_hash
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )?;
        statement.bind_text(1, &record.domain_id)?;
        statement.bind_text(2, &record.receipt_id)?;
        statement.bind_text(3, &record.operation_id)?;
        statement.bind_text(4, &record.event_id)?;
        statement.bind_text(5, &record.object_type)?;
        statement.bind_text(6, &record.object_id)?;
        statement.bind_text(7, &record.object_version)?;
        statement.bind_text(8, &record.receipt_type)?;
        statement.bind_text(9, &record.recorded_at)?;
        statement.bind_text(10, &record.operation_fingerprint)?;
        statement.bind_blob(11, &record.receipt_bytes)?;
        statement.bind_text(12, &record.receipt_hash)?;
        statement.step_done()?;
        require_one_change(connection, "receipt")?;
        Ok(receipt_from(&record, "COMMITTED"))
    })()
}

pub fn count_table(
    connection: &mut VerifiedDatabaseConnection<'_>,
    table: &str,
) -> Result<i64, AtomicError> {
    if !matches!(
        table,
        "gogoke_objects" | "gogoke_events" | "gogoke_receipts" | "gogoke_stream_heads"
    ) {
        return Err(AtomicError::InvalidRecord("table"));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        &format!("SELECT COUNT(*) FROM {table}"),
    )?;
    if !statement.step_row()? {
        return Err(AtomicError::InvalidRecord("count"));
    }
    statement
        .column_text(0)?
        .parse::<i64>()
        .map_err(|_| AtomicError::InvalidRecord("count"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_accepts_js_safe_integer_tokens() {
        for token in ["0", "1", "9007199254740991"] {
            let parsed = Parser::parse(token).expect("safe integer");
            assert_eq!(parsed.canonical(), token);
        }
    }

    #[test]
    fn ecmascript_number_canonicalization_matches_independent_node_golden() {
        for line in include_str!("authority/fixtures/ecmascript_numbers.golden.tsv").lines() {
            let mut columns = line.split('\t');
            let token = columns.next().expect("input token");
            let expected = columns.next().expect("Node JSON.stringify output");
            let outcome = columns.next().expect("Node safe-number disposition");
            assert!(
                columns.next().is_none(),
                "unexpected golden columns: {line}"
            );

            let number = token.parse::<f64>().expect("Node accepted JSON number");
            assert_eq!(canonical_js_number(number), expected, "input {token}");
            match outcome {
                "accept" => {
                    let parsed = Parser::parse(token).expect("safe finite number");
                    assert_eq!(parsed.canonical(), expected, "input {token}");
                }
                "reject" => assert!(
                    require_canonical_json(token.as_bytes(), "number golden").is_err(),
                    "accepted rejected or noncanonical number {token}"
                ),
                _ => panic!("invalid fixture disposition: {outcome}"),
            }
        }
    }

    #[test]
    fn ecmascript_object_key_order_and_duplicates_match_node_golden() {
        for line in include_str!("authority/fixtures/ecmascript_object_keys.golden.tsv").lines() {
            let mut columns = line.split('\t');
            let outcome = columns.next().expect("fixture outcome");
            let input = columns.next().expect("input object");
            let expected = columns.next().expect("Node canonical object or rejection");
            assert!(
                columns.next().is_none(),
                "unexpected fixture columns: {line}"
            );

            match outcome {
                "accept" => {
                    let parsed = Parser::parse(input).expect("valid unique-key object");
                    assert_eq!(parsed.canonical(), expected, "input {input}");
                    assert!(require_canonical_json(expected.as_bytes(), "object.golden").is_ok());
                }
                "reject" => assert!(
                    Parser::parse(input).is_err(),
                    "accepted duplicate-key object {input}"
                ),
                _ => panic!("invalid fixture outcome: {outcome}"),
            }
        }
    }

    #[test]
    fn canonical_json_rejects_noncanonical_numbers_and_invalid_numeric_syntax() {
        for token in ["-0", "1.0", "1e0", "1E+0", "1e-6"] {
            let parsed = Parser::parse(token).expect("valid JSON number");
            assert_ne!(parsed.canonical().as_bytes(), token.as_bytes());
            assert!(require_canonical_json(token.as_bytes(), "number.golden").is_err());
        }

        for token in [
            "+1",
            "01",
            "--1",
            "1.",
            ".1",
            "1e",
            "1e+",
            "1e-",
            "1e309",
            "1e+20",
            "NaN",
            "Infinity",
            "9007199254740992",
            "-9007199254740992",
            "18446744073709551615",
        ] {
            assert!(
                Parser::parse(token).is_err(),
                "accepted invalid numeric token {token}"
            );
        }
        let golden = br#"{"max":9007199254740991,"fraction":-0.5,"tiny":1e-7,"zero":0}"#;
        let canonical = br#"{"fraction":-0.5,"max":9007199254740991,"tiny":1e-7,"zero":0}"#;
        let parsed = Parser::parse(std::str::from_utf8(golden).unwrap()).expect("JSON object");
        assert_eq!(parsed.canonical().as_bytes(), canonical);
        assert!(require_canonical_json(canonical, "number.object.golden").is_ok());

        let leading_zero = br#"{"value":01}"#;
        assert!(Parser::parse(std::str::from_utf8(leading_zero).unwrap()).is_err());
        let noncanonical = br#"{"value":1e0}"#;
        let parsed = Parser::parse(std::str::from_utf8(noncanonical).unwrap())
            .expect("valid number token normalizes");
        assert_eq!(parsed.canonical().as_bytes(), br#"{"value":1}"#);
        assert!(require_canonical_json(noncanonical, "number.object.noncanonical").is_err());
    }

    #[test]
    fn canonical_json_accepts_and_reencodes_legal_nul_escape() {
        let bytes = br#""left\u0000right""#;
        let parsed = Parser::parse(std::str::from_utf8(bytes).unwrap()).expect("escaped NUL");
        assert_eq!(parsed.canonical().as_bytes(), bytes);
        assert!(require_canonical_json(bytes, "nul.golden").is_ok());
        assert!(Parser::parse("\"left\0right\"").is_err());
    }
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("gogoke-route-b2-{label}-{nonce}"));
        std::fs::create_dir(&root).expect("scratch root");
        root
    }

    fn sample(operation_id: &str, event_id: &str, object_id: &str) -> DomainRecordInput {
        DomainRecordInput {
            domain_id: "domain-a".into(),
            object_type: "Seat".into(),
            object_id: object_id.into(),
            object_version: "v1".into(),
            object_bytes: br#"{"id":"seat"}"#.to_vec(),
            native_identity: None,
            event_id: event_id.into(),
            stream_id: "stream-a".into(),
            expected_previous_counter: None,
            counter: "0".into(),
            event_type: "created".into(),
            occurred_at: "2026-09-20T00:00:00Z".into(),
            event_bytes: br#"{"kind":"created"}"#.to_vec(),
            receipt_id: format!("receipt-{operation_id}"),
            operation_id: operation_id.into(),
            receipt_type: "accepted".into(),
            recorded_at: "2026-09-20T00:00:00Z".into(),
            receipt_bytes: br#"{"status":"ok"}"#.to_vec(),
        }
    }

    #[test]
    fn commit_reconcile_and_conflict_share_one_transaction() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("commit");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_core_schema(&mut connection).expect("schema");

        let committed =
            commit_domain_record(&mut connection, sample("op-1", "ev-1", "obj-1")).expect("commit");
        assert_eq!(committed.disposition, "COMMITTED");
        let stored = get_receipt(&mut connection, "domain-a", "op-1")
            .expect("get")
            .expect("present");
        assert_eq!(
            stored.operation_fingerprint,
            committed.operation_fingerprint
        );
        assert!(get_receipt(&mut connection, "domain-a", "missing")
            .expect("missing")
            .is_none());
        assert_eq!(count_table(&mut connection, "gogoke_objects").unwrap(), 1);
        assert_eq!(count_table(&mut connection, "gogoke_events").unwrap(), 1);
        assert_eq!(count_table(&mut connection, "gogoke_receipts").unwrap(), 1);

        let reconciled = commit_domain_record(&mut connection, sample("op-1", "ev-1", "obj-1"))
            .expect("reconcile");
        assert_eq!(reconciled.disposition, "RECONCILED");
        assert_eq!(count_table(&mut connection, "gogoke_events").unwrap(), 1);

        let mut conflicted = sample("op-1", "ev-2", "obj-1");
        conflicted.receipt_id = "receipt-other".into();
        let error = commit_domain_record(&mut connection, conflicted).expect_err("conflict");
        assert!(matches!(error, AtomicError::OperationConflict), "{error:?}");
        assert_eq!(count_table(&mut connection, "gogoke_events").unwrap(), 1);
        assert_eq!(count_table(&mut connection, "gogoke_receipts").unwrap(), 1);

        let mut second = sample("op-2", "ev-2", "obj-2");
        second.expected_previous_counter = Some("0".into());
        second.counter = "1".into();
        let advanced = commit_domain_record(&mut connection, second).expect("advance");
        assert_eq!(advanced.disposition, "COMMITTED");
        assert_eq!(count_table(&mut connection, "gogoke_objects").unwrap(), 2);
        assert_eq!(count_table(&mut connection, "gogoke_events").unwrap(), 2);
        assert_eq!(count_table(&mut connection, "gogoke_receipts").unwrap(), 2);
        assert_eq!(
            count_table(&mut connection, "gogoke_stream_heads").unwrap(),
            1
        );

        connection.close_checked().expect("close");
        drop(root);
        std::fs::remove_file(&database_path).ok();
        let _ = std::fs::remove_file(format!("{}-wal", database_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", database_path.display()));
        std::fs::remove_dir(&root_path).ok();
    }

    #[test]
    fn duplicate_object_identity_rolls_back_without_a_new_receipt() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("write-conflict");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_core_schema(&mut connection).expect("schema");
        commit_domain_record(&mut connection, sample("op-1", "ev-1", "obj-1")).expect("first");

        let mut duplicate = sample("op-2", "ev-2", "obj-1");
        duplicate.expected_previous_counter = Some("0".into());
        duplicate.counter = "1".into();
        let error = commit_domain_record(&mut connection, duplicate).expect_err("object identity");
        assert!(
            matches!(error, AtomicError::WriteConflict("object")),
            "{error:?}"
        );
        assert_eq!(count_table(&mut connection, "gogoke_receipts").unwrap(), 1);
        assert_eq!(count_table(&mut connection, "gogoke_events").unwrap(), 1);
        assert_eq!(count_table(&mut connection, "gogoke_objects").unwrap(), 1);

        connection.close_checked().expect("close");
        drop(root);
        std::fs::remove_file(&database_path).ok();
        let _ = std::fs::remove_file(format!("{}-wal", database_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", database_path.display()));
        std::fs::remove_dir(&root_path).ok();
    }

    #[test]
    fn counter_conflict_rolls_back_without_a_receipt() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("counter");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_core_schema(&mut connection).expect("schema");
        commit_domain_record(&mut connection, sample("op-1", "ev-1", "obj-1")).expect("first");

        let mut second = sample("op-2", "ev-2", "obj-2");
        second.expected_previous_counter = Some("1".into());
        second.counter = "2".into();
        let error = commit_domain_record(&mut connection, second).expect_err("stale counter");
        assert!(matches!(error, AtomicError::CounterConflict), "{error:?}");
        assert_eq!(count_table(&mut connection, "gogoke_receipts").unwrap(), 1);
        assert_eq!(count_table(&mut connection, "gogoke_objects").unwrap(), 1);

        connection.close_checked().expect("close");
        drop(root);
        std::fs::remove_file(&database_path).ok();
        let _ = std::fs::remove_file(format!("{}-wal", database_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", database_path.display()));
        std::fs::remove_dir(&root_path).ok();
    }

    #[test]
    fn non_canonical_json_is_rejected_before_begin() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("json");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_core_schema(&mut connection).expect("schema");
        let mut input = sample("op-1", "ev-1", "obj-1");
        input.object_bytes = br#"{ "id": "seat" }"#.to_vec();
        let error = commit_domain_record(&mut connection, input).expect_err("spaces");
        assert!(
            matches!(error, AtomicError::NonCanonicalJson(_)),
            "{error:?}"
        );
        assert_eq!(count_table(&mut connection, "gogoke_receipts").unwrap(), 0);
        connection.close_checked().expect("close");
        drop(root);
        std::fs::remove_file(&database_path).ok();
        let _ = std::fs::remove_file(format!("{}-wal", database_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", database_path.display()));
        std::fs::remove_dir(&root_path).ok();
    }

    #[test]
    fn unknown_schema_object_is_refused_and_left_on_disk() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("schema");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_core_schema(&mut connection).expect("schema");
        connection
            .execute("CREATE TABLE extra_schema(value TEXT NOT NULL) STRICT")
            .expect("inject unknown table");
        let error = admit_core_schema(&mut connection).expect_err("unknown schema");
        assert!(
            matches!(error, AtomicError::InvalidRecord("unknown schema object")),
            "{error:?}"
        );
        assert_eq!(count_table(&mut connection, "gogoke_receipts").unwrap(), 0);
        let extra = Statement::prepare(connection.as_ptr(), "SELECT COUNT(*) FROM extra_schema")
            .expect("extra remains");
        assert!(extra.step_row().expect("row"));
        assert_eq!(extra.column_text(0).expect("0"), "0");
        drop(extra);
        connection.close_checked().expect("close");
        drop(root);
        std::fs::remove_file(&database_path).ok();
        let _ = std::fs::remove_file(format!("{}-wal", database_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", database_path.display()));
        std::fs::remove_dir(&root_path).ok();
    }
}
