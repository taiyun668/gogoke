"""Build the MC-125 qualification manifest from fixed PLAN bytes.

This is an evidence/traceability instrument.  It deliberately keeps the
PLAN's official due status separate from runner readiness and from any local
harness observations.  The fixed PLAN is read with ``git show`` so a checkout
with CRLF or moving execution files cannot silently change the 59-check map.
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import platform
import re
import subprocess
import sys
import zipfile
from pathlib import Path
from typing import Any, Iterable


PLAN_SHA = "cbdc6ad592947370941024a87dbb9168a5b59055"
FROZEN_SOURCE_SHA = "37429770239dd23e91cc41a8c7420be7a9cb162b"
PACKAGE_START_SHA = "a481d65c7e24b298cba796bdc987eeabbe594b42"
PLAN_ROOT = "docs/design/gogoke-s1-r4-plan-v1"
PLAN_FILES = (
    "CHECKS.json",
    "EXECUTION_PLAN.json",
    "CAPABILITY_TASK_MAP.json",
    "QUALIFICATION_TARGETS.json",
    "GATES_AUTHORIZATION.json",
)
REGISTRY_PATH = "tools/gogoke-s1-r4/registry.json"
RUNNER_PATH = "tools/gogoke-s1-r4/run_checks.py"
OUTPUT_RELATIVE = "artifacts/s1-r4/parallel/qualification-infra-mc125"


def repo_root() -> Path:
    # tools/gogoke-s1-r4/<this file> -> repository root
    return Path(__file__).resolve().parents[2]


ROOT = repo_root()


def run_git(*args: str) -> bytes:
    result = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"git {' '.join(args)} failed ({result.returncode}): {detail}")
    return result.stdout


def fixed_plan_blob(name: str) -> bytes:
    return run_git("show", f"{PLAN_SHA}:{PLAN_ROOT}/{name}")


def json_bytes(data: bytes, label: str) -> Any:
    def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON object key {key!r}")
            result[key] = value
        return result

    try:
        return json.loads(data.decode("utf-8-sig"), object_pairs_hook=reject_duplicate_keys)
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as exc:
        raise RuntimeError(f"invalid JSON in {label}: {exc}") from exc


def read_json(path: Path) -> tuple[Any, bytes]:
    raw = path.read_bytes()
    return json_bytes(raw, str(path)), raw


def sha256_bytes(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def sha256_file(path: Path) -> str | None:
    if not path.is_file():
        return None
    return sha256_bytes(path.read_bytes())


def unique(values: Iterable[str]) -> list[str]:
    return list(dict.fromkeys(str(value) for value in values if value))


def command_text(argv: Iterable[Any]) -> str:
    # Registry argv is already the canonical command vector.  Keep literal
    # placeholders such as {{OUTPUT}} instead of shell re-quoting them.
    return " ".join(str(part) for part in argv)


def safe_command(argv: list[str], timeout: float = 5.0) -> dict[str, Any]:
    try:
        result = subprocess.run(
            argv,
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
            timeout=timeout,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
    except (FileNotFoundError, OSError) as exc:
        return {"argv": argv, "available": False, "exit_code": None, "output": str(exc)}
    except subprocess.TimeoutExpired as exc:
        return {
            "argv": argv,
            "available": True,
            "exit_code": None,
            "output": (exc.stdout or "")[-4000:],
            "timed_out": True,
        }
    return {
        "argv": argv,
        "available": True,
        "exit_code": result.returncode,
        "output": result.stdout.strip()[-4000:],
    }


def toolchain_identity() -> dict[str, Any]:
    commands = {
        "python": [sys.executable, "--version"],
        "node": ["node", "--version"],
        "pnpm": ["pnpm", "--version"],
        "cargo": ["cargo", "--version"],
        "rustc": ["rustc", "--version"],
    }
    return {name: safe_command(argv) for name, argv in commands.items()}


def parse_tap(path: Path) -> dict[str, Any] | None:
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8", errors="replace")
    tests = re.search(r"^# tests (\d+)\s*$", text, re.MULTILINE)
    passed = re.search(r"^# pass (\d+)\s*$", text, re.MULTILINE)
    failed = re.search(r"^# fail (\d+)\s*$", text, re.MULTILINE)
    if not tests or not passed or not failed:
        return {
            "path": str(path.relative_to(ROOT)).replace("\\", "/"),
            "parse_status": "INSTRUMENT_ERROR",
            "positive_count": None,
            "exit_code": None,
        }
    test_count = int(tests.group(1))
    pass_count = int(passed.group(1))
    fail_count = int(failed.group(1))
    return {
        "path": str(path.relative_to(ROOT)).replace("\\", "/"),
        "parse_status": "PARSED_LOG_ONLY",
        "positive_count": pass_count,
        "test_count": test_count,
        "failed_count": fail_count,
        "exit_code_inferred": 0 if fail_count == 0 and pass_count == test_count else 1,
        "evidence_layer": "FIXTURE_SOURCE_ASSERTION_FAKE_DEPENDENCY",
        "qualifies_actual_integrated": False,
        "limits": [
            "The log binds synthetic fixture assertions and fake dependency recording only.",
            "It does not qualify Windows native behavior, Owner-machine predicates, or a due check.",
        ],
    }


def parse_unittest(path: Path) -> dict[str, Any] | None:
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8", errors="replace")
    ran = re.search(r"^Ran (\d+) tests? in ", text, re.MULTILINE)
    success = bool(re.search(r"^OK\s*$", text, re.MULTILINE))
    if not ran:
        return {
            "path": str(path.relative_to(ROOT)).replace("\\", "/"),
            "parse_status": "INSTRUMENT_ERROR",
            "positive_count": None,
            "exit_code": None,
        }
    count = int(ran.group(1))
    return {
        "path": str(path.relative_to(ROOT)).replace("\\", "/"),
        "parse_status": "PARSED_LOG_ONLY",
        "positive_count": count if success else 0,
        "test_count": count,
        "failed_count": 0 if success else None,
        "exit_code_inferred": 0 if success else 1,
        "evidence_layer": "QUALIFICATION_HARNESS_SELF_TEST",
        "qualifies_actual_integrated": False,
        "limits": [
            "This validates qualification harness behavior, not the product due checks.",
            "Harness self-tests cannot be substituted for an actual integrated receipt.",
        ],
    }


def path_text(path: str) -> str:
    return path.replace("\\", "/")


def is_test_surface(path: str) -> bool:
    normalized = path_text(path).lower()
    return (
        "/tests/" in f"/{normalized}"
        or "/test-fixtures/" in f"/{normalized}"
        or normalized.endswith(".test.ts")
        or normalized.endswith(".test.mjs")
        or normalized.endswith("_test.rs")
        or normalized.startswith("tools/gogoke-s1-r4/test_")
    )


def dedupe_records(records: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    seen: set[str] = set()
    output: list[dict[str, Any]] = []
    for record in records:
        key = json.dumps(record, sort_keys=True, ensure_ascii=False)
        if key not in seen:
            seen.add(key)
            output.append(record)
    return output


def gate_for(item: dict[str, Any]) -> dict[str, Any]:
    check_id = str(item["id"])
    deadline = str(item.get("deadline", ""))
    if check_id.startswith("R4-"):
        if deadline in {"G2", "G3", "G4", "G5"}:
            return {
                "declared": deadline,
                "canonical": deadline,
                "basis": "CHECKS.json new_due.deadline is an explicit gate.",
                "traceability_status": "EXPLICIT",
            }
        return {
            "declared": deadline,
            "canonical": "TRACEABILITY_BLOCKED",
            "basis": "Unexpected new_due deadline; no gate inferred.",
            "traceability_status": "BLOCKED",
        }

    # Legacy deadline is a WP identifier.  Only the fixed GATES_AUTHORIZATION
    # statements for the early work packages provide a unique gate mapping.
    if deadline in {"WP10a", "WP01", "WP02"}:
        return {
            "declared": deadline,
            "canonical": "G1",
            "basis": "GATES_AUTHORIZATION.G1 explicitly names WP10a/WP01/WP02 due checks.",
            "traceability_status": "DERIVED_FROM_FIXED_GATE_TEXT",
        }
    if deadline in {"WP03", "WP09a"}:
        return {
            "declared": deadline,
            "canonical": "G2",
            "basis": "GATES_AUTHORIZATION.G2 explicitly names WP03/WP09a due checks.",
            "traceability_status": "DERIVED_FROM_FIXED_GATE_TEXT",
        }
    if deadline == "WP04":
        return {
            "declared": deadline,
            "canonical": "TRACEABILITY_BLOCKED",
            "possible": ["G3", "G4", "G5"],
            "basis": (
                "CHECKS.json legacy deadline is WP04, not a gate. The fixed plan "
                "has inherited WP04 checks at G3, all legacy checks at G4, and all "
                "59 checks at G5; no unique per-check gate is invented."
            ),
            "traceability_status": "BLOCKED_NON_UNIQUE_LEGACY_MAPPING",
        }
    return {
        "declared": deadline,
        "canonical": "TRACEABILITY_BLOCKED",
        "basis": "Legacy deadline is not a fixed gate identifier.",
        "traceability_status": "BLOCKED",
    }


def load_registry() -> tuple[dict[str, Any], bytes]:
    registry, raw = read_json(ROOT / REGISTRY_PATH)
    if not isinstance(registry, dict):
        raise RuntimeError("registry.json root must be an object")
    return registry, raw


def registry_commands(registry: dict[str, Any]) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for group in registry.get("groups", []):
        group_id = str(group.get("id", ""))
        for command in group.get("commands", []):
            record = dict(command)
            record["group_id"] = group_id
            records.append(record)
    command_ids = [str(record.get("id", "")) for record in records]
    duplicate_command_ids = sorted(
        {
            command_id
            for command_id in command_ids
            if command_id and command_ids.count(command_id) > 1
        }
    )
    if duplicate_command_ids:
        raise RuntimeError(
            "registry contains duplicate command IDs: "
            + ", ".join(duplicate_command_ids)
        )

    overlay = registry.get("command_metadata")
    metadata_entries: list[tuple[str, dict[str, Any]]] = []
    if overlay is None:
        overlay_info = {
            "present": False,
            "entry_count": 0,
            "command_count": len(records),
            "missing_ids": sorted(command_ids),
            "unknown_ids": [],
            "duplicate_ids": [],
            "conflicts": [],
            "complete": False,
        }
        return records, overlay_info
    if isinstance(overlay, dict):
        for command_id, metadata in overlay.items():
            if not isinstance(metadata, dict):
                raise RuntimeError(
                    f"command_metadata[{command_id!r}] must be an object"
                )
            metadata_entries.append((str(command_id), dict(metadata)))
    elif isinstance(overlay, list):
        seen_ids: set[str] = set()
        duplicate_ids: list[str] = []
        for index, metadata in enumerate(overlay):
            if not isinstance(metadata, dict) or not metadata.get("id"):
                raise RuntimeError(
                    f"command_metadata[{index}] must be an object with a non-empty id"
                )
            command_id = str(metadata["id"])
            if command_id in seen_ids:
                duplicate_ids.append(command_id)
            seen_ids.add(command_id)
            metadata_entries.append(
                (
                    command_id,
                    {key: value for key, value in metadata.items() if key != "id"},
                )
            )
        if duplicate_ids:
            raise RuntimeError(
                "command_metadata contains duplicate overlay IDs: "
                + ", ".join(sorted(set(duplicate_ids)))
            )
    else:
        raise RuntimeError("registry.command_metadata must be an object or array")

    overlay_ids = [command_id for command_id, _ in metadata_entries]
    duplicate_overlay_ids = sorted(
        {
            command_id
            for command_id in overlay_ids
            if overlay_ids.count(command_id) > 1
        }
    )
    if duplicate_overlay_ids:
        raise RuntimeError(
            "command_metadata contains duplicate overlay IDs: "
            + ", ".join(duplicate_overlay_ids)
        )
    command_id_set = set(command_ids)
    overlay_id_set = set(overlay_ids)
    unknown_ids = sorted(overlay_id_set - command_id_set)
    if unknown_ids:
        raise RuntimeError(
            "command_metadata contains unknown command IDs: " + ", ".join(unknown_ids)
        )
    missing_ids = sorted(command_id_set - overlay_id_set)
    metadata_by_id = dict(metadata_entries)
    conflicts: list[dict[str, Any]] = []
    merged_records: list[dict[str, Any]] = []
    for record in records:
        command_id = str(record.get("id", ""))
        metadata = metadata_by_id.get(command_id, {})
        for key, overlay_value in metadata.items():
            if key in record and record[key] != overlay_value:
                conflicts.append(
                    {
                        "command_id": command_id,
                        "field": key,
                        "inline": record[key],
                        "overlay": overlay_value,
                    }
                )
        merged = dict(record)
        merged.update(metadata)
        merged["_command_metadata_present"] = command_id in metadata_by_id
        merged["_command_metadata"] = metadata
        merged_records.append(merged)
    overlay_info = {
        "present": True,
        "entry_count": len(metadata_entries),
        "command_count": len(records),
        "missing_ids": missing_ids,
        "unknown_ids": unknown_ids,
        "duplicate_ids": duplicate_overlay_ids,
        "conflicts": conflicts,
        "id_complete": not missing_ids and not unknown_ids and not duplicate_overlay_ids,
        "complete": not missing_ids and not unknown_ids and not duplicate_overlay_ids and not conflicts,
    }
    return merged_records, overlay_info


def command_record(command: dict[str, Any], current_logs: dict[str, dict[str, Any]]) -> dict[str, Any]:
    target = path_text(str(command.get("target", "")))
    selector = path_text(str(command.get("selector", target)))
    target_path = ROOT / target
    target_exists = target_path.is_file() or target_path.is_dir()
    declared_readiness = None
    for name in ("readiness", "readiness_status", "status"):
        if name in command:
            declared_readiness = command[name]
            break
    observed_cases = command.get("observed_cases", {})
    return {
        "command_id": str(command.get("id", "")),
        "group": str(command.get("group_id", "")),
        "framework": command.get("framework"),
        "cwd": path_text(str(command.get("cwd", "."))),
        "exact_command": command_text(command.get("argv", [])),
        "argv": list(command.get("argv", [])),
        "selector": selector,
        "target": target,
        "target_exists": target_exists,
        "declared_readiness": declared_readiness,
        "readiness_reason": command.get("readiness_reason"),
        "declared_evidence_layer": command.get("evidence_layer"),
        "qualification_scope": command.get("qualification_scope"),
        "platform_requirement": command.get("platform_requirement"),
        "candidate_required": command.get("candidate_required"),
        "registry_anti_overclaim": command.get("anti_overclaim"),
        "command_metadata_present": command.get("_command_metadata_present", False),
        "command_metadata": command.get("_command_metadata", {}),
        "check_ids": list(command.get("check_ids", [])),
        "planned_test_tags": dict(command.get("planned_test_tags", {})),
        "observed_cases": observed_cases,
        "bind_paths": [path_text(str(p)) for p in command.get("bind_paths", [])],
        "target_sha256": sha256_file(target_path),
        "current_evidence": (
            [current_logs[str(command.get("id", ""))]]
            if current_logs.get(str(command.get("id", "")))
            else []
        ),
    }


def target_readiness(commands: list[dict[str, Any]]) -> dict[str, Any]:
    missing = [record["target"] for record in commands if not record["target_exists"]]
    present = [record["target"] for record in commands if record["target_exists"]]
    evidence = [
        evidence
        for record in commands
        for evidence in record.get("current_evidence", [])
    ]
    has_formal_evidence = any(
        evidence.get("evidence_kind") == "FORMAL_CANDIDATE_RUN"
        for evidence in evidence
    )
    if missing and not present:
        derived = "BLOCKED"
        reason = "All registry targets for this check are absent on current bytes."
    elif missing:
        derived = "PARTIAL_TARGETS_PRESENT"
        reason = "At least one target exists, but another mapped target is absent."
    elif evidence:
        derived = "EVIDENCE_PRESENT_NONQUALIFYING"
        reason = (
            "A formal candidate-run receipt exists, but its status/evidence layer "
            "cannot qualify the due check."
            if has_formal_evidence
            else "A local log exists, but its evidence layer cannot qualify the due check."
        )
    else:
        derived = "RUNNER_TARGET_PRESENT_NOT_RUN"
        reason = "Mapped target exists; no current due-check receipt was observed."
    return {
        "derived": derived,
        "reason": reason,
        "target_count": len(commands),
        "present_target_count": len(present),
        "missing_target_count": len(missing),
        "present_targets": unique(present),
        "missing_targets": unique(missing),
    }


def current_evidence_logs() -> dict[str, dict[str, Any]]:
    evidence_dir = ROOT / OUTPUT_RELATIVE
    logs: dict[str, dict[str, Any]] = {}
    baseline_direct = evidence_dir / "BASELINE_DIRECT.json"
    if baseline_direct.is_file():
        try:
            direct = json.loads(baseline_direct.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            direct = None
        if isinstance(direct, dict):
            for run in direct.get("runs", []):
                command = str(run.get("command", ""))
                if "sealing/sealing.test.mjs" in command:
                    command_id = "sealing-node-tap"
                elif "test_qualification.py" in command:
                    command_id = "qualification-unittest"
                else:
                    continue
                logs[command_id] = {
                    "path": path_text(str(evidence_dir.relative_to(ROOT) / "BASELINE_DIRECT.json")),
                    "log": run.get("log"),
                    "log_sha256": run.get("log_sha256"),
                    "evidence_kind": "BASELINE_DIRECT",
                    "exact_command": command,
                    "exit_code": run.get("exit"),
                    "positive_count": run.get("passed"),
                    "test_count": run.get("tests"),
                    "failed_count": run.get("failed"),
                    "skipped_count": run.get("skipped"),
                    "evidence_layer": run.get("evidence_layer", "UNKNOWN"),
                    "qualifies_actual_integrated": False,
                    "source_commit": run.get("source_commit", direct.get("source_commit")),
                    "frozen_parent": direct.get("frozen_parent"),
                    "test_blob": run.get("test_blob"),
                    "limits": [direct.get("limits", "")],
                    "receipt_source": "BASELINE_DIRECT.json",
                }
            if logs:
                return logs
    tap = parse_tap(evidence_dir / "baseline_sealing_tap.log")
    if tap:
        tap["evidence_kind"] = "BASELINE_LOG_ONLY"
        logs["sealing-node-tap"] = tap
    unittest = parse_unittest(evidence_dir / "baseline_harness_unittest.log")
    if unittest:
        unittest["evidence_kind"] = "BASELINE_LOG_ONLY"
        logs["qualification-unittest"] = unittest
    return logs


def verify_formal_aggregates(
    results: dict[str, Any],
    groups: list[dict[str, Any]],
    observed_check_ids: set[str],
    formal_command_count: int,
) -> None:
    command_rows = [command for group in groups for command in group.get("commands", [])]
    recomputed_group_statuses = Counter(str(group.get("status")) for group in groups)
    recomputed_command_statuses = Counter(str(command.get("status")) for command in command_rows)
    discovered = sum(int(command.get("discovered", 0) or 0) for command in command_rows)
    executed = sum(int(command.get("executed", 0) or 0) for command in command_rows)
    expected_aggregates = {
        "group_count": len(groups),
        "command_count": formal_command_count,
        "unique_check_ids_observed": len(observed_check_ids),
        "group_status_counts": dict(recomputed_group_statuses),
        "command_status_counts": dict(recomputed_command_statuses),
        "positive_tests_executed": executed,
        "positive_tests_passed": sum(int(command.get("passed", 0) or 0) for command in command_rows),
        "tests_failed": sum(int(command.get("failed", 0) or 0) for command in command_rows),
        "tests_skipped": sum(int(command.get("skipped", 0) or 0) for command in command_rows),
        "official_due_checks_accepted": 0,
    }
    for key, expected in expected_aggregates.items():
        if results.get(key) != expected:
            raise RuntimeError(
                f"RUN_RESULTS aggregate {key} does not match commands: "
                f"{results.get(key)!r} != {expected!r}"
            )
    if discovered < executed:
        raise RuntimeError("RUN_RESULTS discovered count is smaller than executed count")


def verify_formal_archive(
    results: dict[str, Any], path: Path, groups: list[dict[str, Any]]
) -> None:
    archive = results.get("archive")
    if not isinstance(archive, dict) or set(archive) != {"path", "bytes", "sha256"}:
        raise RuntimeError("RUN_RESULTS archive identity is malformed")
    if archive.get("path") != "FORMAL_REPORTS.zip":
        raise RuntimeError("RUN_RESULTS archive path is not the fixed package archive")
    archive_path = path.parent / str(archive["path"])
    if not archive_path.is_file():
        raise RuntimeError("RUN_RESULTS formal report archive is absent")
    archive_bytes = archive_path.read_bytes()
    if archive.get("bytes") != len(archive_bytes) or str(archive.get("sha256", "")).lower() != sha256_bytes(archive_bytes).lower():
        raise RuntimeError("RUN_RESULTS formal report archive identity does not match current bytes")
    declared_members: dict[str, tuple[int, str]] = {}
    for group in groups:
        raw_report = group.get("raw_report")
        if not isinstance(raw_report, dict) or set(raw_report) != {"archive_member", "bytes", "sha256"}:
            raise RuntimeError(f"RUN_RESULTS group {group.get('group')} raw report identity is malformed")
        member = str(raw_report["archive_member"])
        if member in declared_members:
            raise RuntimeError(f"RUN_RESULTS duplicate archive member: {member}")
        declared_members[member] = (int(raw_report["bytes"]), str(raw_report["sha256"]).lower())
    with zipfile.ZipFile(archive_path) as reports:
        members = reports.namelist()
        if len(members) != len(set(members)) or set(members) != set(declared_members):
            raise RuntimeError("FORMAL_REPORTS.zip members do not exactly match RUN_RESULTS")
        for member, (expected_bytes, expected_hash) in declared_members.items():
            data = reports.read(member)
            if len(data) != expected_bytes or sha256_bytes(data).lower() != expected_hash:
                raise RuntimeError(f"FORMAL_REPORTS.zip member identity mismatch: {member}")


def load_formal_run_results(
    path: Path,
    registry_commands_list: list[dict[str, Any]],
    expected_check_ids: list[str],
    current_head: str,
    registry_sha256: str,
) -> tuple[dict[str, Any], dict[str, dict[str, Any]]]:
    """Load and bind a formal candidate run without promoting any due check.

    The formal runner records one command entry for every registry command and
    one group entry for every registry group.  We validate those identities
    before exposing the records as current evidence, so a stale or partial
    RUN_RESULTS file cannot silently attach to the manifest.
    """
    if not path.is_file():
        return (
            {
                "present": False,
                "path": path_text(str(path.relative_to(ROOT))),
                "reason": "RUN_RESULTS.json is absent; no formal candidate evidence was loaded.",
            },
            {},
        )

    results, raw = read_json(path)
    if not isinstance(results, dict):
        raise RuntimeError("RUN_RESULTS.json root must be an object")
    groups = results.get("groups")
    if not isinstance(groups, list):
        raise RuntimeError("RUN_RESULTS.json groups must be an array")

    registry_by_id = {
        str(command.get("id", "")): command for command in registry_commands_list
    }
    registry_group_ids = {
        str(command.get("group_id", "")) for command in registry_commands_list
    }
    seen_groups: set[str] = set()
    seen_commands: set[str] = set()
    formal_by_command: dict[str, dict[str, Any]] = {}
    observed_check_ids: set[str] = set()
    group_summaries: list[dict[str, Any]] = []
    for group in groups:
        if not isinstance(group, dict) or not group.get("group"):
            raise RuntimeError("RUN_RESULTS.json contains a group without an id")
        group_id = str(group["group"])
        if group_id in seen_groups:
            raise RuntimeError(f"RUN_RESULTS.json contains duplicate group ID: {group_id}")
        seen_groups.add(group_id)
        if group_id not in registry_group_ids:
            raise RuntimeError(f"RUN_RESULTS.json contains unknown group ID: {group_id}")
        group_commands = group.get("commands")
        if not isinstance(group_commands, list):
            raise RuntimeError(f"RUN_RESULTS group {group_id} commands must be an array")
        raw_report = group.get("raw_report") or {}
        group_summaries.append(
            {
                "group": group_id,
                "status": group.get("status"),
                "execution_status": group.get("execution_status"),
                "invocation_exit": group.get("invocation_exit"),
                "recorded_utc": group.get("recorded_utc"),
                "command_count": len(group_commands),
                "command_ids": [str(command.get("id", "")) for command in group_commands],
                "raw_report": raw_report,
            }
        )
        group_candidate = group.get("candidate") or {}
        for command in group_commands:
            if not isinstance(command, dict) or not command.get("id"):
                raise RuntimeError(f"RUN_RESULTS group {group_id} contains command without an id")
            command_id = str(command["id"])
            if command_id in seen_commands:
                raise RuntimeError(
                    f"RUN_RESULTS.json contains duplicate command ID: {command_id}"
                )
            seen_commands.add(command_id)
            if command_id not in registry_by_id:
                raise RuntimeError(
                    f"RUN_RESULTS.json contains unknown command ID: {command_id}"
                )
            if str(command.get("group", group_id)) != group_id:
                raise RuntimeError(
                    f"RUN_RESULTS command {command_id} is bound to the wrong group"
                )
            registry_command = registry_by_id[command_id]
            observed_check_ids.update(registry_command.get("check_ids", []))
            formal_by_command[command_id] = {
                "source": "RUN_RESULTS.json",
                "evidence_kind": "FORMAL_CANDIDATE_RUN",
                "source_sha256": sha256_bytes(raw),
                "candidate_sha": results.get("candidate_sha"),
                "candidate": {
                    "branch": group_candidate.get("branch"),
                    "commit": group_candidate.get("commit"),
                    "dirty": group_candidate.get("dirty"),
                    "tree": group_candidate.get("tree"),
                },
                "group": group_id,
                "group_status": group.get("status"),
                "group_execution_status": group.get("execution_status"),
                "group_invocation_exit": group.get("invocation_exit"),
                "recorded_utc": group.get("recorded_utc"),
                "command_id": command_id,
                "formal_status": command.get("status"),
                "reason": command.get("reason"),
                "readiness": command.get("readiness"),
                "readiness_reason": command.get("readiness_reason"),
                "exact_command": command_text(
                    command.get("resolved_argv") or command.get("argv", [])
                ),
                "argv": command.get("argv", []),
                "resolved_argv": command.get("resolved_argv", []),
                "cwd": command.get("cwd"),
                "exit_code": command.get("exit_code"),
                "discovered": command.get("discovered"),
                "executed": command.get("executed"),
                "passed": command.get("passed"),
                "failed": command.get("failed"),
                "skipped": command.get("skipped"),
                "positive_count": command.get("passed"),
                "test_count": command.get("executed"),
                "failed_count": command.get("failed"),
                "skipped_count": command.get("skipped"),
                "stdout_sha256": command.get("stdout_sha256"),
                "stderr_sha256": command.get("stderr_sha256"),
                "result_sha256": command.get("result_sha256"),
                "evidence_layer": command.get("evidence_layer"),
                "qualification_scope": command.get("qualification_scope"),
                "platform_requirement": command.get("platform_requirement"),
                "target_candidate": command.get("target_candidate"),
                "raw_report": raw_report,
                "qualifies_actual_integrated": False,
                "qualifies_official_due": False,
                "limits": [
                    "Formal command status is current candidate evidence only; it does not change the fixed due status.",
                    "Python harness evidence is runner instrumentation evidence and cannot qualify product behavior or a gate.",
                ],
            }

    declared_group_count = results.get("group_count")
    declared_command_count = results.get("command_count")
    if declared_group_count != len(groups) or len(groups) != 20:
        raise RuntimeError(
            f"formal group coverage mismatch: declared={declared_group_count} observed={len(groups)} expected=20"
        )
    if declared_command_count != len(formal_by_command) or len(formal_by_command) != 23:
        raise RuntimeError(
            f"formal command coverage mismatch: declared={declared_command_count} observed={len(formal_by_command)} expected=23"
        )
    if seen_groups != registry_group_ids:
        raise RuntimeError(
            "formal group coverage does not equal registry groups: "
            f"missing={sorted(registry_group_ids - seen_groups)} "
            f"extra={sorted(seen_groups - registry_group_ids)}"
        )
    if seen_commands != set(registry_by_id):
        raise RuntimeError(
            "formal command coverage does not equal registry commands: "
            f"missing={sorted(set(registry_by_id) - seen_commands)} "
            f"extra={sorted(seen_commands - set(registry_by_id))}"
        )
    if set(observed_check_ids) != set(expected_check_ids):
        raise RuntimeError(
            "formal check coverage does not equal fixed PLAN: "
            f"missing={sorted(set(expected_check_ids) - observed_check_ids)} "
            f"extra={sorted(observed_check_ids - set(expected_check_ids))}"
        )
    verify_formal_aggregates(results, groups, observed_check_ids, len(formal_by_command))
    verify_formal_archive(results, path, groups)
    if str(results.get("candidate_sha", "")).lower() != current_head.lower():
        raise RuntimeError(
            "RUN_RESULTS candidate does not match current package HEAD: "
            f"{results.get('candidate_sha')} != {current_head}"
        )
    if str(results.get("frozen_source_parent", "")).lower() != FROZEN_SOURCE_SHA.lower():
        raise RuntimeError("RUN_RESULTS frozen source parent does not match the fixed source SHA")
    if str(results.get("plan_sha", "")).lower() != PLAN_SHA.lower():
        raise RuntimeError("RUN_RESULTS plan SHA does not match the fixed PLAN SHA")
    if str(results.get("registry_sha256", "")).lower() != registry_sha256.lower():
        raise RuntimeError("RUN_RESULTS registry SHA does not match current registry bytes")

    formal_summary = {
        "present": True,
        "path": path_text(str(path.relative_to(ROOT))),
        "sha256": sha256_bytes(raw),
        "schema": results.get("schema"),
        "candidate_sha": results.get("candidate_sha"),
        "frozen_source_parent": results.get("frozen_source_parent"),
        "plan_sha": results.get("plan_sha"),
        "registry_sha256": results.get("registry_sha256"),
        "declared_group_count": declared_group_count,
        "observed_group_count": len(groups),
        "declared_command_count": declared_command_count,
        "observed_command_count": len(formal_by_command),
        "command_ids": sorted(formal_by_command),
        "unique_check_ids_observed": results.get("unique_check_ids_observed"),
        "group_status_counts": results.get("group_status_counts"),
        "command_status_counts": results.get("command_status_counts"),
        "positive_tests_executed": results.get("positive_tests_executed"),
        "positive_tests_passed": results.get("positive_tests_passed"),
        "tests_failed": results.get("tests_failed"),
        "tests_skipped": results.get("tests_skipped"),
        "official_due_checks_accepted": results.get("official_due_checks_accepted"),
        "groups": group_summaries,
        "coverage_status": "20_GROUPS_23_COMMANDS_59_CHECK_IDS_BOUND",
        "limits": results.get("limits"),
    }
    return formal_summary, formal_by_command


def source_finding(commands: list[dict[str, Any]]) -> dict[str, Any]:
    paths: list[str] = []
    symbols: list[dict[str, Any]] = []
    evidence: list[dict[str, Any]] = []
    for command in commands:
        target = command["target"]
        paths.append(target)
        symbols.append(
            {
                "command_id": command["command_id"],
                "selector": command["selector"],
                "observed_cases": command["observed_cases"],
            }
        )
        evidence.append(
            {
                "path": target,
                "state": "PRESENT" if command["target_exists"] else "ABSENT",
                "sha256": command["target_sha256"],
                "command_id": command["command_id"],
            }
        )
    return {"paths": unique(paths), "symbols": symbols, "evidence": evidence}


def build_manifest(
    output_dir: Path, *, include_formal_results: bool = True
) -> tuple[dict[str, Any], dict[str, Any]]:
    fixed_blobs = {name: fixed_plan_blob(name) for name in PLAN_FILES}
    fixed = {
        name: json_bytes(fixed_blobs[name], f"{PLAN_SHA}:{name}")
        for name in PLAN_FILES
    }
    checks = fixed["CHECKS.json"]
    execution = fixed["EXECUTION_PLAN.json"]
    capability_map = fixed["CAPABILITY_TASK_MAP.json"]
    registry, registry_raw = load_registry()
    commands, registry_metadata = registry_commands(registry)
    logs = current_evidence_logs()
    current_head = run_git("rev-parse", "HEAD").decode().strip()
    branch = run_git("branch", "--show-current").decode().strip()
    status = run_git("status", "--porcelain=v1", "--untracked-files=all").decode("utf-8", errors="replace")

    due_items = list(checks.get("legacy_due", [])) + list(checks.get("new_due", []))
    expected_legacy = int(checks.get("original_due_count", 0))
    expected_new = len(checks.get("new_due", []))
    ids = [str(item["id"]) for item in due_items]
    if len(due_items) != 59 or expected_legacy != 33 or expected_new != 26:
        raise RuntimeError(
            f"fixed PLAN due counts unexpected: total={len(due_items)} "
            f"legacy={expected_legacy} new={expected_new}"
        )
    if len(set(ids)) != 59:
        raise RuntimeError("fixed PLAN does not contain exactly 59 unique due check IDs")

    if include_formal_results:
        formal_summary, formal_by_command = load_formal_run_results(
            ROOT / OUTPUT_RELATIVE / "RUN_RESULTS.json",
            commands,
            ids,
            current_head,
            sha256_bytes(registry_raw),
        )
    else:
        formal_summary = {
            "present": False,
            "path": path_text(str((ROOT / OUTPUT_RELATIVE / "RUN_RESULTS.json").relative_to(ROOT))),
            "reason": "self-check validates PLAN and registry structure without rebinding frozen run evidence",
        }
        formal_by_command = {}

    tasks = execution.get("tasks", [])
    tasks_by_check: dict[str, list[dict[str, Any]]] = {check_id: [] for check_id in ids}
    for task in tasks:
        for check_id in task.get("checks", []):
            if check_id in tasks_by_check:
                tasks_by_check[check_id].append(task)

    capabilities_by_check: dict[str, list[dict[str, Any]]] = {check_id: [] for check_id in ids}
    for row in capability_map.get("rows", []):
        for check_id in row.get("first_checks", []):
            if check_id in capabilities_by_check:
                capabilities_by_check[check_id].append(row)

    commands_by_check: dict[str, list[dict[str, Any]]] = {check_id: [] for check_id in ids}
    for command in commands:
        record = command_record(command, logs)
        if formal_by_command.get(record["command_id"]):
            record["current_evidence"].append(formal_by_command[record["command_id"]])
        for check_id in command.get("check_ids", []):
            if check_id in commands_by_check:
                commands_by_check[check_id].append(record)

    registry_ids = sorted({check_id for command in commands for check_id in command.get("check_ids", [])})
    if set(registry_ids) != set(ids):
        missing = sorted(set(ids) - set(registry_ids))
        extra = sorted(set(registry_ids) - set(ids))
        raise RuntimeError(f"registry due mapping mismatch: missing={missing} extra={extra}")

    records: list[dict[str, Any]] = []
    for item in due_items:
        check_id = str(item["id"])
        check_commands = commands_by_check[check_id]
        matched_tasks = tasks_by_check[check_id]
        matched_capabilities = capabilities_by_check[check_id]
        readiness = target_readiness(check_commands)
        gate = gate_for(item)
        task_ids = unique(task.get("id", "") for task in matched_tasks)
        capability_ids = unique(row.get("id", "") for row in matched_capabilities)
        # New R4 checks have no legacy capability rows; retain their fixed
        # capability group rather than pretending a Cxx capability exists.
        capability_groups = unique(item.get("groups", [item.get("group", "")]))
        dependencies = unique(
            dependency
            for task in matched_tasks
            for dependency in task.get("depends_on", [])
        )
        bind_paths = unique(
            path
            for command in check_commands
            for path in command.get("bind_paths", [])
        )
        task_scopes = unique(
            scope
            for task in matched_tasks
            for scope in task.get("write_scopes", [])
        )
        implementation_surface = unique(
            [path for path in bind_paths if not is_test_surface(path)] + task_scopes
        )
        test_surface = unique(
            [command["target"] for command in check_commands]
            + [path for path in bind_paths if is_test_surface(path)]
        )
        current_evidence = [
            evidence
            if evidence.get("command_id")
            else {"command_id": command["command_id"], **evidence}
            for command in check_commands
            for evidence in command.get("current_evidence", [])
        ]
        blocked: list[str] = ["OFFICIAL_DUE_STATUS_NOT_RUN"]
        if readiness["missing_targets"]:
            blocked.append("RUNNER_TARGET_MISSING")
        if not current_evidence:
            blocked.append("NO_CURRENT_DUE_CHECK_RECEIPT")
        if current_evidence:
            blocked.append("CURRENT_EVIDENCE_LAYER_NOT_ACTUAL_INTEGRATED")
        if gate["canonical"] == "TRACEABILITY_BLOCKED":
            blocked.append("GATE_MAPPING_NOT_UNIQUE_FROM_FIXED_PLAN")
        if readiness["missing_targets"]:
            integration = {
                "status": "INTEGRATION_REQUIRED",
                "package_adoption_review": "CONTROLLER_REVIEW_REQUIRED",
                "missing_target_paths": readiness["missing_targets"],
                "reason": (
                    "The current MC-125 bytes do not contain every registry target "
                    "needed by this check; the missing qualification surface cannot "
                    "be represented as a PASS."
                ),
                "proposed_adoption": {
                    "owner": "main construction Controller",
                    "action": (
                        "Adopt a real current-candidate test surface at each exact "
                        "missing path, or update the registry to an exact existing "
                        "surface after review; do not synthesize a placeholder."
                    ),
                    "exact_paths": readiness["missing_targets"],
                    "after_adoption": (
                        "Execute the exact mapped command and capture a positive-count, "
                        "candidate-bound receipt before any due-status or gate decision."
                    ),
                },
                "preserve_official_status": "NOT_RUN",
                "production_semantics_changed": False,
                "self_integration": False,
            }
        else:
            integration = {
                "status": "CONTROLLER_REVIEW_REQUIRED",
                "package_adoption_review": "CONTROLLER_REVIEW_REQUIRED",
                "missing_target_paths": [],
                "reason": (
                    "All mapped target paths exist, but this package has no current "
                    "official due-check receipt or gate acceptance."
                ),
                "proposed_adoption": {
                    "owner": "main construction Controller",
                    "action": "Review and adopt this traceability/readiness evidence without promoting due status.",
                    "exact_paths": readiness["present_targets"],
                    "after_adoption": "Run the exact command and bind a candidate/source/config receipt.",
                },
                "preserve_official_status": "NOT_RUN",
                "production_semantics_changed": False,
                "self_integration": False,
            }
        anti_overclaim = [
            "The official fixed PLAN status remains NOT_RUN; target existence is not a due-check PASS.",
            "A harness self-test or source assertion cannot qualify an actual integrated implementation.",
            "Current fixture logs do not prove Windows native, Owner-machine, provider, credentials, or live egress predicates.",
            "No gate acceptance, production integration, or architecture conclusion is made by this manifest.",
        ]
        if check_id in {"T56.L", "T57.L"}:
            anti_overclaim.append(
                "sealing-node-tap is a synthetic fixture with fake dependency recording; it cannot qualify real Windows or Owner predicates."
            )
        record = {
            "check_id": check_id,
            "requirement": item.get("requirement", ""),
            "assertions": item.get("assertions"),
            "status": item.get("status", "NOT_RUN"),
            "official_due_status": item.get("status", "NOT_RUN"),
            "gate": gate,
            "capability": {
                "ids": capability_ids,
                "groups": capability_groups,
                "source": f"{PLAN_SHA}:{PLAN_ROOT}/CAPABILITY_TASK_MAP.json",
            },
            "task": {
                "ids": task_ids,
                "dependencies": dependencies,
                "source": f"{PLAN_SHA}:{PLAN_ROOT}/EXECUTION_PLAN.json",
            },
            "implementation_surface": implementation_surface,
            "test_surface": test_surface,
            "runner": check_commands,
            "exact_command": unique(command["exact_command"] for command in check_commands),
            "candidate_dependency": {
                "frozen_source_sha": FROZEN_SOURCE_SHA,
                "fixed_plan_sha": PLAN_SHA,
                "package_start_sha": PACKAGE_START_SHA,
                "task_dependencies": dependencies,
                "registry_path": REGISTRY_PATH,
                "registry_sha256": sha256_bytes(registry_raw),
            },
            "platform": {
                "required_layer": item.get("evidence", ""),
                "observed_host": {
                    "system": platform.system(),
                    "release": platform.release(),
                    "machine": platform.machine(),
                },
                "owner_machine": "NOT_RUN",
                "native_or_integrated_product": "NOT_PROVEN",
            },
            "auth": {
                "provider_or_account_auth": "NOT_USED",
                "credentials": "NOT_USED",
                "live_egress": "NOT_AUTHORIZED_AND_NOT_USED",
                "owner_authorization": "NOT_RUN",
            },
            "evidence_layer": {
                "required": item.get("evidence", ""),
                "current": unique(
                    evidence.get("evidence_layer", "UNKNOWN")
                    for evidence in current_evidence
                )
                or ["NO_CURRENT_EVIDENCE"],
                "current_qualifies_due": False,
            },
            "readiness": {
                **readiness,
                "official_due_status": item.get("status", "NOT_RUN"),
                "registry_declared": [
                    {
                        "command_id": command["command_id"],
                        "readiness": command.get("declared_readiness"),
                        "reason": command.get("readiness_reason"),
                        "evidence_layer": command.get("declared_evidence_layer"),
                        "qualification_scope": command.get("qualification_scope"),
                        "platform_requirement": command.get("platform_requirement"),
                        "candidate_required": command.get("candidate_required"),
                        "anti_overclaim": command.get("registry_anti_overclaim"),
                    }
                    for command in check_commands
                ],
            },
            "positive_count": {
                "official_due": {
                    "value": None,
                    "verified": False,
                    "status": item.get("status", "NOT_RUN"),
                },
                "current_observations": [
                    {
                        "command_id": evidence["command_id"],
                        "evidence_kind": evidence.get("evidence_kind"),
                        "formal_status": evidence.get("formal_status"),
                        "value": evidence.get("positive_count"),
                        "test_count": evidence.get("test_count"),
                        "exit_code_inferred": evidence.get("exit_code_inferred"),
                        "exit_code": evidence.get("exit_code", evidence.get("exit_code_inferred")),
                        "evidence_layer": evidence.get("evidence_layer"),
                        "candidate_sha": evidence.get("candidate_sha"),
                        "raw_report_sha": (evidence.get("raw_report") or {}).get("sha256"),
                    }
                    for evidence in current_evidence
                ],
                "zero_is_nonaccepting": True,
            },
            "current_evidence": current_evidence,
            "current_byte_finding": source_finding(check_commands),
            "blocked": unique(blocked),
            "integration": integration,
            "anti_overclaim": anti_overclaim,
            "architecture_escalation_candidate": False,
        }
        records.append(record)

    source_identity = {
        "frozen_source_sha": FROZEN_SOURCE_SHA,
        "fixed_plan_sha": PLAN_SHA,
        "package_start_sha": PACKAGE_START_SHA,
        "package_head_at_generation": current_head,
        "branch_at_generation": branch,
        "plan_file_sha256": {
            name: sha256_bytes(fixed_blobs[name]) for name in PLAN_FILES
        },
        "registry_path": REGISTRY_PATH,
        "registry_sha256": sha256_bytes(registry_raw),
        "runner_path": RUNNER_PATH,
        "runner_sha256": sha256_file(ROOT / RUNNER_PATH),
        "manifest_builder_path": path_text(str(Path(__file__).resolve().relative_to(ROOT))),
        "manifest_builder_sha256": sha256_file(Path(__file__).resolve()),
        "formal_run_results_path": formal_summary.get("path"),
        "formal_run_results_sha256": formal_summary.get("sha256"),
    }
    evidence_file_hashes = {}
    for filename in (
        "BASELINE_DIRECT.json",
        "baseline_harness_unittest.log",
        "baseline_sealing_tap.log",
        "RUN_RESULTS.json",
    ):
        evidence_file_hashes[filename] = sha256_file(ROOT / OUTPUT_RELATIVE / filename)
    summary = {
        "check_count": len(records),
        "unique_check_count": len({record["check_id"] for record in records}),
        "legacy_count": sum(1 for item in due_items if not str(item["id"]).startswith("R4-")),
        "new_r4_count": sum(1 for item in due_items if str(item["id"]).startswith("R4-")),
        "official_due_status_counts": {"NOT_RUN": len(records)},
        "readiness_counts": {
            readiness: sum(1 for record in records if record["readiness"]["derived"] == readiness)
            for readiness in sorted({record["readiness"]["derived"] for record in records})
        },
        "registry_command_count": len(commands),
        "registry_unique_due_check_count": len(registry_ids),
        "registry_target_present_command_count": sum(
            1 for command in commands if (ROOT / str(command.get("target", ""))).exists()
        ),
        "registry_target_missing_command_count": sum(
            1 for command in commands if not (ROOT / str(command.get("target", ""))).exists()
        ),
        "integration_required_check_count": sum(
            1 for record in records if record["integration"]["status"] == "INTEGRATION_REQUIRED"
        ),
        "controller_review_check_count": sum(
            1 for record in records if record["integration"]["status"] == "CONTROLLER_REVIEW_REQUIRED"
        ),
        "registry_declared_readiness_counts": {
            readiness: sum(
                1
                for command in commands
                if (command.get("readiness", command.get("readiness_status")) or "UNDECLARED")
                == readiness
            )
            for readiness in sorted(
                {
                    command.get("readiness", command.get("readiness_status")) or "UNDECLARED"
                    for command in commands
                }
            )
        },
        "formal_group_count": formal_summary.get("observed_group_count", 0),
        "formal_command_count": formal_summary.get("observed_command_count", 0),
        "formal_unique_check_ids_observed": formal_summary.get("unique_check_ids_observed"),
        "formal_command_status_counts": formal_summary.get("command_status_counts"),
        "formal_group_status_counts": formal_summary.get("group_status_counts"),
        "formal_positive_tests_executed": formal_summary.get("positive_tests_executed"),
        "formal_positive_tests_passed": formal_summary.get("positive_tests_passed"),
        "formal_official_due_checks_accepted": formal_summary.get("official_due_checks_accepted"),
        "current_evidence_files": evidence_file_hashes,
        "worktree_dirty_at_generation": bool(status.strip()),
    }
    manifest = {
        "schema": "gogoke.s1-r4.qualification-manifest.v1",
        "package": {
            "id": "qualification-infra-mc125",
            "role": "QUALIFICATION_INFRA_CONSTRUCTION",
            "branch": branch,
            "output_directory": OUTPUT_RELATIVE,
        },
        "source_identity": source_identity,
        "registry_metadata_overlay": registry_metadata,
        "toolchain_platform_identity": {
            "platform": {
                "system": platform.system(),
                "release": platform.release(),
                "version": platform.version(),
                "machine": platform.machine(),
                "processor": platform.processor(),
            },
            "toolchain": toolchain_identity(),
        },
        "summary": summary,
        "checks": records,
        "current_evidence": {
            "logs": logs,
            "formal_run_results": formal_summary,
            "evidence_layer_boundary": [
                "Harness self-tests validate instrumentation only.",
                "sealing-node-tap validates synthetic source assertions with fake dependencies only.",
                "Formal RUN_RESULTS command statuses are current candidate evidence; they do not promote an official due status.",
                "No local observation changes an official due status from NOT_RUN.",
            ],
        },
        "not_proven": [
            "No G1-G5 gate is accepted by this manifest.",
            "No product integration or execution-branch integration is performed.",
            "No Owner-machine predicate, real Windows native predicate, provider/account readiness, credential, live model, or egress behavior is proven.",
            "No architecture escalation candidate is established by missing targets, ordinary harness gaps, or non-executed checks.",
        ],
        "integration": {
            "status": "CONTROLLER_REVIEW_REQUIRED",
            "recommended_action": "Review registry readiness additions and adopt this manifest/traceability as package evidence without promoting due statuses.",
            "preserve_official_due_statuses": True,
            "execution_branch_touched": False,
            "parallel_inbox_written": False,
        },
        "architecture_escalation_candidate": False,
        "anti_overclaim": [
            "Exactly 59 PLAN checks are mapped; mapping completeness is not qualification success.",
            "Present target paths are source existence only; absent targets are readiness blockers, not fabricated passes.",
            "A positive count is meaningful only within its declared evidence layer and cannot be borrowed for an official due check.",
        ],
    }

    capability_to_checks: dict[str, list[str]] = {}
    task_to_checks: dict[str, list[str]] = {}
    trace_checks: list[dict[str, Any]] = []
    for record in records:
        for capability_id in record["capability"]["ids"]:
            capability_to_checks.setdefault(capability_id, []).append(record["check_id"])
        for task_id in record["task"]["ids"]:
            task_to_checks.setdefault(task_id, []).append(record["check_id"])
        trace_checks.append(
            {
                "check_id": record["check_id"],
                "gate": record["gate"],
                "capability": record["capability"],
                "task": record["task"],
                "runner_ids": [command["command_id"] for command in record["runner"]],
                "exact_command": record["exact_command"],
                "candidate_dependency": record["candidate_dependency"],
                "platform": record["platform"],
                "auth": record["auth"],
                "evidence_layer": record["evidence_layer"],
                "implementation_surface": record["implementation_surface"],
                "test_surface": record["test_surface"],
                "official_due_status": record["readiness"]["official_due_status"],
                "status": record["status"],
                "readiness": record["readiness"],
                "positive_count": record["positive_count"],
                "current_evidence": record["current_evidence"],
                "current_byte_finding": record["current_byte_finding"],
                "blocked": record["blocked"],
                "integration": record["integration"],
                "anti_overclaim": record["anti_overclaim"],
                "architecture_escalation_candidate": record["architecture_escalation_candidate"],
            }
        )
    traceability = {
        "schema": "gogoke.s1-r4.qualification-traceability.v1",
        "source_identity": source_identity,
        "registry_metadata_overlay": registry_metadata,
        "summary": summary,
        "formal_run_results": formal_summary,
        "check_ids": [record["check_id"] for record in records],
        "capability_to_checks": capability_to_checks,
        "task_to_checks": task_to_checks,
        "checks": trace_checks,
        "missing_runner_targets": sorted(
            {
                target
                for record in records
                for target in record["readiness"]["missing_targets"]
            }
        ),
        "not_proven": manifest["not_proven"],
        "architecture_escalation_candidate": False,
    }
    return manifest, traceability


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=ROOT / OUTPUT_RELATIVE,
        help="package-local output directory (default: %(default)s)",
    )
    parser.add_argument(
        "--self-check",
        action="store_true",
        help="validate fixed PLAN, registry command IDs, and command_metadata overlay without writing artifacts",
    )
    args = parser.parse_args()
    manifest, traceability = build_manifest(
        args.output_dir, include_formal_results=not args.self_check
    )
    if args.self_check:
        print(
            json.dumps(
                {
                    "self_check": (
                        "PASS"
                        if not manifest["registry_metadata_overlay"]["conflicts"]
                        else "PASS_WITH_RECORDED_CONFLICTS"
                    ),
                    "check_count": manifest["summary"]["check_count"],
                    "unique_check_count": manifest["summary"]["unique_check_count"],
                    "registry_command_count": manifest["summary"]["registry_command_count"],
                    "registry_metadata_overlay": manifest["registry_metadata_overlay"],
                    "official_due_status_counts": manifest["summary"]["official_due_status_counts"],
                },
                ensure_ascii=False,
            )
        )
        return 0
    args.output_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = args.output_dir / "QUALIFICATION_MANIFEST.json"
    trace_path = args.output_dir / "TRACEABILITY.json"
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    trace_path.write_text(
        json.dumps(traceability, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(
        json.dumps(
            {
                "manifest": str(manifest_path),
                "traceability": str(trace_path),
                "check_count": manifest["summary"]["check_count"],
                "unique_check_count": manifest["summary"]["unique_check_count"],
                "official_due_status_counts": manifest["summary"]["official_due_status_counts"],
                "registry_command_count": manifest["summary"]["registry_command_count"],
                "registry_target_missing_command_count": manifest["summary"]["registry_target_missing_command_count"],
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
