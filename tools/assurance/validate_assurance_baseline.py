#!/usr/bin/env python3
"""Validate the P26 machine-readable assurance baseline using only stdlib."""

from __future__ import annotations

import argparse
import copy
import json
import re
import sys
from pathlib import Path
from typing import Any

THREAT_ID = re.compile(r"^TR-[0-9]{3}$")
AUDIT_ID = re.compile(r"^AUD-[0-9]{3}$")
EVIDENCE_ID = re.compile(r"^EV-[0-9]{3}$")
CONTROL_ID = re.compile(r"^CTRL-[0-9]{3}$")
GATE_ID = re.compile(r"^GATE-[A-Z0-9-]+$")
ALLOWED_RISK_STATUSES = {"OPEN", "ACCEPTED_BY_OWNER", "NOT_APPLICABLE"}
ALLOWED_EVIDENCE_STATUSES = {"NOT_RUN", "INCONCLUSIVE", "PASS", "FAIL", "WARN", "BLOCKED"}
ALLOWED_PRODUCT_STATUSES = {"INCONCLUSIVE", "NOT_RUN", "BLOCKED", "FAIL", "WARN"}
FORBIDDEN_AUDIT_FIELDS = {"secret", "secret_value", "token", "cookie", "password", "private_key", "raw_credential"}


def read_json(path: Path, errors: list[str]) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        errors.append(f"{path}: cannot read valid JSON: {exc}")
        return {}
    if not isinstance(value, dict):
        errors.append(f"{path}: top level must be an object")
        return {}
    return value


def required_text(record: dict[str, Any], field: str, location: str, errors: list[str]) -> None:
    value = record.get(field)
    if not isinstance(value, str) or not value.strip() or value.strip().upper() in {"TBD", "TODO", "UNASSIGNED"}:
        errors.append(f"{location}: missing usable {field}")


def unique_ids(records: list[Any], pattern: re.Pattern[str], label: str, errors: list[str]) -> None:
    seen: set[str] = set()
    for index, record in enumerate(records):
        location = f"{label}[{index}]"
        if not isinstance(record, dict):
            errors.append(f"{location}: must be an object")
            continue
        identifier = record.get("id")
        if not isinstance(identifier, str) or not pattern.fullmatch(identifier):
            errors.append(f"{location}: invalid stable id {identifier!r}")
            continue
        if identifier in seen:
            errors.append(f"{location}: duplicate id {identifier}")
        seen.add(identifier)


def globally_unique_nested_ids(threats: list[Any], field: str, pattern: re.Pattern[str], errors: list[str]) -> None:
    """Reject a repeated control, evidence, or gate ID anywhere in the register."""
    seen: set[str] = set()
    for index, threat in enumerate(threats):
        if not isinstance(threat, dict):
            continue
        records = [threat.get(field)] if field == "gate" else threat.get(field, [])
        if not isinstance(records, list):
            continue
        for nested in records:
            if not isinstance(nested, dict):
                continue
            identifier = nested.get("id")
            if not isinstance(identifier, str) or not pattern.fullmatch(identifier):
                continue
            if identifier in seen:
                errors.append(f"threats[{index}].{field}: duplicate stable id {identifier}")
            seen.add(identifier)


def validate_threats(register: dict[str, Any], errors: list[str]) -> int:
    if register.get("schema") != "gogo.security.threat-register.v1":
        errors.append("threat register: unexpected schema")
    if register.get("document_status") != "BASELINE_ONLY":
        errors.append("threat register: document_status must be BASELINE_ONLY")
    if set(register.get("risk_status_vocabulary", [])) != ALLOWED_RISK_STATUSES:
        errors.append("threat register: risk_status_vocabulary must exactly declare the approved values")
    if register.get("product_assurance_status") != "INCONCLUSIVE":
        errors.append("threat register: baseline product_assurance_status must remain INCONCLUSIVE")

    threats = register.get("threats")
    if not isinstance(threats, list) or not threats:
        errors.append("threat register: threats must be a non-empty array")
        return 0
    unique_ids(threats, THREAT_ID, "threats", errors)
    globally_unique_nested_ids(threats, "controls", CONTROL_ID, errors)
    globally_unique_nested_ids(threats, "evidence", EVIDENCE_ID, errors)
    globally_unique_nested_ids(threats, "gate", GATE_ID, errors)
    for index, threat in enumerate(threats):
        if not isinstance(threat, dict):
            continue
        location = f"threats[{index}]"
        for field in ("owner", "threat", "risk_status", "product_assurance"):
            required_text(threat, field, location, errors)
        for field in ("asset", "boundary", "dependencies"):
            value = threat.get(field)
            if not isinstance(value, list) or not value or not all(isinstance(item, str) and item.strip() for item in value):
                errors.append(f"{location}: {field} must be a non-empty string array")
        if threat.get("risk_status") not in ALLOWED_RISK_STATUSES:
            errors.append(f"{location}: unapproved risk_status {threat.get('risk_status')!r}")
        if threat.get("product_assurance") not in ALLOWED_PRODUCT_STATUSES:
            errors.append(f"{location}: product_assurance {threat.get('product_assurance')!r} cannot claim PASS")
        if threat.get("risk_status") == "ACCEPTED_BY_OWNER" and not isinstance(threat.get("risk_acceptance"), dict):
            errors.append(f"{location}: ACCEPTED_BY_OWNER requires a risk_acceptance record")

        controls = threat.get("controls")
        if not isinstance(controls, list) or not controls:
            errors.append(f"{location}: missing control")
        else:
            unique_ids(controls, CONTROL_ID, f"{location}.controls", errors)
            for control in controls:
                if isinstance(control, dict):
                    required_text(control, "description", location + ".controls", errors)

        evidence = threat.get("evidence")
        not_run = False
        if not isinstance(evidence, list) or not evidence:
            errors.append(f"{location}: missing evidence requirement")
        else:
            unique_ids(evidence, EVIDENCE_ID, f"{location}.evidence", errors)
            for item in evidence:
                if isinstance(item, dict):
                    required_text(item, "requirement", location + ".evidence", errors)
                    status = item.get("status")
                    if status not in ALLOWED_EVIDENCE_STATUSES:
                        errors.append(f"{location}.evidence: invalid status {status!r}")
                    not_run = not_run or status == "NOT_RUN"

        gate = threat.get("gate")
        if not isinstance(gate, dict):
            errors.append(f"{location}: missing gate")
        else:
            gate_id = gate.get("id")
            if not isinstance(gate_id, str) or not GATE_ID.fullmatch(gate_id):
                errors.append(f"{location}.gate: invalid stable id {gate_id!r}")
            gate_status = gate.get("current_status")
            if gate_status not in ALLOWED_PRODUCT_STATUSES:
                errors.append(f"{location}.gate: current_status {gate_status!r} cannot claim PASS")
            if not_run and gate_status == "PASS":
                errors.append(f"{location}: NOT_RUN evidence must not be presented as PASS")
    return len(threats)


def validate_audit_vocabulary(vocabulary: dict[str, Any], errors: list[str]) -> int:
    if vocabulary.get("schema") != "gogo.security.audit-vocabulary.v1":
        errors.append("audit vocabulary: unexpected schema")
    if vocabulary.get("document_status") != "BASELINE_ONLY":
        errors.append("audit vocabulary: document_status must be BASELINE_ONLY")
    if set(vocabulary.get("forbidden_field_names", [])) != FORBIDDEN_AUDIT_FIELDS:
        errors.append("audit vocabulary: forbidden_field_names must exactly declare secret-bearing fields")
    common = vocabulary.get("required_common_fields")
    if not isinstance(common, list) or not common:
        errors.append("audit vocabulary: required_common_fields must be non-empty")
    events = vocabulary.get("events")
    if not isinstance(events, list) or not events:
        errors.append("audit vocabulary: events must be a non-empty array")
        return 0
    unique_ids(events, AUDIT_ID, "events", errors)
    names: set[str] = set()
    for index, event in enumerate(events):
        if not isinstance(event, dict):
            continue
        location = f"events[{index}]"
        for field in ("name", "authoritative_plane", "producer"):
            required_text(event, field, location, errors)
        name = event.get("name")
        if isinstance(name, str):
            if name in names:
                errors.append(f"{location}: duplicate event name {name}")
            names.add(name)
        fields = event.get("required_fields")
        if not isinstance(fields, list) or not fields or not all(isinstance(item, str) and item.strip() for item in fields):
            errors.append(f"{location}: required_fields must be a non-empty string array")
        elif FORBIDDEN_AUDIT_FIELDS.intersection(fields):
            errors.append(f"{location}: required_fields must not contain a secret-bearing field")
    return len(events)


def validate(root: Path) -> tuple[list[str], int, int]:
    errors: list[str] = []
    register = read_json(root / "docs/security/threat-register.json", errors)
    vocabulary = read_json(root / "docs/security/audit-vocabulary.json", errors)
    threat_count = validate_threats(register, errors) if register else 0
    event_count = validate_audit_vocabulary(vocabulary, errors) if vocabulary else 0
    return errors, threat_count, event_count


def run_self_test(root: Path) -> list[str]:
    register = json.loads((root / "docs/security/threat-register.json").read_text(encoding="utf-8"))
    cases: list[tuple[str, Any]] = [
        ("missing owner", lambda value: value["threats"][0].pop("owner")),
        ("duplicate threat id", lambda value: value["threats"][1].__setitem__("id", "TR-001")),
        ("duplicate evidence id", lambda value: value["threats"][1]["evidence"][0].__setitem__("id", "EV-001")),
        ("unapproved risk status", lambda value: value["threats"][0].__setitem__("risk_status", "MITIGATED")),
        ("NOT_RUN masquerades as PASS", lambda value: value["threats"][0]["gate"].__setitem__("current_status", "PASS")),
    ]
    failures: list[str] = []
    for name, mutate in cases:
        candidate = copy.deepcopy(register)
        mutate(candidate)
        errors: list[str] = []
        validate_threats(candidate, errors)
        if not errors:
            failures.append(name)
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2], help="workspace root")
    parser.add_argument("--self-test", action="store_true", help="exercise required rejection cases without writing files")
    args = parser.parse_args()
    root = args.root.resolve()
    errors, threat_count, event_count = validate(root)
    if args.self_test and not errors:
        for name in run_self_test(root):
            errors.append(f"self-test did not reject: {name}")
    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        return 1
    print(f"PASS: assurance baseline structure validated ({threat_count} threats, {event_count} audit events)")
    print("PRODUCT_ASSURANCE: INCONCLUSIVE (no product-security or RC verdict is implied)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
