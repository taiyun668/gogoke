use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use gogoke_public_contracts_s1::*;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureManifest {
    schema_version: u64,
    canonical_positive: Vec<PositiveFixture>,
    negative: Vec<NegativeFixture>,
    native_protocol_sources: Vec<NativeProtocolSource>,
}

#[derive(Debug, Deserialize)]
struct PositiveFixture {
    file: String,
    target: String,
}

#[derive(Debug, Deserialize)]
struct NegativeFixture {
    file: String,
    encoding: String,
    target: String,
    error: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeProtocolSource {
    provider: String,
    source_path: String,
    sha256: String,
    source_anchor: String,
    representative_frame: String,
    expected_error: String,
}

fn fixture(path: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(path),
    )
    .expect("fixture must be readable")
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .expect("contracts/s1 must be below repository root")
        .to_path_buf()
}

fn fixture_text(path: &str) -> Vec<u8> {
    let mut bytes = fixture(path);
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes.pop();
    }
    bytes
}

fn manifest() -> FixtureManifest {
    serde_json::from_slice(&fixture("manifest.json")).expect("fixture manifest must parse")
}

fn schema() -> Value {
    serde_json::from_slice(
        &fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema.json"))
            .expect("schema must be readable"),
    )
    .expect("schema must parse")
}

fn manifest_fixture(entry: &NegativeFixture) -> Vec<u8> {
    let bytes = fixture_text(&entry.file);
    if entry.encoding == "hex" {
        decode_hex(std::str::from_utf8(&bytes).expect("hex fixture is UTF-8"))
    } else {
        bytes
    }
}

fn decode_manifest_target(target: &str, bytes: &[u8]) -> Result<(), CodecError> {
    match target {
        "Session" => decode_json::<Session>(bytes).map(|_| ()),
        "NativeBinding" => decode_json::<NativeBinding>(bytes).map(|_| ()),
        "Execution" => decode_json::<Execution>(bytes).map(|_| ()),
        "Delivery" => decode_json::<Delivery>(bytes).map(|_| ()),
        "PublicEvent" => decode_json::<PublicEvent>(bytes).map(|_| ()),
        "CapabilitySnapshot" => decode_json::<CapabilitySnapshot>(bytes).map(|_| ()),
        "ContextPackage" => decode_json::<ContextPackage>(bytes).map(|_| ()),
        "HumanActionRequest" => decode_json::<HumanActionRequest>(bytes).map(|_| ()),
        "ProcessIdentity" => decode_json::<ProcessIdentity>(bytes).map(|_| ()),
        "OwnershipResult" => decode_json::<OwnershipResult>(bytes).map(|_| ()),
        "CommandSuccess" => decode_json::<CommandSuccess<Value>>(bytes).map(|_| ()),
        "CommandFailure" => decode_json::<CommandFailure>(bytes).map(|_| ()),
        other => panic!("manifest references unknown public target {other}"),
    }
}

fn manifest_error_code(value: &str) -> CodecErrorCode {
    match value {
        "InvalidUtf8" => CodecErrorCode::InvalidUtf8,
        "FrameTooLarge" => CodecErrorCode::FrameTooLarge,
        "DuplicateKey" => CodecErrorCode::DuplicateKey,
        "NonCanonicalNumber" => CodecErrorCode::NonCanonicalNumber,
        "InvalidJson" => CodecErrorCode::InvalidJson,
        "MissingRequiredField" => CodecErrorCode::MissingRequiredField,
        "UnknownMajorVersion" => CodecErrorCode::UnknownMajorVersion,
        "ValueOverflow" => CodecErrorCode::ValueOverflow,
        "BoundsExceeded" => CodecErrorCode::BoundsExceeded,
        "SchemaViolation" => CodecErrorCode::SchemaViolation,
        other => panic!("manifest references unknown codec error {other}"),
    }
}

fn schema_object_matches(definition: &Value, document: &Value) -> bool {
    let Some(object) = document.as_object() else {
        return false;
    };
    let Some(properties) = definition.get("properties").and_then(Value::as_object) else {
        return false;
    };
    let Some(required) = definition.get("required").and_then(Value::as_array) else {
        return false;
    };
    if required.iter().any(|field| {
        field
            .as_str()
            .is_none_or(|field| !object.contains_key(field))
    }) {
        return false;
    }
    if definition.get("additionalProperties") == Some(&Value::Bool(false))
        && object.keys().any(|field| !properties.contains_key(field))
    {
        return false;
    }
    properties.iter().all(|(field, property)| {
        let Some(actual) = object.get(field) else {
            return true;
        };
        if property
            .get("const")
            .is_some_and(|constant| constant != actual)
        {
            return false;
        }
        property
            .get("enum")
            .and_then(Value::as_array)
            .is_none_or(|allowed| allowed.iter().any(|candidate| candidate == actual))
    })
}

fn schema_one_of_matches(schema: &Value, document: &Value) -> Vec<String> {
    let definitions = schema
        .get("$defs")
        .and_then(Value::as_object)
        .expect("schema $defs");
    schema
        .get("oneOf")
        .and_then(Value::as_array)
        .expect("schema oneOf")
        .iter()
        .filter_map(|variant| variant.get("$ref").and_then(Value::as_str))
        .map(|reference| {
            reference
                .strip_prefix("#/$defs/")
                .expect("local schema ref")
        })
        .filter(|name| {
            schema_object_matches(definitions.get(*name).expect("schema definition"), document)
        })
        .map(str::to_owned)
        .collect()
}

fn id(last: u8) -> OpaqueId {
    OpaqueId::parse(format!("00000000-0000-0000-0000-{last:012x}")).expect("test UUID")
}

#[test]
fn canonical_positive_fixtures_round_trip_byte_exactly() {
    let bytes = fixture_text("positive/session.json");
    let session = decode_json::<Session>(&bytes).expect("Session fixture");
    assert_eq!(canonical_json(&session).expect("canonical Session"), bytes);
    assert_eq!(session.revision.get(), 9_007_199_254_740_993);

    let bytes = fixture_text("positive/public-event-no-native-thread-turn.json");
    let event = decode_json::<PublicEvent>(&bytes).expect("provider-neutral event fixture");
    assert_eq!(canonical_json(&event).expect("canonical event"), bytes);
    assert_eq!(event.sequence.get(), u64::MAX);

    let bytes = fixture_text("positive/ownership-unknown.json");
    let ownership = decode_json::<OwnershipResult>(&bytes).expect("unknown ownership fixture");
    assert_eq!(
        canonical_json(&ownership).expect("canonical ownership"),
        bytes
    );
}

#[test]
fn schema_and_manifest_drive_unique_positive_fixture_coverage() {
    let schema = schema();
    let manifest = manifest();
    let schema_version = schema["$defs"]["SchemaVersion"]["const"]
        .as_u64()
        .expect("schema version const");
    assert_eq!(manifest.schema_version, schema_version);

    let variants: Vec<String> = schema["oneOf"]
        .as_array()
        .expect("schema oneOf")
        .iter()
        .map(|variant| {
            variant["$ref"]
                .as_str()
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .expect("local schema oneOf ref")
                .to_owned()
        })
        .collect();
    for variant in &variants {
        assert!(
            manifest
                .canonical_positive
                .iter()
                .any(|entry| entry.target == *variant),
            "schema variant {variant} has no canonical positive fixture"
        );
    }

    for entry in &manifest.canonical_positive {
        let bytes = fixture_text(&entry.file);
        let document: Value = serde_json::from_slice(&bytes).expect(&entry.file);
        assert_eq!(
            schema_one_of_matches(&schema, &document),
            vec![entry.target.clone()],
            "{} must match exactly one schema oneOf variant",
            entry.file
        );
        decode_manifest_target(&entry.target, &bytes).expect(&entry.file);
    }
}

#[test]
fn manifest_positive_fixtures_round_trip_byte_exactly() {
    for entry in &manifest().canonical_positive {
        let bytes = fixture_text(&entry.file);
        let canonical = match entry.target.as_str() {
            "Session" => canonical_json(&decode_json::<Session>(&bytes).expect(&entry.file)),
            "NativeBinding" => {
                canonical_json(&decode_json::<NativeBinding>(&bytes).expect(&entry.file))
            }
            "Execution" => canonical_json(&decode_json::<Execution>(&bytes).expect(&entry.file)),
            "Delivery" => canonical_json(&decode_json::<Delivery>(&bytes).expect(&entry.file)),
            "PublicEvent" => {
                canonical_json(&decode_json::<PublicEvent>(&bytes).expect(&entry.file))
            }
            "CapabilitySnapshot" => {
                canonical_json(&decode_json::<CapabilitySnapshot>(&bytes).expect(&entry.file))
            }
            "ContextPackage" => {
                canonical_json(&decode_json::<ContextPackage>(&bytes).expect(&entry.file))
            }
            "HumanActionRequest" => {
                canonical_json(&decode_json::<HumanActionRequest>(&bytes).expect(&entry.file))
            }
            "ProcessIdentity" => {
                canonical_json(&decode_json::<ProcessIdentity>(&bytes).expect(&entry.file))
            }
            "OwnershipResult" => {
                canonical_json(&decode_json::<OwnershipResult>(&bytes).expect(&entry.file))
            }
            "CommandSuccess" => {
                canonical_json(&decode_json::<CommandSuccess<Value>>(&bytes).expect(&entry.file))
            }
            "CommandFailure" => {
                canonical_json(&decode_json::<CommandFailure>(&bytes).expect(&entry.file))
            }
            other => panic!("manifest references unknown public target {other}"),
        }
        .expect(&entry.file);
        assert_eq!(canonical, bytes, "{}", entry.file);
    }
}

#[test]
fn manifest_negative_fixtures_have_the_declared_error() {
    for entry in &manifest().negative {
        let error =
            decode_manifest_target(&entry.target, &manifest_fixture(entry)).expect_err(&entry.file);
        assert_eq!(
            error.code,
            manifest_error_code(&entry.error),
            "{}: {error}",
            entry.file
        );
    }
}

#[test]
fn utc_timestamps_and_schema_use_one_canonical_grammar() {
    let schema = schema();
    assert_eq!(
        schema["$defs"]["UtcTimestamp"]["pattern"],
        "^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])T([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]([.][0-9]+)?(Z|[+]00:00)$"
    );
    for valid in ["2024-02-29T23:59:59Z", "2026-09-19T16:00:00.123+00:00"] {
        assert_eq!(UtcTimestamp::parse(valid).expect(valid).as_str(), valid);
    }
    for invalid in [
        "2023-02-29T00:00:00Z",
        "2016-12-31T23:59:60Z",
        "2026-09-19T16:00:00-00:00",
        "2026-09-19t16:00:00z",
    ] {
        assert!(UtcTimestamp::parse(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn escaped_utf16_surrogate_pairs_are_valid_but_lone_surrogates_are_invalid_json() {
    let paired = String::from_utf8(fixture_text("positive/session.json"))
        .expect("session fixture UTF-8")
        .replace("S1 public session", "\\uD83D\\uDE00");
    decode_json::<Session>(paired.as_bytes()).expect("paired escaped surrogate");
    for path in [
        "negative/escaped-lone-high-surrogate.json",
        "negative/escaped-lone-low-surrogate.json",
    ] {
        let error = decode_json::<Session>(&fixture_text(path)).expect_err(path);
        assert_eq!(error.code, CodecErrorCode::InvalidJson, "{path}: {error}");
    }
}

#[test]
fn public_frame_title_and_context_limits_are_exact() {
    let mut at_limit = fixture_text("positive/session.json");
    at_limit.resize(MAX_FRAME_BYTES, b' ');
    decode_json::<Session>(&at_limit).expect("exactly 4 MiB frame");
    let over_limit = vec![b' '; MAX_FRAME_BYTES + 1];
    let error = decode_json::<Session>(&over_limit).expect_err("frame over 4 MiB");
    assert_eq!(error.code, CodecErrorCode::FrameTooLarge);

    let mut session: Value =
        serde_json::from_slice(&fixture_text("positive/session.json")).expect("session fixture");
    session["title"] = Value::String("é".repeat(128));
    decode_json::<Session>(&serde_json::to_vec(&session).expect("256-byte title"))
        .expect("256-byte title");
    session["title"] = Value::String(format!("{}a", "é".repeat(128)));
    let error = decode_json::<Session>(&serde_json::to_vec(&session).expect("257-byte title"))
        .expect_err("257-byte title");
    assert_eq!(error.code, CodecErrorCode::BoundsExceeded);

    let mut context: Value = serde_json::from_slice(&fixture_text("positive/context-package.json"))
        .expect("context fixture");
    let item = context["items"][0].clone();
    context["items"] = Value::Array(vec![item.clone(); MAX_CONTEXT_ITEMS]);
    decode_json::<ContextPackage>(&serde_json::to_vec(&context).expect("128 context items"))
        .expect("128 context items");
    context["items"] = Value::Array(vec![item; MAX_CONTEXT_ITEMS + 1]);
    let error =
        decode_json::<ContextPackage>(&serde_json::to_vec(&context).expect("129 context items"))
            .expect_err("129 context items");
    assert_eq!(error.code, CodecErrorCode::BoundsExceeded);
}

#[test]
fn legacy_native_frames_are_digest_pinned_and_never_public_authority() {
    let manifest = manifest();
    assert_eq!(manifest.native_protocol_sources.len(), 5);
    let providers: BTreeSet<&str> = manifest
        .native_protocol_sources
        .iter()
        .map(|source| source.provider.as_str())
        .collect();
    assert_eq!(providers, BTreeSet::from(["claude", "codex", "grok"]));

    for source in &manifest.native_protocol_sources {
        assert!(
            source
                .source_path
                .starts_with("apps/desktop/contracts/s1/fixtures/native-protocol-sources/")
                && !source.source_path.contains(".."),
            "{}",
            source.source_path
        );
        let bytes = fs::read(repository_root().join(&source.source_path))
            .unwrap_or_else(|error| panic!("{}: {error}", source.source_path));
        assert_eq!(sha256_hex(&bytes), source.sha256, "{}", source.source_path);
        let text = std::str::from_utf8(&bytes).expect("native source must be UTF-8");
        assert!(
            text.contains(&source.source_anchor),
            "{} source anchor",
            source.source_path
        );
        serde_json::from_str::<Value>(&source.representative_frame)
            .unwrap_or_else(|error| panic!("{} representative frame: {error}", source.source_path));
        let error = decode_json::<PublicEvent>(source.representative_frame.as_bytes())
            .expect_err(&source.source_path);
        assert_eq!(
            error.code,
            manifest_error_code(&source.expected_error),
            "{} must not decode as public authority",
            source.source_path
        );
    }
}

#[test]
fn escaped_schema_version_key_is_the_decoded_top_level_field() {
    let canonical = fixture_text("positive/session.json");
    let escaped = String::from_utf8(canonical.clone())
        .expect("session fixture is UTF-8")
        .replace("\"schemaVersion\"", "\"schema\\u0056ersion\"");
    let decoded = decode_json::<Session>(escaped.as_bytes()).expect("escaped key version 1");
    assert_eq!(
        canonical_json(&decoded).expect("canonical session"),
        canonical
    );

    let error = decode_json::<Session>(&fixture_text(
        "negative/schema-version-escaped-key-fractional.json",
    ))
    .expect_err("escaped key fractional version");
    assert_eq!(error.code, CodecErrorCode::SchemaViolation);
}

#[test]
fn duplicate_key_precedes_schema_number_semantics_in_both_source_orders() {
    for path in [
        "negative/duplicate-before-schema-version-fractional.json",
        "negative/schema-version-fractional-before-duplicate.json",
    ] {
        let error = decode_json::<Session>(&fixture_text(path)).expect_err(path);
        assert_eq!(error.code, CodecErrorCode::DuplicateKey, "{path}: {error}");
    }

    let path = "negative/generic-integer-overflow-before-escaped-duplicate.json";
    let error = decode_json::<PublicEvent>(&fixture_text(path)).expect_err(path);
    assert_eq!(error.code, CodecErrorCode::DuplicateKey, "{path}: {error}");
}

#[test]
fn required_negative_fixtures_fail_closed() {
    let cases = [
        ("negative/duplicate-key.json", CodecErrorCode::DuplicateKey),
        (
            "negative/noncanonical-number.json",
            CodecErrorCode::NonCanonicalNumber,
        ),
        (
            "negative/missing-required.json",
            CodecErrorCode::MissingRequiredField,
        ),
        (
            "negative/overflow-counter.json",
            CodecErrorCode::ValueOverflow,
        ),
        (
            "negative/unknown-major.json",
            CodecErrorCode::UnknownMajorVersion,
        ),
        (
            "negative/unknown-field.json",
            CodecErrorCode::SchemaViolation,
        ),
        (
            "negative/invalid-enum.json",
            CodecErrorCode::SchemaViolation,
        ),
        (
            "negative/invalid-uuid.json",
            CodecErrorCode::SchemaViolation,
        ),
        (
            "negative/leading-zero-counter.json",
            CodecErrorCode::NonCanonicalNumber,
        ),
    ];
    for (path, expected) in cases {
        let error = decode_json::<Session>(&fixture_text(path)).expect_err(path);
        assert_eq!(error.code, expected, "{path}: {error}");
    }

    let error = decode_json::<PublicEvent>(&fixture_text("negative/wrong-nullable.json"))
        .expect_err("wrong nullable type");
    assert_eq!(error.code, CodecErrorCode::SchemaViolation);

    let error = decode_json::<CommandSuccess<serde_json::Value>>(&fixture_text(
        "negative/missing-receipt.json",
    ))
    .expect_err("missing required receipt");
    assert_eq!(error.code, CodecErrorCode::MissingRequiredField);

    let error = decode_json::<ProcessIdentity>(&fixture_text("negative/pid-string.json"))
        .expect_err("pid string");
    assert_eq!(error.code, CodecErrorCode::SchemaViolation);

    let error = decode_json::<ProcessIdentity>(&fixture_text("negative/pid-overflow.json"))
        .expect_err("pid overflow");
    assert_eq!(error.code, CodecErrorCode::ValueOverflow);

    let hex = String::from_utf8(fixture_text("negative/invalid-utf8.hex")).expect("hex text");
    let bytes = decode_hex(&hex);
    let error = decode_json::<Session>(&bytes).expect_err("invalid UTF-8");
    assert_eq!(error.code, CodecErrorCode::InvalidUtf8);
}

#[test]
fn generic_json_integer_domain_matches_serde_json() {
    for token in [
        "-9223372036854775808",
        "9223372036854775807",
        "9223372036854775808",
        "18446744073709551615",
    ] {
        let wire = format!(
            r#"{{"bindingId":null,"eventId":"00000000-0000-0000-0000-000000000011","executionId":null,"generation":"0","kind":"diagnostic","payload":{{"value":{token}}},"schemaVersion":1,"sequence":"1","sessionId":"00000000-0000-0000-0000-000000000001","streamEpoch":"1"}}"#
        );
        decode_json::<PublicEvent>(wire.as_bytes()).expect(token);
    }
    for token in ["-9223372036854775809", "18446744073709551616"] {
        let wire = format!(
            r#"{{"bindingId":null,"eventId":"00000000-0000-0000-0000-000000000011","executionId":null,"generation":"0","kind":"diagnostic","payload":{{"value":{token}}},"schemaVersion":1,"sequence":"1","sessionId":"00000000-0000-0000-0000-000000000001","streamEpoch":"1"}}"#
        );
        let error = decode_json::<PublicEvent>(wire.as_bytes()).expect_err(token);
        assert_eq!(
            error.code,
            CodecErrorCode::NonCanonicalNumber,
            "{token}: {error}"
        );
    }
}

#[test]
fn decimal_counter_rejects_all_noncanonical_or_overflow_spellings() {
    for revision in [
        "",
        "00",
        "01",
        "+1",
        "-1",
        " 1",
        "1.0",
        "18446744073709551616",
    ] {
        let wire = format!(
            r#"{{"lifecycle":"active","privacyDomainId":"00000000-0000-0000-0000-000000000003","revision":"{revision}","schemaVersion":1,"scopeId":"00000000-0000-0000-0000-000000000002","sessionId":"00000000-0000-0000-0000-000000000001","title":"counter"}}"#
        );
        assert!(
            decode_json::<Session>(wire.as_bytes()).is_err(),
            "{revision}"
        );
    }
}

#[test]
fn nullable_fields_are_required_and_null_is_distinct_from_missing() {
    let missing_execution_id = br#"{"bindingId":null,"eventId":"00000000-0000-0000-0000-000000000011","generation":"0","kind":"session_updated","payload":{},"schemaVersion":1,"sequence":"1","sessionId":"00000000-0000-0000-0000-000000000001","streamEpoch":"1"}"#;
    let error =
        decode_json::<PublicEvent>(missing_execution_id).expect_err("missing nullable field");
    assert_eq!(error.code, CodecErrorCode::MissingRequiredField);
}

#[test]
fn every_c02_object_has_a_provider_neutral_canonical_round_trip() {
    let session = Session {
        schema_version: SCHEMA_VERSION,
        session_id: id(1),
        scope_id: id(2),
        privacy_domain_id: id(3),
        title: "session".into(),
        lifecycle: SessionLifecycle::Active,
        revision: DecimalU64::new(1),
    };
    round_trip(&session);

    let binding = NativeBinding {
        schema_version: SCHEMA_VERSION,
        binding_id: id(4),
        session_id: id(1),
        driver_id: "fake".into(),
        instance_id: id(5),
        profile_revision: DecimalU64::new(2),
        auth_revision: DecimalU64::new(3),
        domain_id: id(3),
        generation: DecimalU64::new(4),
        native_session_id: None,
        continuation_mode: ContinuationMode::New,
    };
    round_trip(&binding);

    let execution = Execution {
        schema_version: SCHEMA_VERSION,
        execution_id: id(6),
        session_id: id(1),
        binding_id: id(4),
        generation: DecimalU64::new(4),
        state: ExecutionState::Running,
        created_at: UtcTimestamp::parse("2026-09-19T16:00:00Z").unwrap(),
        completed_at: None,
        result_ref: None,
    };
    round_trip(&execution);

    let delivery = Delivery {
        schema_version: SCHEMA_VERSION,
        operation_id: id(7),
        execution_id: id(6),
        session_id: id(1),
        binding_id: id(4),
        generation: DecimalU64::new(4),
        request_fingerprint: "sha256:fixture".into(),
        intent_kind: "delivery.send".into(),
        acceptance_state: AcceptanceState::Recorded,
        durable_receipt_id: None,
        native_receipt: None,
    };
    round_trip(&delivery);

    let event = PublicEvent {
        schema_version: SCHEMA_VERSION,
        event_id: id(8),
        session_id: id(1),
        execution_id: Some(id(6)),
        binding_id: Some(id(4)),
        generation: DecimalU64::new(4),
        stream_epoch: DecimalU64::new(1),
        sequence: DecimalU64::new(u64::MAX),
        kind: PublicEventKind::ExecutionStarted,
        payload: json!({"sourceContinuity":"unknown"}),
    };
    round_trip(&event);

    let capability = CapabilitySnapshot {
        schema_version: SCHEMA_VERSION,
        snapshot_id: id(9),
        instance_id: id(5),
        executable_id: ExecutableId {
            path: "X:/synthetic/fake.exe".into(),
            hash: "sha256:fake".into(),
            version: "1.0.0".into(),
            platform: "windows".into(),
        },
        mode: "synthetic".into(),
        profile_revision: DecimalU64::new(2),
        auth_revision: DecimalU64::new(3),
        observed_at: UtcTimestamp::parse("2026-09-19T16:00:00Z").unwrap(),
        expires_at: UtcTimestamp::parse("2026-09-19T16:05:00Z").unwrap(),
        evidence_source: "fixture".into(),
        capabilities: BTreeMap::from([(
            "streaming".into(),
            CapabilityEvidence {
                state: CapabilityState::Unknown,
                reason: "not probed".into(),
                evidence_ref: None,
            },
        )]),
    };
    round_trip(&capability);

    let context = ContextPackage {
        schema_version: SCHEMA_VERSION,
        package_id: id(10),
        recipient: "reviewer".into(),
        scope_id: id(2),
        domain_id: id(3),
        task_id: "synthetic-task".into(),
        source_version: "sha256:source".into(),
        items: vec![ContextItem {
            reference: "object:1".into(),
            digest: "sha256:item".into(),
            visibility: "selected".into(),
        }],
        permission_ceiling: BTreeMap::new(),
        expires_at: UtcTimestamp::parse("2026-09-19T17:00:00Z").unwrap(),
    };
    round_trip(&context);

    let action = HumanActionRequest {
        schema_version: SCHEMA_VERSION,
        request_id: id(11),
        execution_id: id(6),
        binding_id: id(4),
        generation: DecimalU64::new(4),
        continuation_id: "provider-continuation".into(),
        domain_id: id(3),
        expires_at: UtcTimestamp::parse("2026-09-19T17:00:00Z").unwrap(),
        allowed_answers: vec!["approve".into(), "deny".into()],
        permission_ceiling: BTreeMap::new(),
        state: HumanActionState::Pending,
    };
    round_trip(&action);

    let process = ProcessIdentity {
        schema_version: SCHEMA_VERSION,
        host_id: id(12),
        instance_id: id(5),
        process_handle_ref: "owned-handle:1".into(),
        pid: 42,
        started_at_ticks: None,
        launch_epoch: DecimalU64::new(1),
        image_digest: "sha256:image".into(),
        containment_id: None,
    };
    round_trip(&process);

    let ownership = OwnershipResult {
        schema_version: SCHEMA_VERSION,
        observation: ObservationState::Unknown,
        in_memory_state: RecordedState::Unknown,
        persisted_state: RecordedState::Unknown,
        durability_ack: DurabilityAckState::Unknown,
        custody_state: CustodyState::Retained,
        errors: vec![],
    };
    round_trip(&ownership);
}

fn round_trip<T>(value: &T)
where
    T: PublicDocument + std::fmt::Debug + PartialEq,
{
    let bytes = canonical_json(value).expect("canonical encode");
    let decoded: T = decode_json(&bytes).expect("strict decode");
    assert_eq!(&decoded, value);
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(pair, 16).unwrap()
        })
        .collect()
}

fn sha256_hex(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (input.len() as u64) * 8;
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for block in padded.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let start = index * 4;
            *word = u32::from_be_bytes(block[start..start + 4].try_into().expect("SHA word"));
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }

    state.iter().map(|word| format!("{word:08x}")).collect()
}
