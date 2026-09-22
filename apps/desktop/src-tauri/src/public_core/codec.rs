use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use serde::{
    de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::{Map, Number, Value};

use super::{Validate, MAX_FRAME_BYTES, SCHEMA_VERSION};

const DUPLICATE_KEY_MARKER: &str = "PUBLIC_JSON_DUPLICATE_KEY";
const NON_CANONICAL_NUMBER_MARKER: &str = "PUBLIC_JSON_NON_CANONICAL_NUMBER";

pub trait PublicDocument: Serialize + DeserializeOwned + Validate {}
impl<T> PublicDocument for T where T: Serialize + DeserializeOwned + Validate {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecErrorCode {
    InvalidUtf8,
    FrameTooLarge,
    DuplicateKey,
    NonCanonicalNumber,
    InvalidJson,
    MissingRequiredField,
    UnknownMajorVersion,
    ValueOverflow,
    BoundsExceeded,
    SchemaViolation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError {
    pub code: CodecErrorCode,
    pub message: String,
}

impl CodecError {
    fn new(code: CodecErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl Error for CodecError {}

/// Strictly decode one public document.
///
/// Validation order is deliberate: frame/UTF-8, full-file duplicate scan,
/// canonical integer lexemes, duplicate-preserving parse, major version,
/// typed schema, semantic bounds.
pub fn decode_json<T: PublicDocument>(bytes: &[u8]) -> Result<T, CodecError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(CodecError::new(
            CodecErrorCode::FrameTooLarge,
            "public frame exceeds 4 MiB",
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| CodecError::new(CodecErrorCode::InvalidUtf8, "frame is not UTF-8"))?;
    detect_duplicate_keys(text)?;
    validate_decoded_schema_version_number(text)?;
    validate_number_lexemes(text)?;
    let value = parse_without_duplicate_keys(text)?;
    validate_schema_version(&value)?;
    let document: T = serde_json::from_value(value).map_err(classify_typed_error)?;
    document.validate().map_err(classify_validation_error)?;
    Ok(document)
}

/// Encode a public document using stable UTF-8 key ordering, compact JSON,
/// array order preservation, and canonical integer-only JSON numbers.
pub fn canonical_json<T: Serialize + Validate>(document: &T) -> Result<Vec<u8>, CodecError> {
    document.validate().map_err(classify_validation_error)?;
    let value = serde_json::to_value(document)
        .map_err(|error| CodecError::new(CodecErrorCode::SchemaViolation, error.to_string()))?;
    validate_schema_version(&value)?;
    let mut output = String::new();
    write_canonical(&value, &mut output)?;
    if output.len() > MAX_FRAME_BYTES {
        return Err(CodecError::new(
            CodecErrorCode::FrameTooLarge,
            "encoded public frame exceeds 4 MiB",
        ));
    }
    Ok(output.into_bytes())
}

fn validate_schema_version(value: &Value) -> Result<(), CodecError> {
    let version_value = value
        .as_object()
        .and_then(|object| object.get("schemaVersion"))
        .ok_or_else(|| {
            CodecError::new(
                CodecErrorCode::MissingRequiredField,
                "top-level schemaVersion is required",
            )
        })?;
    let version = version_value
        .as_u64()
        .ok_or_else(schema_version_violation)?;
    if version == 0 {
        return Err(schema_version_violation());
    }
    if version != SCHEMA_VERSION {
        return Err(CodecError::new(
            CodecErrorCode::UnknownMajorVersion,
            format!("unsupported schemaVersion {version}"),
        ));
    }
    Ok(())
}

fn schema_version_violation() -> CodecError {
    CodecError::new(
        CodecErrorCode::SchemaViolation,
        "top-level schemaVersion must be a positive integer",
    )
}

fn detect_duplicate_keys(text: &str) -> Result<(), CodecError> {
    if DuplicateKeyScanner::new(text).scan() == Ok(true) {
        return Err(CodecError::new(
            CodecErrorCode::DuplicateKey,
            "duplicate JSON object key",
        ));
    }
    Ok(())
}

struct DuplicateKeyScanner<'a> {
    text: &'a str,
    index: usize,
}

impl<'a> DuplicateKeyScanner<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, index: 0 }
    }

    fn scan(mut self) -> Result<bool, ()> {
        let has_duplicate_key = self.scan_value(0)?;
        self.skip_whitespace();
        if self.index != self.text.len() {
            return Err(());
        }
        Ok(has_duplicate_key)
    }

    fn scan_value(&mut self, depth: usize) -> Result<bool, ()> {
        if depth > 128 {
            return Err(());
        }
        self.skip_whitespace();
        match self.current_byte() {
            Some(b'{') => self.scan_object(depth + 1),
            Some(b'[') => self.scan_array(depth + 1),
            Some(b'"') => {
                self.scan_string()?;
                Ok(false)
            }
            Some(b't') => self.scan_literal(b"true"),
            Some(b'f') => self.scan_literal(b"false"),
            Some(b'n') => self.scan_literal(b"null"),
            Some(b'-' | b'0'..=b'9') => {
                self.scan_number()?;
                Ok(false)
            }
            _ => Err(()),
        }
    }

    fn scan_object(&mut self, depth: usize) -> Result<bool, ()> {
        self.index += 1;
        let mut keys = BTreeSet::new();
        let mut has_duplicate_key = false;
        self.skip_whitespace();
        if self.consume_byte(b'}') {
            return Ok(false);
        }
        loop {
            self.skip_whitespace();
            let key = self.scan_string()?;
            has_duplicate_key |= !keys.insert(key);
            self.skip_whitespace();
            if !self.consume_byte(b':') {
                return Err(());
            }
            has_duplicate_key |= self.scan_value(depth)?;
            self.skip_whitespace();
            if self.consume_byte(b'}') {
                return Ok(has_duplicate_key);
            }
            if !self.consume_byte(b',') {
                return Err(());
            }
        }
    }

    fn scan_array(&mut self, depth: usize) -> Result<bool, ()> {
        self.index += 1;
        let mut has_duplicate_key = false;
        self.skip_whitespace();
        if self.consume_byte(b']') {
            return Ok(false);
        }
        loop {
            has_duplicate_key |= self.scan_value(depth)?;
            self.skip_whitespace();
            if self.consume_byte(b']') {
                return Ok(has_duplicate_key);
            }
            if !self.consume_byte(b',') {
                return Err(());
            }
        }
    }

    fn scan_string(&mut self) -> Result<String, ()> {
        let start = self.index;
        if !self.consume_byte(b'"') {
            return Err(());
        }
        let mut escaped = false;
        while let Some(byte) = self.current_byte() {
            self.index += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return serde_json::from_str(&self.text[start..self.index]).map_err(|_| ());
            }
        }
        Err(())
    }

    fn scan_number(&mut self) -> Result<(), ()> {
        self.consume_byte(b'-');
        match self.current_byte() {
            Some(b'0') => self.index += 1,
            Some(b'1'..=b'9') => {
                self.index += 1;
                while matches!(self.current_byte(), Some(b'0'..=b'9')) {
                    self.index += 1;
                }
            }
            _ => return Err(()),
        }
        if self.consume_byte(b'.') {
            if !matches!(self.current_byte(), Some(b'0'..=b'9')) {
                return Err(());
            }
            while matches!(self.current_byte(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
        }
        if matches!(self.current_byte(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.current_byte(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            if !matches!(self.current_byte(), Some(b'0'..=b'9')) {
                return Err(());
            }
            while matches!(self.current_byte(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
        }
        Ok(())
    }

    fn scan_literal(&mut self, literal: &[u8]) -> Result<bool, ()> {
        if self.text.as_bytes()[self.index..].starts_with(literal) {
            self.index += literal.len();
            Ok(false)
        } else {
            Err(())
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.current_byte(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.index += 1;
        }
    }

    fn consume_byte(&mut self, expected: u8) -> bool {
        if self.current_byte() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn current_byte(&self) -> Option<u8> {
        self.text.as_bytes().get(self.index).copied()
    }
}

fn validate_decoded_schema_version_number(text: &str) -> Result<(), CodecError> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let Ok(probe) = RootSchemaVersionProbe::deserialize(&mut deserializer) else {
        return Ok(());
    };
    if deserializer.end().is_err() {
        return Ok(());
    }
    if probe.has_fractional_schema_version {
        return Err(schema_version_violation());
    }
    Ok(())
}

struct RootSchemaVersionProbe {
    has_fractional_schema_version: bool,
}

impl<'de> Deserialize<'de> for RootSchemaVersionProbe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(RootSchemaVersionProbeVisitor)
    }
}

struct RootSchemaVersionProbeVisitor;

impl<'de> Visitor<'de> for RootSchemaVersionProbeVisitor {
    type Value = RootSchemaVersionProbe;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a top-level public JSON object")
    }

    fn visit_map<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut has_fractional_schema_version = false;
        while let Some(key) = entries.next_key::<String>()? {
            if key == "schemaVersion" {
                let value = entries.next_value::<Value>()?;
                has_fractional_schema_version |= match value {
                    Value::Number(number)
                        if number.as_i64().is_none() && number.as_u64().is_none() =>
                    {
                        number
                            .as_f64()
                            .is_some_and(|value| value.is_finite() && value.fract() != 0.0)
                    }
                    _ => false,
                };
            } else {
                entries.next_value::<de::IgnoredAny>()?;
            }
        }
        Ok(RootSchemaVersionProbe {
            has_fractional_schema_version,
        })
    }
}

fn validate_number_lexemes(text: &str) -> Result<(), CodecError> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte == b'-' || byte.is_ascii_digit() {
            let start = index;
            index += 1;
            while index < bytes.len()
                && !matches!(
                    bytes[index],
                    b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}' | b':'
                )
            {
                index += 1;
            }
            let token = &text[start..index];
            if !is_canonical_integer(token) {
                return Err(CodecError::new(
                    CodecErrorCode::NonCanonicalNumber,
                    format!("non-canonical JSON number at byte {start}"),
                ));
            }
            if !is_serde_json_integer_domain(token) {
                return Err(CodecError::new(
                    CodecErrorCode::NonCanonicalNumber,
                    format!("JSON integer is outside the i64/u64 domain at byte {start}"),
                ));
            }
            continue;
        }
        index += 1;
    }
    Ok(())
}

fn is_canonical_integer(token: &str) -> bool {
    if token == "0" {
        return true;
    }
    let digits = token.strip_prefix('-').unwrap_or(token);
    if digits.is_empty()
        || digits.starts_with('0')
        || !digits.as_bytes().iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    true
}

fn is_serde_json_integer_domain(token: &str) -> bool {
    let (negative, digits) = match token.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, token),
    };
    // serde_json represents generic JSON integers as i64 for the negative
    // domain and u64 for the non-negative domain.  Rejecting outside those
    // ranges before typed decoding keeps Rust and the TypeScript BigInt parser
    // on the same wire-level error class.
    let maximum = if negative {
        "9223372036854775808"
    } else {
        "18446744073709551615"
    };
    digits.len() < maximum.len() || (digits.len() == maximum.len() && digits <= maximum)
}

fn parse_without_duplicate_keys(text: &str) -> Result<Value, CodecError> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value = StrictValue::deserialize(&mut deserializer).map_err(classify_parse_error)?;
    deserializer.end().map_err(classify_parse_error)?;
    Ok(value.0)
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("canonical JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::custom(NON_CANONICAL_NUMBER_MARKER))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(StrictValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom(format!(
                    "{}:{}",
                    DUPLICATE_KEY_MARKER, key
                )));
            }
            let value = entries.next_value::<StrictValue>()?;
            values.insert(key, value.0);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}

fn write_canonical(value: &Value, output: &mut String) -> Result<(), CodecError> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => {
            if value.as_i64().is_none() && value.as_u64().is_none() {
                return Err(CodecError::new(
                    CodecErrorCode::NonCanonicalNumber,
                    "floating-point JSON numbers are forbidden",
                ));
            }
            output.push_str(&value.to_string());
        }
        Value::String(value) => {
            output.push_str(&serde_json::to_string(value).map_err(|error| {
                CodecError::new(CodecErrorCode::SchemaViolation, error.to_string())
            })?)
        }
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write_canonical(value, output)?;
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let sorted: BTreeMap<&[u8], (&str, &Value)> = values
                .iter()
                .map(|(key, value)| (key.as_bytes(), (key.as_str(), value)))
                .collect();
            for (index, (_, (key, value))) in sorted.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).map_err(|error| {
                    CodecError::new(CodecErrorCode::SchemaViolation, error.to_string())
                })?);
                output.push(':');
                write_canonical(value, output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

fn classify_parse_error(error: serde_json::Error) -> CodecError {
    let message = error.to_string();
    if message.contains(DUPLICATE_KEY_MARKER) {
        CodecError::new(CodecErrorCode::DuplicateKey, "duplicate JSON object key")
    } else if message.contains(NON_CANONICAL_NUMBER_MARKER) {
        CodecError::new(
            CodecErrorCode::NonCanonicalNumber,
            "floating-point JSON numbers are forbidden",
        )
    } else {
        CodecError::new(CodecErrorCode::InvalidJson, message)
    }
}

fn classify_typed_error(error: serde_json::Error) -> CodecError {
    let message = error.to_string();
    let code = if message.contains("missing field") {
        CodecErrorCode::MissingRequiredField
    } else if message.contains("PID_VALUE_OVERFLOW")
        || message.contains("UNSIGNED_DECIMAL_OVERFLOW")
    {
        CodecErrorCode::ValueOverflow
    } else if message.contains("NON_CANONICAL_UNSIGNED_DECIMAL") {
        CodecErrorCode::NonCanonicalNumber
    } else {
        CodecErrorCode::SchemaViolation
    };
    CodecError::new(code, message)
}

fn classify_validation_error(message: &'static str) -> CodecError {
    let code = if message.contains("LIMIT_EXCEEDED") {
        CodecErrorCode::BoundsExceeded
    } else {
        CodecErrorCode::SchemaViolation
    };
    CodecError::new(code, message)
}
