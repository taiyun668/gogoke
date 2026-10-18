use super::*;

#[test]
fn host_original_request_records_actor_cause_and_route_without_a_fake_turn() {
    let fields = HostFields { domain: "projectA", escalation_request: "host-escalate-A",
        trigger: "host-reject-cap-A", cause: "gate-decision-A", source: "seatA",
        destination: "seatB", policy_revision: 9, route_revision: 7,
        body: "Host rule REJECT_CAP: fixed notice" };
    let ids = identity_fields(&fields);
    let raw = original_request_fields(&fields, &ids, "enqueue", None);
    let text = std::str::from_utf8(raw.strip_suffix(b"\n").unwrap()).unwrap();
    let Json::Object(original) = Parser::parse(text).unwrap() else { panic!("object"); };
    assert_eq!(internal_text(&original, "actor").unwrap(), HOST_RULE_ACTOR);
    assert_eq!(internal_text(&original, "sourceSeatId").unwrap(), "seatA");
    assert_eq!(internal_text(&original, "destinationSeatId").unwrap(), "seatB");
    assert_eq!(internal_text(&original, "causeEventId").unwrap(), "gate-decision-A");
    assert_eq!(internal_text(&original, "routeRevision").unwrap(), "7");
    assert!(!original.contains_key(&JsonString::from_str("turnId")));
    assert!(!original.contains_key(&JsonString::from_str("generation")));
    assert_eq!(raw, original_request_fields(&fields, &ids, "enqueue", None));
}
