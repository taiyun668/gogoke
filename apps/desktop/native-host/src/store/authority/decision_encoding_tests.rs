//! Pure native JSON encoding regression definitions; no provider or database.
//! They are not execution evidence until the native Rust test binary runs.
use super::{durable_record_json, json_string, DurableDecisionRecord};

fn record() -> DurableDecisionRecord {
    DurableDecisionRecord {
        operation_id: "operation-one".into(), scenario_id: "DF02".into(),
        family: "RESOURCE_SELECTION".into(), state_view_hash: "view-one".into(),
        candidate_hash: "candidates-one".into(), question_version: "1".into(),
        rubric_version: "1".into(), model_requested: None, model_resolved: None,
        task_revision: "1".into(), policy_revision: "1".into(), capability_revision: "1".into(),
        binding_generation: "1".into(), backend_kind: "RULES".into(), choice: "candidate-one".into(),
        reason: "QUALIFIED_BOUNDED_SELECTION".into(), budget_units: 7, deadline_epoch_ms: 1000,
    }
}
const GOLDEN: &str = r#"{"backendKind":"RULES","bindingGeneration":"1","budgetUnits":"7","candidateHash":"candidates-one","capabilityRevision":"1","choice":"candidate-one","deadlineEpochMs":"1000","family":"RESOURCE_SELECTION","modelRequested":null,"modelResolved":null,"operationId":"operation-one","policyRevision":"1","questionVersion":"1","reason":"QUALIFIED_BOUNDED_SELECTION","rubricVersion":"1","scenarioId":"DF02","state":"COMMITTED","stateViewHash":"view-one","taskRevision":"1"}"#;

#[test]
fn encoding_empty_string_has_two_delimiters() {
    assert_eq!(json_string(""), "\"\"");
}
#[test]
fn encoding_quote_is_escaped_not_a_new_field() {
    assert_eq!(json_string("\""), "\"\\\"\"");
}
#[test]
fn encoding_backslash_is_escaped_once() {
    assert_eq!(json_string("\\"), "\"\\\\\"");
}
#[test]
fn encoding_short_controls_use_json_spelling() {
    assert_eq!(json_string("\u{0008}\u{000c}\n\r\t"), "\"\\b\\f\\n\\r\\t\"");
}
#[test]
fn encoding_other_controls_use_four_digit_lowercase_hex() {
    for value in 0u32..32 {
        if [8, 9, 10, 12, 13].contains(&value) { continue; }
        let text = char::from_u32(value).unwrap().to_string();
        assert_eq!(json_string(&text), format!("\"\\u{value:04x}\""));
    }
}
#[test]
fn encoding_unicode_and_forward_slash_remain_utf8() {
    assert_eq!(json_string("上下文/😀"), "\"上下文/😀\"");
}
#[test]
fn encoding_injection_shaped_string_stays_one_string_value() {
    assert_eq!(json_string("\",\"state\":\"COMMITTED"), "\"\\\",\\\"state\\\":\\\"COMMITTED\"");
}
#[test]
fn encoding_default_durable_record_matches_exact_canonical_bytes() {
    assert_eq!(durable_record_json(&record()), GOLDEN);
}
#[test]
fn encoding_safe_integer_limits_remain_lossless_decimal_strings() {
    let mut value = record(); value.budget_units = 9_007_199_254_740_991;
    value.deadline_epoch_ms = 9_007_199_254_740_991;
    assert_eq!(durable_record_json(&value), GOLDEN
        .replace("\"budgetUnits\":\"7\"", "\"budgetUnits\":\"9007199254740991\"")
        .replace("\"deadlineEpochMs\":\"1000\"", "\"deadlineEpochMs\":\"9007199254740991\""));
}
#[test]
fn encoding_model_none_and_literal_none_are_not_conflated() {
    let mut value = record(); value.model_requested = Some("NONE".into());
    value.model_resolved = Some("model\"quoted".into());
    assert_eq!(durable_record_json(&value), GOLDEN
        .replace("\"modelRequested\":null", "\"modelRequested\":\"NONE\"")
        .replace("\"modelResolved\":null", "\"modelResolved\":\"model\\\"quoted\""));
}
