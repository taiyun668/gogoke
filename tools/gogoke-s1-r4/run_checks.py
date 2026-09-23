#!/usr/bin/env python3
"""Fail-closed S1-R4 execution evidence runner.

The registry is deliberately explicit.  A group may contain several exact
commands (for example the R4 codec has both server and desktop evidence), but
no command may be invented at runtime.  A command can only become evidence
when its selector and target are committed in the candidate, its framework
machine output is present and valid, and every check binding carries the exact
planned tag from the fixed R4 plan.

This runner does not accept a worktree plan as authority. Public plan bytes are
read from the candidate Git object and locked by the Owner-merged authorization
MANIFEST blob; PLAN_COMMIT remains the private provenance identity.
Successful test execution is evidence for independent review, never product
or gate acceptance.  Registry readiness and evidence-layer metadata are
mechanically bound to each command; a missing target must carry an explicit
non-ready state and a source-diagnostic fake can never be promoted to native
or Owner-machine evidence.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

sys.dont_write_bytecode = True


ROOT = Path(__file__).resolve().parents[2]
REGISTRY_REL = "tools/gogoke-s1-r4/registry.json"
REGISTRY_PATH = ROOT / REGISTRY_REL
PLAN_REL = "docs/design/gogoke-s1-r4-plan-v1"
PLAN_COMMIT = "cbdc6ad592947370941024a87dbb9168a5b59055"
SOURCE_HEAD = "88ef8e7dfbf5ba5aef58743dc45fa660f946276e"
PUBLIC_BINDING_REL = f"{PLAN_REL}/PUBLIC_EXECUTION_BINDING.json"
PUBLIC_MANIFEST_REL = f"{PLAN_REL}/MANIFEST.json"
AUTH_REL = "artifacts/s1-r4/intake/PUBLIC_AUTHORIZATION_RECEIPT.json"
AUTH_OWNER_INSTRUCTION = "OWNER_MERGED_PUBLIC_S1_R4_AUTHORIZATION"
PUBLIC_REPOSITORY = "taiyun668/gogoke"
TOOLCHAIN_RECEIPT_REL = "artifacts/s1-r4/intake/CONTROLLER_TOOLCHAIN_RECEIPT.json"
TOOLCHAIN_RECEIPT_SCHEMA = "gogoke.s1-r4.controller-toolchain-receipt.v1"
BLOCKED_UNQUALIFIED_REASONS = {"RUST_CLOSURE_NOT_QUALIFIED", "TOOLCHAIN_NOT_QUALIFIED", "DEPENDENCY_CLOSURE_NOT_QUALIFIED"}

GROUPS = (
    "qualification", "sealing", "codec", "boundary", "store", "root", "host",
    "process", "adapters", "capabilities", "delivery", "continuation", "events",
    "policy", "context", "decision", "evaluation", "dream", "vertical", "upgrade",
)
FRAMEWORKS = {"vitest_json", "node_tap", "python_unittest", "rust_libtest"}
READINESS_READY = "READY"
READINESS_STATES = {
    READINESS_READY,
    "NOT_READY",
    "NEEDS_CURRENT_CANDIDATE",
    "NEEDS_CONTROLLED_PLATFORM",
    "NEEDS_OWNER_MACHINE",
    "NEEDS_FUTURE_AUTHORIZATION",
    "TRACEABILITY_BLOCKED",
}
EVIDENCE_LAYERS = {
    "PYTHON_HARNESS",
    "SOURCE_DIAGNOSTIC_FAKE",
    "NATIVE_UNIT_TEST",
    "QUALIFICATION_FIXTURE",
    "CONTROLLED_PLATFORM",
    "OWNER_MACHINE",
}
EVIDENCE_SCOPE_BY_LAYER = {
    "PYTHON_HARNESS": "runner_harness_only",
    "SOURCE_DIAGNOSTIC_FAKE": "source_diagnostic_only",
    "NATIVE_UNIT_TEST": "native_unit_only",
    "QUALIFICATION_FIXTURE": "qualification_fixture_only",
    "CONTROLLED_PLATFORM": "controlled_platform_only",
    "OWNER_MACHINE": "owner_machine_only",
}
PLATFORM_REQUIREMENTS = {"host", "controlled_platform", "owner_machine"}
EXPECTED_OBLIGATIONS = {
    "legacy_tasks": 19,
    "legacy_master_tests": 68,
    "legacy_deadlines": 157,
    "legacy_due": 33,
    "new_due": 26,
    "all_due": 59,
    "capability_behaviors": 84,
    "tasks": 32,
}
STATUS_PASS = "PASS"
STATUS_FAIL = "FAIL_EXECUTION"
STATUS_INSTRUMENT = "FAIL_INSTRUMENT"
STATUS_BLOCKED = "BLOCKED"
OUTPUT_MARKER = "{{OUTPUT}}"


class RunnerError(RuntimeError):
    pass


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest().upper()


def stable(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def digest_json(value: Any) -> str:
    return sha256_bytes(stable(value).encode("utf-8"))


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")


def repo_rel(path: Path) -> str:
    try:
        return path.resolve().relative_to(ROOT.resolve()).as_posix()
    except ValueError as exc:
        raise RunnerError(f"path escapes repository: {path}") from exc


def resolve_repo(value: str | Path, base: Path = ROOT) -> Path:
    path = Path(value)
    if not path.is_absolute():
        path = base / path
    resolved = path.resolve()
    repo_rel(resolved)
    return resolved


def isolated_git_environment(git_path: str) -> dict[str, str]:
    env, _ = sanitized_environment(git_path)
    env.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_SYSTEM": os.devnull, "GIT_TERMINAL_PROMPT": "0", "GIT_OPTIONAL_LOCKS": "0", "GIT_CONFIG_COUNT": "0"})
    return env


def git_run(args: list[str]) -> subprocess.CompletedProcess[bytes]:
    git_path = resolve_tool("git")
    if not git_path:
        raise RunnerError("git executable is unavailable")
    try:
        return subprocess.run([git_path, *args], cwd=ROOT, env=isolated_git_environment(git_path), capture_output=True, check=False)
    except OSError as exc:
        raise RunnerError(f"git unavailable: {exc}") from exc


def git_bytes(ref: str, relative: str) -> bytes:
    process = git_run(["show", f"{ref}:{relative}"])
    if process.returncode != 0:
        raise RunnerError(f"Git object missing: {ref}:{relative}: {process.stderr.decode('utf-8', 'replace').strip()}")
    return process.stdout


def git_text(ref: str, relative: str) -> str:
    return git_bytes(ref, relative).decode("utf-8")


def git_oid(ref: str, relative: str) -> tuple[str | None, str | None]:
    try:
        process = git_run(["ls-tree", ref, "--", relative])
    except RunnerError as exc:
        return None, str(exc)
    if process.returncode != 0:
        return None, process.stderr.decode("utf-8", "replace")
    line = process.stdout.decode("utf-8", "replace").strip()
    if not line:
        return None, None
    fields = line.split(None, 3)
    if len(fields) < 3:
        return None, f"malformed ls-tree row for {relative}"
    mode, kind, oid = fields[:3]
    if kind != "blob":
        return None, f"candidate target is not a blob: mode={mode} kind={kind}"
    return oid, None


def git_identity() -> dict[str, Any]:
    git_path = resolve_tool("git")
    if not git_path:
        return {"commit": None, "tree": None, "branch": None, "dirty": False, "dirty_paths": [], "status_exit": 127, "errors": ["git executable is unavailable"], "git_tool": None}

    def one(*args: str) -> tuple[int, str, str]:
        process = git_run(list(args))
        return process.returncode, process.stdout.decode("utf-8", "replace").strip(), process.stderr.decode("utf-8", "replace").strip()

    code, commit, error = one("rev-parse", "HEAD")
    tcode, tree, terr = one("rev-parse", "HEAD^{tree}")
    status_process = git_run(["status", "--porcelain=v1", "--untracked-files=all"])
    scode, status, serr = status_process.returncode, status_process.stdout.decode("utf-8", "replace").rstrip("\r\n"), status_process.stderr.decode("utf-8", "replace").strip()
    bcode, branch, berr = one("branch", "--show-current")
    dirty_paths = [line[3:] for line in status.splitlines() if len(line) >= 4]
    return {
        "commit": commit if code == 0 else None,
        "tree": tree if tcode == 0 else None,
        "branch": branch if bcode == 0 else None,
        "dirty": bool(dirty_paths),
        "dirty_paths": dirty_paths,
        "status_exit": scode,
        "errors": [item for item in (error, terr, serr, berr) if item],
        "git_tool": tool_identity(git_path),
    }


def load_json_bytes(value: bytes, label: str) -> dict[str, Any]:
    try:
        parsed = json.loads(value.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise RunnerError(f"invalid fixed JSON {label}: {exc}") from exc
    if not isinstance(parsed, dict):
        raise RunnerError(f"fixed JSON root is not an object: {label}")
    return parsed


def fixed_plan_root(value: str) -> Path:
    candidate = Path(value)
    choices = [candidate] if candidate.is_absolute() else [Path.cwd() / candidate, ROOT / candidate]
    for choice in choices:
        resolved = choice.resolve()
        try:
            if repo_rel(resolved) == PLAN_REL and resolved.is_dir():
                return resolved
        except RunnerError:
            continue
    raise RunnerError(f"--plan-root must resolve to the fixed {PLAN_REL} directory")


def load_fixed_plan(plan_root: Path) -> dict[str, Any]:
    public_commit = git_identity().get("commit")
    if not public_commit:
        raise RunnerError("public candidate HEAD is unavailable")
    manifest = load_json_bytes(git_bytes(public_commit, PUBLIC_MANIFEST_REL), "MANIFEST.json")
    expected = manifest.get("sha256")
    if not isinstance(expected, dict) or not expected:
        raise RunnerError("public candidate plan manifest has no sha256 map")
    files: dict[str, dict[str, Any]] = {}
    errors: list[str] = []
    for relative, expected_sha in expected.items():
        if not isinstance(relative, str) or not isinstance(expected_sha, str):
            errors.append("manifest contains non-string path/hash")
            continue
        object_path = f"{PLAN_REL}/{relative}"
        try:
            data = git_bytes(public_commit, object_path)
            oid, oid_error = git_oid(public_commit, object_path)
            observed = sha256_bytes(data)
            files[relative] = {"sha256": observed, "bytes": len(data), "git_blob": oid}
            if observed != expected_sha.upper():
                errors.append(f"public candidate plan hash mismatch: {relative}")
            if oid_error:
                errors.append(f"public candidate plan blob error: {relative}: {oid_error}")
        except RunnerError as exc:
            errors.append(str(exc))
    required = {
        "CHECKS.json", "EXECUTION_PLAN.json", "CAPABILITY_TASK_MAP.json",
        "GATES_AUTHORIZATION.json", "RUNBOOK.md", "INPUTS.json",
        "inputs/LEGACY_TASKS.json", "inputs/LEGACY_TEST_MATRIX.json", "inputs/CAPABILITIES.tsv",
    }
    if not required.issubset(set(expected)):
        errors.append("public candidate plan manifest is missing required files")
    if errors:
        # Keep all facts in a structured report while preventing execution.
        plan_error = "; ".join(errors)
    else:
        plan_error = None
    def fixed_json(relative: str) -> dict[str, Any]:
        return load_json_bytes(git_bytes(public_commit, f"{PLAN_REL}/{relative}"), relative)

    return {
        "root": plan_root,
        "commit": PLAN_COMMIT,
        "public_commit": public_commit,
        "source_head": SOURCE_HEAD,
        "manifest": manifest,
        "manifest_sha256": sha256_bytes(git_bytes(public_commit, PUBLIC_MANIFEST_REL)),
        "files": files,
        "errors": errors,
        "checks": fixed_json("CHECKS.json"),
        "execution": fixed_json("EXECUTION_PLAN.json"),
        "capability_map": fixed_json("CAPABILITY_TASK_MAP.json"),
        "gates": fixed_json("GATES_AUTHORIZATION.json"),
        "inputs": fixed_json("INPUTS.json"),
        "legacy_tasks": fixed_json("inputs/LEGACY_TASKS.json"),
        "legacy_matrix": fixed_json("inputs/LEGACY_TEST_MATRIX.json"),
        "capabilities_tsv": git_text(public_commit, f"{PLAN_REL}/inputs/CAPABILITIES.tsv"),
        "fixed_plan_error": plan_error,
    }


def verify_obligations(plan: dict[str, Any]) -> dict[str, Any]:
    checks = plan["checks"]
    matrix = plan["legacy_matrix"]
    legacy_tasks = plan["legacy_tasks"]
    execution = plan["execution"]
    capability_map = plan["capability_map"]
    counts = {
        "legacy_tasks": len(legacy_tasks.get("tasks", [])),
        "legacy_master_tests": len(matrix.get("retained_master_rows", {})),
        "legacy_deadlines": len(matrix.get("checks", {})),
        "legacy_due": len(checks.get("legacy_due", [])),
        "new_due": len(checks.get("new_due", [])),
        "all_due": len(checks.get("legacy_due", [])) + len(checks.get("new_due", [])),
        "capability_behaviors": len(capability_map.get("rows", [])),
        "tasks": len(execution.get("tasks", [])),
    }
    ids = {
        "legacy_task_ids": [row.get("id") for row in legacy_tasks.get("tasks", [])],
        "legacy_master_test_ids": list(matrix.get("retained_master_rows", {}).keys()),
        "legacy_deadline_ids": list(matrix.get("checks", {}).keys()),
        "legacy_due_ids": [row.get("id") for row in checks.get("legacy_due", [])],
        "new_due_ids": [row.get("id") for row in checks.get("new_due", [])],
        "all_due_ids": [row.get("id") for row in [*checks.get("legacy_due", []), *checks.get("new_due", [])]],
        "capability_ids": [row.get("id") or row.get("capability_behavior") for row in capability_map.get("rows", [])],
        "task_ids": [row.get("id") for row in execution.get("tasks", [])],
    }
    errors: list[str] = []
    for key, expected in EXPECTED_OBLIGATIONS.items():
        if counts[key] != expected:
            errors.append(f"{key}: expected {expected}, observed {counts[key]}")
    for key, values in ids.items():
        if any(not isinstance(item, str) or not item for item in values):
            errors.append(f"{key} contains empty/non-string ID")
        if len(set(values)) != len(values):
            errors.append(f"{key} contains duplicate IDs")
    legacy_id_set = set(matrix.get("checks", {}))
    for row in checks.get("legacy_due", []):
        if row.get("id") not in legacy_id_set:
            errors.append(f"legacy due check missing from 157-row matrix: {row.get('id')}")
        if not isinstance(row.get("planned_test_tag"), str) or not row.get("planned_test_tag"):
            errors.append(f"legacy due check has no planned_test_tag: {row.get('id')}")
        if not isinstance(row.get("groups"), list) or not row.get("groups"):
            errors.append(f"legacy due check has no groups: {row.get('id')}")
    for row in checks.get("new_due", []):
        if row.get("group") not in GROUPS:
            errors.append(f"new due check has unknown group: {row.get('id')}")
        if not isinstance(row.get("planned_test_tag"), str) or not row.get("planned_test_tag"):
            errors.append(f"new due check has no planned_test_tag: {row.get('id')}")
    mapping = {
        "tasks": [{"id": row.get("id"), "wp": row.get("wp"), "checks": row.get("checks", [])} for row in execution.get("tasks", [])],
        "capabilities": [{"id": row.get("id") or row.get("capability_behavior"), "owning_wp": row.get("owning_wp"), "first_checks": row.get("first_checks", [])} for row in capability_map.get("rows", [])],
    }
    return {"expected": EXPECTED_OBLIGATIONS, "observed": counts, "ids": ids, "mapping": mapping, "errors": errors, "ok": not errors}


def due_rows(plan: dict[str, Any]) -> list[dict[str, Any]]:
    return [*plan["checks"].get("legacy_due", []), *plan["checks"].get("new_due", [])]


def due_groups(plan: dict[str, Any]) -> dict[str, list[str]]:
    result = {group: [] for group in GROUPS}
    for row in due_rows(plan):
        groups = row.get("groups") if isinstance(row.get("groups"), list) else [row.get("group")]
        for group in groups:
            if group in result and row.get("id") not in result[group]:
                result[group].append(row["id"])
    return result


def merge_command_metadata(
    command: dict[str, Any], overlay: dict[str, Any], context: str
) -> dict[str, Any]:
    merged = dict(command)
    for key, value in overlay.items():
        if key in merged and merged[key] != value:
            raise RunnerError(f"{context}: command_metadata conflicts with inline field {key}")
        merged[key] = value
    return merged


def read_registry() -> dict[str, list[dict[str, Any]]]:
    try:
        raw = json.loads(REGISTRY_PATH.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise RunnerError(f"cannot read runner registry: {exc}") from exc
    if not isinstance(raw, dict) or raw.get("schema") != "gogoke.s1-r4.runner-registry.v2":
        raise RunnerError("runner registry schema is not v2")
    entries = raw.get("groups")
    if not isinstance(entries, list):
        raise RunnerError("runner registry groups is not a list")
    metadata = raw.get("command_metadata", {})
    if not isinstance(metadata, dict) or any(not isinstance(key, str) or not isinstance(value, dict) for key, value in metadata.items()):
        raise RunnerError("runner registry command_metadata is malformed")
    result: dict[str, list[dict[str, Any]]] = {}
    for group_entry in entries:
        if not isinstance(group_entry, dict) or not isinstance(group_entry.get("id"), str):
            raise RunnerError("malformed registry group")
        group = group_entry["id"]
        if group in result:
            raise RunnerError(f"duplicate registry group: {group}")
        commands = group_entry.get("commands")
        if not isinstance(commands, list) or not commands:
            raise RunnerError(f"{group}: registry group has no commands")
        normalized: list[dict[str, Any]] = []
        for command in commands:
            if not isinstance(command, dict):
                normalized.append({"id": None, "_malformed_command": command})
                continue
            command_id = command.get("id")
            overlay = metadata.get(command_id, {}) if isinstance(command_id, str) else {}
            if not isinstance(overlay, dict):
                raise RunnerError(f"{group}/{command_id}: command metadata is malformed")
            normalized.append(merge_command_metadata(command, overlay, f"{group}/{command_id}"))
        result[group] = normalized
    if tuple(result) != GROUPS:
        raise RunnerError("registry groups do not exactly match the fixed 20 groups")
    known_ids = {
        command.get("id")
        for commands in result.values()
        for command in commands
        if isinstance(command, dict) and isinstance(command.get("id"), str)
    }
    extra_metadata = sorted(set(metadata) - known_ids)
    if extra_metadata:
        raise RunnerError(f"runner registry command_metadata has unknown command ids: {','.join(extra_metadata)}")
    return result


def canonical_registry_payload(value: dict[str, Any] | dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    if "groups" in value and isinstance(value.get("groups"), list):
        metadata = value.get("command_metadata", {})
        if not isinstance(metadata, dict):
            metadata = {}
        groups: list[dict[str, Any]] = []
        for group in value.get("groups", []):
            if not isinstance(group, dict):
                groups.append(group)
                continue
            commands: list[dict[str, Any]] = []
            for command in group.get("commands", []):
                if not isinstance(command, dict):
                    commands.append(command)
                    continue
                command_id = command.get("id")
                overlay = metadata.get(command_id, {})
                if not isinstance(overlay, dict):
                    raise RunnerError(f"registry/{command_id}: command metadata is malformed")
                commands.append(merge_command_metadata(command, overlay, f"registry/{command_id}"))
            groups.append({**group, "commands": commands})
        return {"schema": value.get("schema"), "groups": groups}
    return {"schema": "gogoke.s1-r4.runner-registry.v2", "groups": [{"id": group, "commands": commands} for group, commands in value.items()]}


def validate_registry(plan: dict[str, Any], registry: dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    expected_by_group = due_groups(plan)
    errors: list[str] = []
    observed_by_group: dict[str, list[str]] = {group: [] for group in GROUPS}
    all_due = {row["id"]: row for row in due_rows(plan)}
    command_count = 0
    command_ids: set[str] = set()
    for group in GROUPS:
        commands = registry[group]
        for command in commands:
            command_count += 1
            if not isinstance(command, dict):
                errors.append(f"{group}: command entry is not an object")
                continue
            cid = command.get("id")
            framework = command.get("framework")
            selector = command.get("selector")
            target = command.get("target")
            cwd = command.get("cwd")
            argv = command.get("argv")
            check_ids = command.get("check_ids")
            tags = command.get("planned_test_tags")
            observed_cases = command.get("observed_cases")
            bind_paths = command.get("bind_paths", [])
            readiness = command.get("readiness")
            readiness_reason = command.get("readiness_reason")
            evidence_layer = command.get("evidence_layer")
            qualification_scope = command.get("qualification_scope")
            platform_requirement = command.get("platform_requirement")
            candidate_required = command.get("candidate_required")
            anti_overclaim = command.get("anti_overclaim")
            if not all(isinstance(value, str) and value for value in (cid, framework, selector, target, cwd)):
                errors.append(f"{group}: command identity/cwd/target is incomplete")
            if isinstance(cid, str):
                if cid in command_ids:
                    errors.append(f"{group}/{cid}: duplicate command id")
                command_ids.add(cid)
            if framework not in FRAMEWORKS:
                errors.append(f"{group}/{cid}: unsupported framework {framework}")
            if readiness not in READINESS_STATES:
                errors.append(f"{group}/{cid}: readiness must be one of the fixed readiness states")
            elif readiness == READINESS_READY:
                if readiness_reason is not None and (not isinstance(readiness_reason, str) or not readiness_reason):
                    errors.append(f"{group}/{cid}: READY readiness_reason must be null or a non-empty string")
            elif not isinstance(readiness_reason, str) or not readiness_reason:
                errors.append(f"{group}/{cid}: non-ready command needs an explicit readiness_reason")
            if evidence_layer not in EVIDENCE_LAYERS:
                errors.append(f"{group}/{cid}: evidence_layer is missing or unsupported")
            elif qualification_scope != EVIDENCE_SCOPE_BY_LAYER[evidence_layer]:
                errors.append(f"{group}/{cid}: evidence layer/scope overclaim or mismatch")
            if platform_requirement not in PLATFORM_REQUIREMENTS:
                errors.append(f"{group}/{cid}: platform_requirement is missing or unsupported")
            if not isinstance(candidate_required, bool):
                errors.append(f"{group}/{cid}: candidate_required must be boolean")
            if not isinstance(anti_overclaim, str) or not anti_overclaim.strip():
                errors.append(f"{group}/{cid}: anti_overclaim is missing")
            if evidence_layer == "SOURCE_DIAGNOSTIC_FAKE" and platform_requirement != "host":
                errors.append(f"{group}/{cid}: source diagnostic fake cannot claim controlled platform or Owner-machine evidence")
            if not isinstance(argv, list) or not argv or not all(isinstance(item, str) and item for item in argv):
                errors.append(f"{group}/{cid}: argv is malformed")
            elif selector not in argv and selector not in " ".join(argv):
                errors.append(f"{group}/{cid}: exact selector is not in argv")
            if not isinstance(check_ids, list) or not all(isinstance(item, str) and item for item in check_ids):
                errors.append(f"{group}/{cid}: check_ids is missing")
                check_ids = []
            if not isinstance(tags, dict):
                errors.append(f"{group}/{cid}: planned_test_tags is missing")
                tags = {}
            if not isinstance(observed_cases, dict):
                errors.append(f"{group}/{cid}: observed_cases is missing")
                observed_cases = {}
            if not isinstance(bind_paths, list) or not all(isinstance(item, str) and item for item in bind_paths):
                errors.append(f"{group}/{cid}: bind_paths is malformed")
            else:
                for bind_path in bind_paths:
                    try:
                        resolve_repo(bind_path)
                    except RunnerError as exc:
                        errors.append(f"{group}/{cid}: invalid bind path {bind_path}: {exc}")
            if isinstance(cwd, str) and isinstance(target, str):
                try:
                    cwd_path = resolve_repo(cwd)
                    target_path = resolve_repo(target)
                    if not cwd_path.is_dir():
                        errors.append(f"{group}/{cid}: cwd does not exist")
                    if target_path.is_dir():
                        errors.append(f"{group}/{cid}: target is a directory")
                    if readiness == READINESS_READY and not target_path.is_file():
                        errors.append(f"{group}/{cid}: READY target does not exist; mark an explicit non-ready state")
                    if framework != "rust_libtest" and isinstance(selector, str):
                        if not target_path.as_posix().endswith(selector.replace("\\", "/")):
                            errors.append(f"{group}/{cid}: selector and target are substituted/mismatched")
                    if framework == "rust_libtest" and isinstance(selector, str):
                        if selector not in target_path.stem:
                            errors.append(f"{group}/{cid}: Rust selector does not bind the committed test target")
                except RunnerError as exc:
                    errors.append(f"{group}/{cid}: {exc}")
            if any(token in str(selector) for token in ("*", "?", "{", "}")):
                errors.append(f"{group}/{cid}: selector is not exact")
            for check_id in check_ids:
                if check_id not in all_due:
                    errors.append(f"{group}/{cid}: unknown check_id {check_id}")
                    continue
                if check_id not in expected_by_group[group]:
                    errors.append(f"{group}/{cid}: check_id {check_id} does not belong to group {group}")
                expected_tag = all_due[check_id].get("planned_test_tag")
                if tags.get(check_id) != expected_tag:
                    errors.append(f"{group}/{cid}: planned tag mismatch for {check_id}")
                expected_cases = observed_cases.get(check_id)
                if not isinstance(expected_cases, list) or not expected_cases or not all(isinstance(item, str) and item for item in expected_cases):
                    errors.append(f"{group}/{cid}: independently observable case marker missing for {check_id}")
                observed_by_group[group].append(check_id)
            # A command that declares no checks is allowed only for the
            # qualification group; every due check must have an explicit tag.
            if group != "qualification" and not check_ids:
                errors.append(f"{group}/{cid}: command masks all due checks")
    for group in GROUPS:
        missing = sorted(set(expected_by_group[group]) - set(observed_by_group[group]))
        if missing:
            errors.append(f"{group}: due checks have no exact registry command: {','.join(missing)}")
    return {"errors": errors, "ok": not errors, "command_count": command_count, "expected_by_group": expected_by_group, "observed_by_group": observed_by_group}


def bind_registry(candidate: dict[str, Any], supplied: dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    """Bind an in-memory registry argument to both candidate and worktree bytes."""

    errors: list[str] = []
    supplied_digest = digest_json(canonical_registry_payload(supplied))
    try:
        worktree_bytes = REGISTRY_PATH.read_bytes()
        worktree_registry = json.loads(worktree_bytes.decode("utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        return {"supplied_digest": supplied_digest, "worktree_digest": None, "candidate_digest": None, "errors": [f"worktree registry cannot be loaded: {exc}"]}
    if not isinstance(worktree_registry, dict):
        errors.append("worktree registry JSON root is not an object")
        worktree_registry = {}
    worktree_digest = digest_json(canonical_registry_payload(worktree_registry))
    candidate_digest: str | None = None
    try:
        candidate_bytes = git_bytes(candidate["commit"], REGISTRY_REL)
        candidate_registry = json.loads(candidate_bytes.decode("utf-8"))
        candidate_digest = digest_json(canonical_registry_payload(candidate_registry))
        if worktree_bytes != candidate_bytes:
            errors.append("worktree registry bytes differ from candidate registry blob")
        if candidate_digest != worktree_digest:
            errors.append("candidate registry canonical digest differs from worktree registry")
    except (RunnerError, UnicodeError, json.JSONDecodeError) as exc:
        errors.append(f"candidate registry cannot be loaded: {exc}")
    if supplied_digest != worktree_digest:
        errors.append("supplied in-memory registry differs from freshly loaded worktree registry")
    if candidate_digest is not None and supplied_digest != candidate_digest:
        errors.append("supplied in-memory registry differs from candidate registry blob")
    return {"supplied_digest": supplied_digest, "worktree_digest": worktree_digest, "candidate_digest": candidate_digest, "errors": errors, "ok": not errors}


def validate_auth_value(value: dict[str, Any], binding: dict[str, Any], manifest_blob: str) -> list[str]:
    errors: list[str] = []
    required = {
        "schema": binding["authorization_receipt_schema"],
        "owner_instruction": AUTH_OWNER_INSTRUCTION,
        "repository": PUBLIC_REPOSITORY,
        "continuous_after_gate_pass": True,
        "completion_claim": False,
    }
    for key, expected in required.items():
        if type(value.get(key)) is not type(expected) or value.get(key) != expected:
            errors.append(f"authorization {key} expected {expected!r}, observed {value.get(key)!r}")
    expected_validity = {
        "path": AUTH_REL,
        "introduced_by": "a merge commit on taiyun668/gogoke main whose GitHub merged_by is taiyun668",
        "missing_stale_or_inconsistent": "FAIL_CLOSED",
    }
    expected_plan = {
        "provenance_plan_commit": PLAN_COMMIT,
        "public_plan_path": f"{PLAN_REL}/",
        "public_plan_manifest_blob": manifest_blob,
        "stale_when": "the public plan MANIFEST.json blob differs from the value above",
    }
    expected_carryover = {
        "source_archive_repository": "taiyun668/gogo-party",
        "source_archive_head": "b976e8f29f8d41adffa9ee60d3fe464a2fc3505e",
        "public_content_import_commit": "f3136b0a84086d7b5f77abb1f87e854648cd54e3",
        "public_carryover_checkpoint": "MC-001",
    }
    for key, expected in (("validity", expected_validity), ("plan", expected_plan), ("carryover", expected_carryover)):
        if value.get(key) != expected:
            errors.append(f"authorization {key} differs from the public binding")
    if value.get("authorized_gates") != ["G0", "G1", "G2", "G3", "G4", "G5"]:
        errors.append("authorization authorized_gates is not exactly ordered G0-G5")
    if set(value) != set(required) | {"validity", "plan", "carryover", "authorized_gates"}:
        errors.append("authorization field set differs from the Owner draft")
    return errors


def public_binding(candidate_commit: str) -> tuple[dict[str, Any] | None, str | None, list[str]]:
    errors: list[str] = []
    try:
        binding_bytes = git_bytes(candidate_commit, PUBLIC_BINDING_REL)
        manifest_bytes = git_bytes(candidate_commit, PUBLIC_MANIFEST_REL)
        binding = load_json_bytes(binding_bytes, PUBLIC_BINDING_REL)
        manifest = load_json_bytes(manifest_bytes, PUBLIC_MANIFEST_REL)
        if binding.get("schema") != "gogoke.s1-r4.public-execution-binding.v1" or binding.get("repository") != PUBLIC_REPOSITORY:
            errors.append("public execution binding repository/schema mismatch")
        if binding.get("provenance_plan_commit") != PLAN_COMMIT or binding.get("authorization_receipt_path") != AUTH_REL or binding.get("authorization_receipt_schema") != "gogoke.s1-r4.public-authorization.v1":
            errors.append("public execution binding provenance/authorization mismatch")
        expected_hash = manifest.get("sha256", {}).get("PUBLIC_EXECUTION_BINDING.json")
        if expected_hash != hashlib.sha256(binding_bytes).hexdigest():
            errors.append("public execution binding does not match current plan manifest")
        oid, oid_error = git_oid(candidate_commit, PUBLIC_MANIFEST_REL)
        if oid_error or not oid:
            errors.append(f"current public plan manifest is not a committed blob: {oid_error}")
        return binding, oid, errors
    except (RunnerError, TypeError, AttributeError) as exc:
        return None, None, [f"public plan binding unavailable: {exc}"]


def authorization_introduction(candidate_commit: str) -> tuple[str | None, list[str]]:
    process = git_run(["log", "--first-parent", "-m", "--diff-filter=A", "--format=%H", candidate_commit, "--", AUTH_REL])
    if process.returncode != 0:
        return None, ["authorization introduction history cannot be read"]
    commits = process.stdout.decode("ascii", "replace").splitlines()
    if len(commits) != 1 or not re.fullmatch(r"[0-9a-f]{40}", commits[0]):
        return None, ["authorization must have exactly one first-parent introduction commit"]
    merge = commits[0]
    parents = git_run(["rev-list", "--parents", "-n", "1", merge])
    if parents.returncode != 0 or len(parents.stdout.decode("ascii", "replace").split()) != 3:
        return None, ["authorization introduction is not an ordinary two-parent merge commit"]
    ancestor = git_run(["merge-base", "--is-ancestor", merge, candidate_commit])
    if ancestor.returncode != 0:
        return None, ["authorization merge is not a candidate ancestor"]
    return merge, []


def owner_merged_public_authorization(merge_commit: str) -> tuple[bool, str | None]:
    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        return False, "read-only GITHUB_TOKEN unavailable"
    url = f"https://api.github.com/repos/{PUBLIC_REPOSITORY}/commits/{merge_commit}/pulls"
    request = urllib.request.Request(url, headers={
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {token}",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            pulls = json.load(response)
        if not isinstance(pulls, list):
            return False, "GitHub associated-PR response is not a list"
        for item in pulls:
            if item.get("merge_commit_sha") != merge_commit or item.get("base", {}).get("ref") != "main":
                continue
            if item.get("base", {}).get("repo", {}).get("full_name") != PUBLIC_REPOSITORY:
                continue
            number = item.get("number")
            if type(number) is not int:
                continue
            detail_request = urllib.request.Request(
                f"https://api.github.com/repos/{PUBLIC_REPOSITORY}/pulls/{number}",
                headers=request.headers,
            )
            with urllib.request.urlopen(detail_request, timeout=15) as response:
                detail = json.load(response)
            if (detail.get("merge_commit_sha") == merge_commit and detail.get("merged_at")
                    and detail.get("merged_by", {}).get("login") == "taiyun668"
                    and detail.get("base", {}).get("ref") == "main"
                    and detail.get("base", {}).get("repo", {}).get("full_name") == PUBLIC_REPOSITORY):
                return True, None
        return False, "GitHub has no matching main PR merged_by taiyun668"
    except (OSError, ValueError, TypeError, AttributeError) as exc:
        return False, f"GitHub authorization lookup failed closed: {type(exc).__name__}"


def auth_from_candidate(candidate: dict[str, Any]) -> tuple[dict[str, Any] | None, list[str]]:
    errors: list[str] = []
    commit = candidate.get("commit") or "HEAD"
    binding, manifest_blob, binding_errors = public_binding(commit)
    errors.extend(binding_errors)
    if binding is None or manifest_blob is None or errors:
        return None, errors
    oid, oid_error = git_oid(commit, AUTH_REL)
    if oid_error or not oid:
        errors.append(f"authorization receipt is not a committed blob: {oid_error or AUTH_REL}")
        return None, errors
    try:
        value = load_json_bytes(git_bytes(commit, AUTH_REL), AUTH_REL)
    except RunnerError as exc:
        errors.append(str(exc))
        return None, errors
    errors.extend(validate_auth_value(value, binding, manifest_blob))
    merge, introduction_errors = authorization_introduction(commit)
    errors.extend(introduction_errors)
    try:
        if merge and git_bytes(merge, AUTH_REL) != git_bytes(commit, AUTH_REL):
            errors.append("authorization receipt blob differs from Owner merge introduction")
    except RunnerError as exc:
        errors.append(f"authorization introduction blob unavailable: {exc}")
    if merge:
        owner_merged, github_error = owner_merged_public_authorization(merge)
        if not owner_merged:
            errors.append(github_error or "authorization merge not verified as Owner-merged")
    value["owner_merge_commit"] = merge
    value["public_plan_manifest_blob"] = manifest_blob
    value["git_blob"] = oid
    value["path"] = AUTH_REL
    value["sha256"] = sha256_bytes(git_bytes(commit, AUTH_REL))
    return value, errors


def candidate_blob_record(candidate_commit: str, relative: str) -> dict[str, Any]:
    oid, error = git_oid(candidate_commit, relative)
    record: dict[str, Any] = {"path": relative, "committed": bool(oid), "git_blob": oid, "sha256": None, "bytes": None, "error": error}
    if oid:
        data = git_bytes(candidate_commit, relative)
        record["sha256"] = sha256_bytes(data)
        record["bytes"] = len(data)
    return record


def candidate_target(candidate_commit: str, relative: str) -> dict[str, Any]:
    record = candidate_blob_record(candidate_commit, relative)
    record["working_tree_exists"] = (ROOT / relative).is_file()
    return record


def git_tree_files(commit: str, path: str) -> list[str]:
    try:
        process = git_run(["ls-tree", "-r", "--name-only", commit, "--", path])
    except RunnerError:
        return []
    if process.returncode != 0:
        return []
    return [line for line in process.stdout.decode("utf-8", "replace").splitlines() if line]


def bound_paths(entry: dict[str, Any], candidate_commit: str) -> list[str]:
    paths = [REGISTRY_REL, "tools/gogoke-s1-r4/run_checks.py", entry["target"]]
    for relative in entry.get("bind_paths", []):
        if relative not in paths:
            paths.append(relative)
    cwd = entry.get("cwd")
    if cwd == "apps/desktop":
        paths.extend(["apps/desktop/package.json", "apps/desktop/package-lock.json", "apps/desktop/vite.config.ts", "apps/desktop/tsconfig.json"])
    elif cwd == "third_party/t3code":
        paths.extend(["third_party/t3code/package.json", "third_party/t3code/pnpm-lock.yaml", "third_party/t3code/pnpm-workspace.yaml", "third_party/t3code/vite.config.ts", "third_party/t3code/tsconfig.base.json"])
    elif entry.get("framework") == "rust_libtest":
        paths.extend(["rust-toolchain.toml", "apps/desktop/src-tauri/Cargo.toml", "apps/desktop/src-tauri/Cargo.lock"])
    expanded: list[str] = []
    for relative in paths:
        tree_files = git_tree_files(candidate_commit, relative)
        expanded.extend(tree_files or [relative])
    return list(dict.fromkeys(expanded))


def bind_worktree(entry: dict[str, Any], candidate: dict[str, Any]) -> tuple[list[dict[str, Any]], list[str]]:
    records: list[dict[str, Any]] = []
    errors: list[str] = []
    for relative in bound_paths(entry, candidate["commit"]):
        candidate_record = candidate_blob_record(candidate["commit"], relative)
        path = ROOT / relative
        actual_exists = path.is_file()
        actual_sha = sha256_bytes(path.read_bytes()) if actual_exists else None
        record = {"path": relative, "candidate": candidate_record, "worktree_exists": actual_exists, "worktree_sha256": actual_sha}
        records.append(record)
        if not candidate_record["committed"]:
            errors.append(f"bound path is not a candidate blob: {relative}")
        elif not actual_exists:
            errors.append(f"bound path is missing from worktree: {relative}")
        elif actual_sha != candidate_record["sha256"]:
            errors.append(f"bound path differs from candidate blob: {relative}")
    return records, errors


def expected_lockfiles(entry: dict[str, Any]) -> list[str]:
    if entry.get("cwd") == "apps/desktop":
        return ["apps/desktop/package.json", "apps/desktop/package-lock.json"]
    if entry.get("cwd") == "third_party/t3code":
        return ["third_party/t3code/package.json", "third_party/t3code/pnpm-lock.yaml"]
    if entry.get("framework") == "rust_libtest":
        return ["rust-toolchain.toml", "apps/desktop/src-tauri/Cargo.toml", "apps/desktop/src-tauri/Cargo.lock"]
    return []


def allowed_dependency_roots(entry: dict[str, Any]) -> list[str]:
    roots, _ = derived_dependency_roots(entry)
    return roots


def wrapper_entry_files(entry: dict[str, Any]) -> tuple[list[str], list[str]]:
    program = Path(entry["argv"][0]).name.lower()
    if program in {"npm", "npm.cmd", "npm.ps1"}:
        tool = resolve_tool(entry["argv"][0])
        if not tool:
            return [], ["npm executable cannot be resolved"]
        root = Path(tool).parent / "node_modules" / "npm" / "bin"
        files = [str((root / "npm-cli.js").resolve()), str((root / "npm-prefix.js").resolve())]
        return files, [] if all(Path(path).is_file() for path in files) else ["npm semantic wrapper entry files cannot be derived"]
    if program in {"pnpm", "pnpm.cmd", "pnpm.ps1"}:
        node = resolve_tool("node")
        if not node:
            return [], ["node executable cannot be resolved for corepack"]
        root = Path(node).parent / "node_modules" / "corepack" / "dist"
        files = [str((root / "corepack.js").resolve()), str((root / "pnpm.js").resolve())]
        return files, [] if all(Path(path).is_file() for path in files) else ["corepack/pnpm semantic wrapper entry files cannot be derived"]
    return [], []


def derived_dependency_roots(entry: dict[str, Any]) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    roots: list[str] = []
    if entry.get("framework") == "vitest_json" and entry.get("cwd") == "apps/desktop":
        roots.append(str((ROOT / "apps/desktop/node_modules").resolve()))
    elif entry.get("framework") == "vitest_json" and entry.get("cwd") == "third_party/t3code":
        roots.append(str((ROOT / "third_party/t3code/node_modules").resolve()))
    elif entry.get("framework") == "rust_libtest":
        errors.append("Rust closure roots require an explicit Controller-defined sysroot/tool receipt")
    wrapper_files, wrapper_errors = wrapper_entry_files(entry)
    errors.extend(wrapper_errors)
    program = Path(entry["argv"][0]).name.lower()
    if program in {"npm", "npm.cmd", "npm.ps1"} and wrapper_files:
        roots.append(str((Path(wrapper_files[0]).parent.parent).resolve()))
    elif program in {"pnpm", "pnpm.cmd", "pnpm.ps1"} and wrapper_files:
        roots.append(str((Path(wrapper_files[0]).parent.parent).resolve()))
    return roots, errors


def required_runtime_tools(entry: dict[str, Any]) -> list[str]:
    program = Path(entry["argv"][0]).name.lower()
    if program in {"npm", "npm.cmd", "npm.ps1", "pnpm", "pnpm.cmd", "pnpm.ps1"}:
        node = resolve_tool("node")
        return [node] if node else []
    if program in {"cargo", "cargo.exe"}:
        rustc = resolve_tool("rustc")
        return [rustc] if rustc else []
    return []


def git_is_ancestor(base: str, current: str) -> tuple[bool, str | None]:
    try:
        process = git_run(["merge-base", "--is-ancestor", base, current])
    except RunnerError as exc:
        return False, str(exc)
    if process.returncode == 0:
        return True, None
    if process.returncode == 1:
        return False, "qualified_base_commit is not an ancestor of current candidate"
    return False, process.stderr.decode("utf-8", "replace").strip() or "git merge-base failed"


def git_diff_paths(base: str, current: str) -> tuple[list[str], str | None]:
    try:
        process = git_run(["diff", "--name-only", f"{base}..{current}"])
    except RunnerError as exc:
        return [], str(exc)
    if process.returncode != 0:
        return [], process.stderr.decode("utf-8", "replace").strip() or "git diff failed"
    return [line for line in process.stdout.decode("utf-8", "replace").splitlines() if line], None


def validate_qualified_base(value: dict[str, Any], candidate: dict[str, Any]) -> list[str]:
    try:
        base = value["qualified_base_commit"]
        ancestor, ancestor_error = git_is_ancestor(base, candidate["commit"])
        changed_paths, diff_error = git_diff_paths(base, candidate["commit"])
        base_receipt_oid, base_receipt_error = git_oid(base, TOOLCHAIN_RECEIPT_REL)
    except (KeyError, TypeError, ValueError, OSError) as exc:
        return [f"qualified base validation malformed: {type(exc).__name__}: {exc}"]
    errors: list[str] = []
    if base_receipt_oid or base_receipt_error:
        errors.append(base_receipt_error or "qualified_base_commit already contains the toolchain receipt")
    if not ancestor or ancestor_error:
        errors.append(ancestor_error or "qualified_base_commit is not an ancestor")
    if diff_error:
        errors.append(diff_error)
    elif set(changed_paths) != {TOOLCHAIN_RECEIPT_REL}:
        errors.append("qualified candidate changed paths beyond the receipt")
    return errors


def deterministic_tree_digest(root: Path, exclusions: list[str]) -> tuple[str | None, str | None]:
    if not root.is_dir():
        return None, "dependency closure root is missing"
    excluded = {item.replace("\\", "/").strip("/") for item in exclusions}
    rows: list[dict[str, Any]] = []
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if any(relative == item or relative.startswith(item + "/") for item in excluded):
            continue
        try:
            if path.is_symlink():
                rows.append({"path": relative, "type": "symlink", "size": len(str(path.readlink())), "sha256": sha256_bytes(str(path.readlink()).encode("utf-8"))})
            elif path.is_dir():
                rows.append({"path": relative, "type": "directory", "size": 0, "sha256": None})
            elif path.is_file():
                data = path.read_bytes()
                rows.append({"path": relative, "type": "file", "size": len(data), "sha256": sha256_bytes(data)})
            else:
                return None, f"unsupported dependency closure entry: {relative}"
        except OSError as exc:
            return None, f"dependency closure read failed for {relative}: {exc}"
    return digest_json(rows), None


def validate_toolchain_receipt(value: dict[str, Any], entry: dict[str, Any], candidate: dict[str, Any], expected_command_ids: set[str] | None = None) -> list[str]:
    try:
        return _validate_toolchain_receipt_impl(value, entry, candidate, expected_command_ids)
    except (AttributeError, KeyError, OSError, TypeError, ValueError) as exc:
        return [f"toolchain receipt malformed: {type(exc).__name__}: {exc}"]


def validate_all_toolchain_receipt(value: dict[str, Any], candidate: dict[str, Any], registry: dict[str, list[dict[str, Any]]]) -> list[str]:
    commands = [command for group in registry.values() for command in group]
    expected_ids = {command.get("id") for command in commands}
    errors: list[str] = []
    if not isinstance(value, dict) or not isinstance(value.get("entries"), dict) or set(value.get("entries", {})) != expected_ids:
        errors.append("toolchain receipt entries do not exactly match the full registry command set")
    for command in commands:
        errors.extend(validate_toolchain_receipt(value, command, candidate, expected_ids))
    return list(dict.fromkeys(errors))


def _validate_toolchain_receipt_impl(value: dict[str, Any], entry: dict[str, Any], candidate: dict[str, Any], expected_command_ids: set[str] | None = None) -> list[str]:
    errors: list[str] = []
    if not isinstance(value, dict) or not isinstance(entry, dict) or not isinstance(candidate, dict):
        return ["toolchain receipt input is not an object"]
    allowed_root = {"schema", "qualified_base_commit", "entries"}
    if set(value) != allowed_root or value.get("schema") != TOOLCHAIN_RECEIPT_SCHEMA or not isinstance(value.get("qualified_base_commit"), str) or not re.fullmatch(r"[0-9a-fA-F]{40}", value.get("qualified_base_commit", "")):
        errors.append("toolchain receipt root schema/qualified_base_commit is invalid")
    entries = value.get("entries")
    if not isinstance(entries, dict):
        return errors + ["toolchain receipt entries is not an object"]
    expected_ids = expected_command_ids or {entry.get("id")}
    if set(entries) != expected_ids:
        errors.append("toolchain receipt has missing or extra command entries")
    item = entries.get(entry.get("id"))
    if not isinstance(item, dict):
        return errors + ["toolchain receipt command entry is missing"]
    if item.get("state") == "BLOCKED_UNQUALIFIED":
        if set(item) != {"state", "command_id", "framework", "reason_code"}:
            errors.append("blocked-unqualified receipt entry has missing or extra fields")
        if item.get("command_id") != entry.get("id") or item.get("framework") != entry.get("framework"):
            errors.append("blocked-unqualified command/framework identity mismatch")
        if item.get("reason_code") not in BLOCKED_UNQUALIFIED_REASONS:
            errors.append("blocked-unqualified reason_code is not in the fixed enum")
        return errors
    allowed_item = {"state", "command_id", "framework", "argv_sha256", "primary_executable", "runtime_executables", "wrapper_entry_files", "dependency_closure_roots", "lockfile_sha256", "bound_path_sha256"}
    if set(item) != allowed_item or item.get("state") != "QUALIFIED":
        errors.append("qualified receipt command entry has missing/extra fields or invalid state")
    if item.get("command_id") != entry.get("id") or item.get("framework") != entry.get("framework"):
        errors.append("qualified command/framework identity mismatch")
    if item.get("argv_sha256") != digest_json(entry.get("argv")):
        errors.append("toolchain receipt argv hash mismatch")
    resolved_primary = resolve_tool(entry["argv"][0])
    primary = item.get("primary_executable")
    if resolved_primary and isinstance(primary, dict) and str(primary.get("path", "")).lower() != resolved_primary.lower():
        errors.append("toolchain receipt primary executable does not match runtime-resolved executable")
    executables = [item.get("primary_executable"), *(item.get("runtime_executables") if isinstance(item.get("runtime_executables"), list) else [])]
    if not executables:
        errors.append("toolchain receipt has no executable closure")
    for executable in executables:
        if not isinstance(executable, dict) or set(executable) != {"path", "sha256"} or not isinstance(executable.get("path"), str) or not Path(executable["path"]).is_absolute() or not isinstance(executable.get("sha256"), str) or not re.fullmatch(r"[0-9A-Fa-f]{64}", executable["sha256"]):
            errors.append("toolchain receipt executable entry is malformed")
    required_runtime = required_runtime_tools(entry)
    runtime_paths = [item.get("path") for item in item.get("runtime_executables", [])] if isinstance(item.get("runtime_executables"), list) else []
    if set(path.lower() for path in runtime_paths if isinstance(path, str)) != set(path.lower() for path in required_runtime):
        errors.append("toolchain receipt runtime executable closure does not match command wrapper requirements")
    wrapper_files, wrapper_errors = wrapper_entry_files(entry)
    errors.extend(wrapper_errors)
    receipt_wrapper_files = item.get("wrapper_entry_files")
    if not isinstance(receipt_wrapper_files, list) or set(receipt_wrapper_files) != set(wrapper_files) or any(not isinstance(path, str) or not Path(path).is_absolute() for path in receipt_wrapper_files):
        errors.append("toolchain receipt wrapper entry file set does not match command-derived wrapper files")
    roots = item.get("dependency_closure_roots")
    allowed_roots_list, root_errors = derived_dependency_roots(entry)
    errors.extend(root_errors)
    allowed_roots = {path.lower() for path in allowed_roots_list}
    if not isinstance(roots, list) or not roots:
        if allowed_roots or entry.get("framework") == "rust_libtest":
            errors.append("toolchain receipt has no required dependency closure roots")
    else:
        root_paths = {str(root.get("path", "")).lower() for root in roots if isinstance(root, dict)}
        if not allowed_roots or root_paths != allowed_roots:
            errors.append("toolchain receipt dependency roots are not the exact command-derived allowed roots")
        for root in roots:
            if not isinstance(root, dict) or set(root) != {"path", "tree_sha256", "exclusions"} or not isinstance(root.get("path"), str) or not Path(root["path"]).is_absolute() or not isinstance(root.get("tree_sha256"), str) or not re.fullmatch(r"[0-9A-Fa-f]{64}", root["tree_sha256"]) or root.get("exclusions") != []:
                errors.append("toolchain receipt dependency closure entry is malformed")
    lock_hashes = item.get("lockfile_sha256")
    expected_locks = set(expected_lockfiles(entry))
    if not isinstance(lock_hashes, dict) or set(lock_hashes) != expected_locks or any(not isinstance(value, str) or not re.fullmatch(r"[0-9A-Fa-f]{64}", value) for value in lock_hashes.values()):
        errors.append("toolchain receipt lockfile hash set is missing, extra, or malformed")
    bound_hashes = item.get("bound_path_sha256")
    expected_bound = set(bound_paths(entry, candidate["commit"]))
    if not isinstance(bound_hashes, dict) or set(bound_hashes) != expected_bound or any(not isinstance(value, str) or not re.fullmatch(r"[0-9A-Fa-f]{64}", value) for value in bound_hashes.values()):
        errors.append("toolchain receipt bound-path hash set is missing, extra, or malformed")
    return errors


def verify_toolchain_materialization(receipt_item: dict[str, Any], entry: dict[str, Any], candidate: dict[str, Any], phase: str) -> tuple[dict[str, Any], list[str]]:
    try:
        return _verify_toolchain_materialization_impl(receipt_item, entry, candidate, phase)
    except (AttributeError, KeyError, OSError, TypeError, ValueError) as exc:
        return {"phase": phase, "errors": [f"materialization input malformed: {type(exc).__name__}: {exc}"]}, [f"materialization input malformed: {type(exc).__name__}: {exc}"]


def _verify_toolchain_materialization_impl(receipt_item: dict[str, Any], entry: dict[str, Any], candidate: dict[str, Any], phase: str) -> tuple[dict[str, Any], list[str]]:
    errors: list[str] = []
    observed: dict[str, Any] = {"phase": phase, "executables": [], "dependency_closure_roots": [], "lockfiles": []}
    executables = [receipt_item["primary_executable"], *receipt_item["runtime_executables"]]
    for executable in executables:
        path = Path(executable["path"])
        data = path.read_bytes() if path.is_file() else None
        digest = sha256_bytes(data) if data is not None else None
        observed["executables"].append({"path": str(path), "sha256": digest})
        if digest is None or digest.lower() != executable["sha256"].lower():
            errors.append(f"{phase}: executable digest mismatch: {path}")
    observed["wrapper_entry_files"] = []
    for wrapper_path in receipt_item["wrapper_entry_files"]:
        path = Path(wrapper_path)
        data = path.read_bytes() if path.is_file() else None
        digest = sha256_bytes(data) if data is not None else None
        observed["wrapper_entry_files"].append({"path": str(path), "sha256": digest})
        if digest is None:
            errors.append(f"{phase}: wrapper entry file is missing: {path}")
    for root in receipt_item["dependency_closure_roots"]:
        digest, error = deterministic_tree_digest(Path(root["path"]), [])
        observed["dependency_closure_roots"].append({"path": root["path"], "tree_sha256": digest, "exclusions": []})
        if error or digest is None or digest.lower() != root["tree_sha256"].lower():
            errors.append(f"{phase}: dependency closure digest mismatch: {root['path']}: {error or ''}")
    for relative, expected in receipt_item["lockfile_sha256"].items():
        actual = candidate_blob_record(candidate["commit"], relative)
        observed["lockfiles"].append(actual)
        worktree = ROOT / relative
        worktree_digest = sha256_bytes(worktree.read_bytes()) if worktree.is_file() else None
        actual_digest = actual.get("sha256")
        if not isinstance(actual_digest, str) or actual_digest.lower() != expected.lower() or worktree_digest is None or worktree_digest.lower() != expected.lower():
            errors.append(f"{phase}: lockfile digest mismatch: {relative}")
    observed["bound_paths"] = []
    for relative, expected in receipt_item["bound_path_sha256"].items():
        actual = candidate_blob_record(candidate["commit"], relative)
        worktree = ROOT / relative
        worktree_digest = sha256_bytes(worktree.read_bytes()) if worktree.is_file() else None
        observed["bound_paths"].append(actual)
        actual_digest = actual.get("sha256")
        if not isinstance(actual_digest, str) or actual_digest.lower() != expected.lower() or worktree_digest is None or worktree_digest.lower() != expected.lower():
            errors.append(f"{phase}: bound path digest mismatch: {relative}")
    return observed, errors


def activate_toolchain_materialization(receipt_item: dict[str, Any], entry: dict[str, Any], candidate: dict[str, Any], phase: str) -> dict[str, Any]:
    observed, errors = verify_toolchain_materialization(receipt_item, entry, candidate, phase)
    return {"proven": not errors, "observed": observed, "errors": errors}


def dependency_identity(entry: dict[str, Any], candidate: dict[str, Any]) -> dict[str, Any]:
    if entry.get("cwd") == "apps/desktop":
        root = ROOT / "apps/desktop/node_modules"
    elif entry.get("cwd") == "third_party/t3code":
        root = ROOT / "third_party/t3code/node_modules"
    else:
        root = ROOT / "apps/desktop/src-tauri/target"
    lock_identity = [candidate_blob_record(candidate["commit"], relative) for relative in expected_lockfiles(entry)]
    marker = root / ".modules.yaml" if root.name == "node_modules" else root / ".fingerprint"
    receipt = candidate_blob_record(candidate["commit"], TOOLCHAIN_RECEIPT_REL)
    result: dict[str, Any] = {"trust_class": "python_bootstrap" if entry.get("framework") == "python_unittest" else "external_requires_controller_receipt", "toolchain_receipt": {"path": TOOLCHAIN_RECEIPT_REL, "schema": TOOLCHAIN_RECEIPT_SCHEMA, "candidate": receipt}, "root": str(root), "exists": root.is_dir(), "materialized_non_candidate": root.is_dir(), "proven": entry.get("framework") == "python_unittest", "reason": None, "marker": artifact_file(marker, source="worktree_dependency_marker"), "lockfiles": lock_identity}
    if entry.get("framework") != "python_unittest":
        if not receipt["committed"]:
            result["proven"] = False
            result["reason"] = "Controller-qualified toolchain receipt is absent; external framework is BLOCKED"
            return result
        try:
            value = load_json_bytes(git_bytes(candidate["commit"], TOOLCHAIN_RECEIPT_REL), TOOLCHAIN_RECEIPT_REL)
            receipt_errors = validate_all_toolchain_receipt(value, candidate, read_registry())
            result["toolchain_receipt"]["value"] = value
            if receipt_errors:
                result["proven"] = False
                result["reason"] = "; ".join(receipt_errors)
                return result
            base_errors = validate_qualified_base(value, candidate)
            if base_errors:
                result["proven"] = False
                result["reason"] = "; ".join(base_errors)
                return result
            receipt_item = value["entries"][entry["id"]]
            if receipt_item.get("state") == "BLOCKED_UNQUALIFIED":
                result["proven"] = False
                result["reason"] = receipt_item["reason_code"]
                result["state"] = "BLOCKED_UNQUALIFIED"
                return result
            activation = activate_toolchain_materialization(receipt_item, entry, candidate, "before")
            result["materialization_before"] = activation["observed"]
            if activation["errors"]:
                result["proven"] = False
                result["reason"] = "; ".join(activation["errors"])
            else:
                result["proven"] = True
                result["reason"] = None
        except RunnerError as exc:
            result["proven"] = False
            result["reason"] = str(exc)
    return result


def os_identity() -> dict[str, Any]:
    return {
        "platform": sys.platform,
        "system": platform.system(),
        "release": platform.release(),
        "version": platform.version(),
        "machine": platform.machine(),
        "python": platform.python_version(),
    }


def resolve_tool(program: str) -> str | None:
    name = program.lower()
    choices = [program]
    if name in {"python", "python3"}:
        return sys.executable
    if name in {"npm", "npm.cmd", "npm.ps1"}:
        choices = ["npm.cmd", "npm.exe", "npm"]
    elif name in {"pnpm", "pnpm.cmd", "pnpm.ps1"}:
        choices = ["pnpm.cmd", "pnpm.exe", "pnpm"]
    elif name in {"node", "node.exe"}:
        choices = ["node.exe", "node"]
    elif name in {"cargo", "cargo.exe"}:
        choices = ["cargo.exe", "cargo"]
    for choice in choices:
        found = shutil.which(choice)
        if found:
            return found
    return None


def tool_identity(path: str | None) -> dict[str, Any]:
    if not path:
        return {"path": None, "sha256": None, "version": None}
    executable = Path(path)
    digest = sha256_bytes(executable.read_bytes()) if executable.is_file() else None
    try:
        isolated_env, _ = sanitized_environment(path)
        process = subprocess.run([path, "--version"], env=isolated_env, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=10, check=False)
        version = (process.stdout or process.stderr).strip().splitlines()[:1]
    except (OSError, subprocess.TimeoutExpired):
        version = []
    return {"path": path, "sha256": digest, "version": version[0] if version else None}


def sanitized_environment(tool_path: str | None = None) -> tuple[dict[str, str], list[str]]:
    """Construct a fresh Windows-local environment; never inherit by default."""

    allow = {
        "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "TEMP", "TMP", "OS",
        "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE", "PROCESSOR_IDENTIFIER",
        "PROCESSOR_LEVEL", "PROCESSOR_REVISION", "PROGRAMDATA", "PROGRAMFILES",
        "PROGRAMFILES(X86)", "COMMONPROGRAMFILES", "COMMONPROGRAMFILES(X86)",
    }
    removed = sorted(key for key in os.environ if key.upper() not in allow)
    env = {key: os.environ[key] for key in allow if key in os.environ}
    path_dirs: list[str] = []
    program_name = Path(tool_path).name.lower() if tool_path else ""
    tool_candidates = [tool_path, shutil.which("git"), os.environ.get("COMSPEC")]
    if "python" in program_name or not tool_path:
        tool_candidates.append(sys.executable)
    if any(name in program_name for name in ("node", "npm", "pnpm")):
        tool_candidates.extend([shutil.which("node"), shutil.which("npm"), shutil.which("pnpm")])
    if "cargo" in program_name or "rustc" in program_name:
        tool_candidates.extend([shutil.which("cargo"), shutil.which("rustc")])
    for candidate in tool_candidates:
        if candidate:
            path_dirs.append(str(Path(candidate).resolve().parent))
    system_root = env.get("SystemRoot") or env.get("SYSTEMROOT")
    if system_root:
        path_dirs.extend([str(Path(system_root) / "System32"), system_root])
    env["PATH"] = os.pathsep.join(dict.fromkeys(path_dirs))
    env.update({"CI": "1", "PYTHONDONTWRITEBYTECODE": "1", "GOGOKE_S1_R4_RUNNER": "1", "GOGOKE_S1_R4_LIVE_DISABLED": "1", "GOGOKE_S1_R4_EGRESS_DISABLED": "1"})
    return env, removed


def parse_vitest_json(path: Path) -> tuple[dict[str, Any] | None, str | None]:
    if not path.is_file():
        return None, "Vitest JSON outputFile is missing"
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        return None, f"Vitest JSON outputFile is invalid: {exc}"
    if not isinstance(value, dict):
        return None, "Vitest machine output root is not an object"
    required = ("numTotalTests", "numPassedTests", "numFailedTests", "numPendingTests", "testResults")
    if any(not isinstance(value.get(key), int) for key in required[:-1]) or not isinstance(value.get("testResults"), list):
        return None, "Vitest JSON output is missing strict count fields"
    total = value["numTotalTests"]
    passed = value["numPassedTests"]
    failed = value["numFailedTests"]
    skipped = value["numPendingTests"] + (value.get("numTodoTests") if isinstance(value.get("numTodoTests"), int) else 0)
    if min(total, passed, failed, skipped) < 0 or total != passed + failed + skipped:
        return None, "Vitest JSON count invariant failed"
    assertions: list[dict[str, Any]] = []
    for suite in value["testResults"]:
        if not isinstance(suite, dict) or not isinstance(suite.get("assertionResults"), list):
            return None, "Vitest JSON testResults lacks assertionResults"
        assertions.extend(item for item in suite["assertionResults"] if isinstance(item, dict))
    if len(assertions) != total:
        return None, "Vitest JSON assertion count does not equal numTotalTests"
    statuses = [item.get("status") for item in assertions]
    if any(status not in {"passed", "failed", "pending", "todo", "skipped"} for status in statuses):
        return None, "Vitest JSON contains unsupported assertion status"
    observed_passed = sum(status == "passed" for status in statuses)
    observed_failed = sum(status == "failed" for status in statuses)
    observed_skipped = sum(status in {"pending", "todo", "skipped"} for status in statuses)
    if (observed_passed, observed_failed, observed_skipped) != (passed, failed, skipped):
        return None, "Vitest assertion statuses contradict summary counts"
    observed = {str(item.get("fullName", "")) for item in assertions}
    return {"discovered": total, "executed": total - skipped, "passed": passed, "failed": failed, "skipped": skipped, "observed_names": sorted(observed), "machine_schema": "vitest-json"}, None


def parse_node_tap(path: Path) -> tuple[dict[str, Any] | None, str | None]:
    if not path.is_file():
        return None, "Node TAP artifact is missing"
    text = path.read_text(encoding="utf-8", errors="replace").replace("\r", "")
    lines = [line for line in text.splitlines() if not line.startswith((" ", "\t"))]
    if not any(line == "TAP version 13" for line in lines):
        return None, "Node output is not TAP version 13"
    plans = [int(match.group(1)) for line in lines if (match := re.match(r"^1\.\.(\d+)\s*$", line))]
    tests: list[tuple[str, str, bool, bool]] = []
    for line in lines:
        match = re.match(r"^(not )?ok\s+\d+\s*-\s*(.*?)(?:\s+#\s+(SKIP|TODO)\b.*)?$", line, re.I)
        if match:
            directive = (match.group(3) or "").upper()
            tests.append((match.group(2), directive, bool(match.group(1)), bool(directive)))
    comments: dict[str, int] = {}
    for line in lines:
        match = re.match(r"^#\s+(tests|pass|fail|skipped|todo)\s+(\d+)\s*$", line)
        if match:
            comments[match.group(1)] = int(match.group(2))
    if not plans or not tests or len(tests) != plans[-1] or set(comments) != {"tests", "pass", "fail", "skipped", "todo"}:
        return None, "Node TAP lacks a complete top-level plan/count summary"
    total = len(tests)
    failed = sum(item[2] for item in tests if not item[3])
    skipped = sum(item[3] and item[1] == "SKIP" for item in tests)
    todo = sum(item[3] and item[1] == "TODO" for item in tests)
    passed = total - failed - skipped - todo
    if total != comments["tests"] or passed != comments["pass"] or failed != comments["fail"] or skipped != comments["skipped"] or todo != comments["todo"] or total != plans[-1]:
        return None, "Node TAP count invariant failed"
    return {"discovered": total, "executed": total - skipped - todo, "passed": passed, "failed": failed, "skipped": skipped + todo, "observed_names": [item[0] for item in tests], "machine_schema": "tap-13"}, None


def parse_python_unittest(text: str) -> tuple[dict[str, Any] | None, str | None]:
    lines = text.replace("\r", "").splitlines()
    ran = [int(match.group(1)) for line in lines if (match := re.search(r"^Ran\s+(\d+)\s+tests?", line))]
    cases = [(match.group(1), match.group(2).lower()) for line in lines if (match := re.match(r"^(test_[A-Za-z0-9_]+)\s+\([^)]*\)\s+\.\.\.\s+(ok|FAIL|ERROR|skipped|expected failure)", line, re.I))]
    if not ran or not cases or len(cases) != ran[-1]:
        return None, "Python unittest output lacks independently enumerable cases"
    skipped = sum(status in {"skipped", "expected failure"} for _, status in cases)
    failed = sum(status in {"fail", "error"} for _, status in cases)
    total = ran[-1]
    final_ok = bool(re.search(r"^OK(?:\s|$)", "\n".join(lines), re.M))
    final_failed = bool(re.search(r"^FAILED\s*\(", "\n".join(lines), re.M))
    final_line = next((line.strip() for line in lines if re.match(r"^(?:OK|FAILED)\b", line.strip())), "")
    detail_counts: dict[str, int] = {}
    detail_match = re.search(r"\(([^)]*)\)", final_line)
    if detail_match:
        for key, value in re.findall(r"(failures|errors|skipped|expected failures)\s*=\s*(\d+)", detail_match.group(1)):
            detail_counts[key] = int(value)
    if detail_counts.get("skipped", skipped) != skipped or detail_counts.get("expected failures", sum(status == "expected failure" for _, status in cases)) != sum(status == "expected failure" for _, status in cases) or detail_counts.get("failures", sum(status == "fail" for _, status in cases)) != sum(status == "fail" for _, status in cases) or detail_counts.get("errors", sum(status == "error" for _, status in cases)) != sum(status == "error" for _, status in cases):
        return None, "Python unittest final detail counts contradict case lines"
    if failed and not final_failed:
        return None, "Python unittest final status contradicts failed case lines"
    if not failed and not skipped and not final_ok:
        return None, "Python unittest final OK status is missing"
    if final_ok and failed:
        return None, "Python unittest reports OK with failed cases"
    return {"discovered": total, "executed": total - skipped, "passed": total - failed - skipped, "failed": failed, "skipped": skipped, "observed_names": [name for name, _ in cases], "machine_schema": "python-unittest-verbose"}, None


def parse_rust_libtest(text: str) -> tuple[dict[str, Any] | None, str | None]:
    clean = re.sub(r"\x1b\[[0-9;]*m", "", text.replace("\r", ""))
    rows = re.findall(r"^test result:\s*(?:ok|FAILED)\.\s*(\d+)\s+passed;\s*(\d+)\s+failed;\s*(\d+)\s+ignored", clean, re.I | re.M)
    if not rows:
        return None, "Rust libtest summary is missing"
    passed = sum(int(row[0]) for row in rows)
    failed = sum(int(row[1]) for row in rows)
    skipped = sum(int(row[2]) for row in rows)
    total = passed + failed + skipped
    cases = [(match.group(1), match.group(2).lower()) for line in clean.splitlines() if (match := re.match(r"^test\s+(.+?)\s+\.\.\.\s+(ok|FAILED|ignored)", line))]
    if total <= 0 or not cases or len(cases) < passed + failed + skipped:
        return None, "Rust libtest reported zero tests"
    case_passed = sum(status == "ok" for _, status in cases)
    case_failed = sum(status == "failed" for _, status in cases)
    case_skipped = sum(status == "ignored" for _, status in cases)
    if (case_passed, case_failed, case_skipped) != (passed, failed, skipped):
        return None, "Rust libtest case lines contradict summary counts"
    return {"discovered": total, "executed": total - skipped, "passed": passed, "failed": failed, "skipped": skipped, "observed_names": [name for name, _ in cases], "machine_schema": "rust-libtest-summary"}, None


def parse_machine(framework: str, *, machine_path: Path, stdout: str, stderr: str) -> tuple[dict[str, Any] | None, str | None]:
    if framework == "vitest_json":
        return parse_vitest_json(machine_path)
    if framework == "node_tap":
        return parse_node_tap(machine_path)
    if framework == "python_unittest":
        return parse_python_unittest(stdout + "\n" + stderr)
    if framework == "rust_libtest":
        return parse_rust_libtest(stdout + "\n" + stderr)
    return None, f"unsupported framework: {framework}"


def command_status(counts: dict[str, Any] | None, *, exit_code: int | None, target_committed: bool, tool: str | None, error: str | None, timed_out: bool) -> tuple[str, str]:
    if not target_committed:
        return STATUS_INSTRUMENT, "selector target is not a committed candidate blob"
    if not tool:
        return STATUS_BLOCKED, "registered toolchain executable is unavailable"
    if timed_out:
        return STATUS_FAIL, "subprocess timed out"
    if exit_code != 0:
        return STATUS_FAIL, f"subprocess exited {exit_code}"
    if error or counts is None:
        return STATUS_INSTRUMENT, error or "framework machine result is missing"
    if counts["discovered"] <= 0:
        return STATUS_INSTRUMENT, "zero tests discovered or executed"
    if counts["failed"] > 0:
        return STATUS_FAIL, f"framework reported {counts['failed']} failed test(s)"
    if counts["skipped"] > 0:
        return STATUS_FAIL, f"framework reported {counts['skipped']} skipped/todo test(s)"
    if counts["executed"] <= 0:
        return STATUS_INSTRUMENT, "zero tests discovered or executed"
    return STATUS_PASS, "framework machine result passed with zero failure/skip"


def artifact_file(path: Path, *, source: str) -> dict[str, Any]:
    exists = path.is_file()
    data = path.read_bytes() if exists else b""
    return {"path": str(path), "exists": exists, "bytes": len(data), "sha256": sha256_bytes(data) if exists else None, "source": source}


def substitute_output(argv: list[str], output: Path) -> list[str]:
    return [item.replace(OUTPUT_MARKER, str(output)) for item in argv]


def run_command(entry: dict[str, Any], candidate: dict[str, Any], temp_root: Path) -> dict[str, Any]:
    group = entry["group"]
    target = entry["target"]
    target_record = candidate_target(candidate["commit"], target)
    cwd = resolve_repo(entry["cwd"])
    tool = resolve_tool(entry["argv"][0])
    output_path: Path | None = None
    if OUTPUT_MARKER in entry["argv"]:
        output_path = temp_root / f"{entry['id']}.machine.json"
    argv = substitute_output(entry["argv"], output_path or temp_root / f"{entry['id']}.machine.log")
    record: dict[str, Any] = {
        "id": entry["id"], "group": group, "framework": entry["framework"],
        "selector": entry["selector"], "check_ids": entry["check_ids"],
        "planned_test_tags": entry["planned_test_tags"], "observed_cases": entry.get("observed_cases", {}), "target": target,
        "readiness": entry.get("readiness"),
        "readiness_reason": entry.get("readiness_reason"),
        "blocked_reason": entry.get("blocked_reason", entry.get("readiness_reason")),
        "evidence_layer": entry.get("evidence_layer"),
        "qualification_scope": entry.get("qualification_scope"),
        "platform_requirement": entry.get("platform_requirement"),
        "candidate_required": entry.get("candidate_required"),
        "anti_overclaim": entry.get("anti_overclaim"),
        "target_candidate": target_record, "argv": entry["argv"], "resolved_argv": argv,
        "cwd": str(cwd), "exit_code": None, "discovered": None, "executed": None,
        "passed": None, "failed": None, "skipped": None, "stdout": "", "stderr": "",
        "stdout_sha256": None, "stderr_sha256": None, "machine_artifact": None,
        "status": STATUS_INSTRUMENT,
        "reason": None,
        "tool": tool,
        "tool_identity": (
            tool_identity(tool)
            if entry["framework"] == "python_unittest"
            else {"path": tool, "sha256": None, "version": None, "status": "NOT_PROBED_UNQUALIFIED"}
        ),
        "dependency_root": str(cwd / "node_modules") if entry["framework"] == "vitest_json" else None,
        "dependency_identity": dependency_identity(entry, candidate),
    }
    if candidate.get("dirty"):
        record["reason"] = "candidate dirty/untracked; command not executed"
        return record
    if entry.get("readiness") in READINESS_STATES and entry.get("readiness") != READINESS_READY:
        record["status"] = STATUS_BLOCKED
        record["reason"] = f"readiness {entry['readiness']}: {entry.get('readiness_reason') or 'command is not currently runnable'}"
        return record
    if not target_record["committed"]:
        record["reason"] = "selector target is not a committed candidate blob"
        return record
    if entry["framework"] != "python_unittest" and not record["dependency_identity"].get("proven"):
        record["status"] = STATUS_BLOCKED
        record["reason"] = record["dependency_identity"].get("reason") or "external toolchain trust is not proven; command is BLOCKED"
        return record
    if entry["framework"] != "python_unittest":
        record["tool_identity"] = tool_identity(tool)
    before_bindings, binding_errors = bind_worktree(entry, candidate)
    record["bindings_before"] = before_bindings
    if binding_errors:
        record["status"] = STATUS_INSTRUMENT
        record["reason"] = "; ".join(binding_errors)
        return record
    if not tool:
        record["status"], record["reason"] = command_status(None, exit_code=None, target_committed=True, tool=None, error=None, timed_out=False)
        return record
    env, removed_env = sanitized_environment(tool)
    record["removed_environment_keys"] = removed_env
    try:
        process = subprocess.run([tool, *argv[1:]], cwd=cwd, env=env, text=True, capture_output=True, encoding="utf-8", errors="replace", timeout=300, check=False)
        stdout, stderr = process.stdout, process.stderr
        record["exit_code"] = process.returncode
        timed_out = False
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout.decode("utf-8", "replace") if isinstance(exc.stdout, bytes) else (exc.stdout or "")
        stderr = exc.stderr.decode("utf-8", "replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
        record["exit_code"] = None
        timed_out = True
    except OSError as exc:
        stdout, stderr, timed_out = "", str(exc), False
        record["exit_code"] = 127
    record["stdout"] = stdout[-128000:]
    record["stderr"] = stderr[-128000:]
    record["stdout_sha256"] = sha256_bytes(stdout.encode("utf-8", "replace"))
    record["stderr_sha256"] = sha256_bytes(stderr.encode("utf-8", "replace"))
    machine_path = output_path
    if entry["framework"] == "node_tap":
        machine_path = temp_root / f"{entry['id']}.tap"
        machine_path.write_text(stdout, encoding="utf-8")
    elif entry["framework"] == "rust_libtest":
        machine_path = temp_root / f"{entry['id']}.rust.txt"
        machine_path.write_text(stdout + "\n" + stderr, encoding="utf-8")
    elif entry["framework"] == "python_unittest":
        machine_path = temp_root / f"{entry['id']}.unittest.txt"
        machine_path.write_text(stdout + "\n" + stderr, encoding="utf-8")
    if machine_path is not None:
        record["machine_artifact"] = artifact_file(machine_path, source="evidence_temp")
    counts, parse_error = parse_machine(entry["framework"], machine_path=machine_path or temp_root / "missing", stdout=stdout, stderr=stderr)
    if counts:
        for key in ("discovered", "executed", "passed", "failed", "skipped"):
            record[key] = counts[key]
        record["observed_names"] = counts.get("observed_names", [])
        record["machine_schema"] = counts.get("machine_schema")
    status, reason = command_status(counts, exit_code=record["exit_code"], target_committed=True, tool=tool, error=parse_error, timed_out=timed_out)
    after_bindings, after_errors = bind_worktree(entry, candidate)
    record["bindings_after"] = after_bindings
    after_dependency = dependency_identity(entry, candidate)
    record["dependency_identity_after"] = after_dependency
    if entry["framework"] != "python_unittest":
        receipt_value = after_dependency.get("toolchain_receipt", {}).get("value")
        if after_dependency.get("proven") and isinstance(receipt_value, dict):
            try:
                receipt_item = receipt_value["entries"][entry["id"]]
                materialization_after, materialization_errors = verify_toolchain_materialization(receipt_item, entry, candidate, "after")
                record["dependency_identity_after"]["materialization_after"] = materialization_after
                after_errors.extend(materialization_errors)
            except (KeyError, TypeError):
                after_errors.append("after: toolchain receipt command entry disappeared")
        elif not after_dependency.get("proven"):
            after_errors.append(after_dependency.get("reason") or "after: external toolchain trust is not proven")
    after_candidate = git_identity()
    if after_candidate.get("commit") != candidate.get("commit") or after_candidate.get("tree") != candidate.get("tree") or after_candidate.get("dirty"):
        after_errors.append("candidate HEAD/tree/status changed during command")
    if after_candidate.get("status_exit") != 0 or after_candidate.get("errors"):
        after_errors.append("git status/identity failed after command")
    if after_errors:
        status, reason = STATUS_INSTRUMENT, "; ".join(after_errors)
    record["status"], record["reason"] = status, reason
    record["result_sha256"] = digest_json({"exit_code": record["exit_code"], "stdout_sha256": record["stdout_sha256"], "stderr_sha256": record["stderr_sha256"], "machine": record["machine_artifact"]})
    return record


def auth_and_plan_identity(plan: dict[str, Any], candidate: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    auth, auth_errors = auth_from_candidate(candidate)
    identity = {
        "plan_commit": PLAN_COMMIT,
        "source_head": SOURCE_HEAD,
        "plan_root": PLAN_REL,
        "plan_manifest_sha256": plan["manifest_sha256"],
        "plan_files": plan["files"],
        "authorization": auth,
    }
    errors = list(plan["errors"]) + auth_errors
    return identity, errors


def validate_command_case_bindings(commands: list[dict[str, Any]]) -> list[str]:
    errors: list[str] = []
    for command in commands:
        command_id = command.get("id")
        if command.get("status") != STATUS_PASS:
            continue
        observed = set(command.get("observed_names", []))
        for check_id in command.get("check_ids", []):
            expected = set(command.get("observed_cases", {}).get(check_id, []))
            if not expected:
                errors.append(f"{command_id}/{check_id}: command has no expected case marker")
            elif not expected.intersection(observed):
                errors.append(f"{command_id}/{check_id}: its own command output lacks the expected case marker")
    return errors


def build_report(plan: dict[str, Any], group: str, registry: dict[str, list[dict[str, Any]]], *, invoked_argv: list[str]) -> dict[str, Any]:
    candidate = git_identity()
    obligations = verify_obligations(plan)
    registry_binding = bind_registry(candidate, registry)
    try:
        effective_registry = read_registry()
    except RunnerError:
        effective_registry = registry
    registry_check = validate_registry(plan, effective_registry)
    identity, identity_errors = auth_and_plan_identity(plan, candidate)
    preflight_errors = list(identity_errors) + obligations["errors"] + registry_binding["errors"] + registry_check["errors"]
    if candidate["dirty"]:
        preflight_errors.append("candidate worktree has dirty/untracked files")
    if candidate.get("status_exit") != 0 or candidate.get("errors"):
        preflight_errors.append("git status/identity failed at preflight")
    if not candidate.get("commit"):
        preflight_errors.append("candidate commit is unavailable")
    command_records: list[dict[str, Any]] = []
    temp_root = Path(tempfile.mkdtemp(prefix=f"gogoke-s1-r4-{group}-"))
    # Dirty candidates are never executed as evidence.  This avoids a command
    # observing one tree while the receipt claims another.
    if not preflight_errors:
        for entry in effective_registry[group]:
            command = dict(entry)
            command["group"] = group
            command_records.append(run_command(command, candidate, temp_root))
    else:
        command_records = [{
            "id": entry.get("id"),
            "group": group,
            "check_ids": entry.get("check_ids", []),
            "planned_test_tags": entry.get("planned_test_tags", {}),
            "observed_cases": entry.get("observed_cases", {}),
            "selector": entry.get("selector"),
            "readiness": entry.get("readiness"),
            "readiness_reason": entry.get("readiness_reason"),
            "blocked_reason": entry.get("blocked_reason", entry.get("readiness_reason")),
            "evidence_layer": entry.get("evidence_layer"),
            "qualification_scope": entry.get("qualification_scope"),
            "platform_requirement": entry.get("platform_requirement"),
            "candidate_required": entry.get("candidate_required"),
            "anti_overclaim": entry.get("anti_overclaim"),
            "status": STATUS_INSTRUMENT,
            "reason": "preflight failed; command not executed",
        } for entry in effective_registry[group]]
    preflight_errors.extend(validate_command_case_bindings(command_records))
    check_map: dict[str, dict[str, Any]] = {}
    for command in command_records:
        for check_id in command.get("check_ids", []):
            check_map.setdefault(check_id, {"planned_test_tag": command.get("planned_test_tags", {}).get(check_id), "commands": []})["commands"].append({
                "id": command.get("id"),
                "selector": command.get("selector"),
                "status": command.get("status"),
                "reason": command.get("reason"),
                "readiness": command.get("readiness"),
                "readiness_reason": command.get("readiness_reason"),
                "blocked_reason": command.get("blocked_reason", command.get("readiness_reason")),
                "evidence_layer": command.get("evidence_layer"),
                "qualification_scope": command.get("qualification_scope"),
                "platform_requirement": command.get("platform_requirement"),
                "candidate_required": command.get("candidate_required"),
                "anti_overclaim": command.get("anti_overclaim"),
                "expected_cases": command.get("observed_cases", {}).get(check_id, []),
                "observed_names": command.get("observed_names", []),
            })
    expected_checks = registry_check.get("expected_by_group", {}).get(group, [])
    for check_id in expected_checks:
        if check_id not in check_map:
            preflight_errors.append(f"{group}: check has no observed command: {check_id}")
        elif not check_map[check_id].get("planned_test_tag"):
            preflight_errors.append(f"{group}: check has no planned_test_tag binding: {check_id}")
        elif any(command.get("status") == STATUS_PASS for command in check_map[check_id]["commands"]) and not any(command.get("status") == STATUS_PASS and set(command.get("expected_cases", [])) & set(command.get("observed_names", [])) for command in check_map[check_id]["commands"]):
            preflight_errors.append(f"{group}: check has no observed framework-native case marker: {check_id}")
    if any(command.get("status") == STATUS_INSTRUMENT for command in command_records):
        execution_status = STATUS_INSTRUMENT
    elif any(command.get("status") == STATUS_BLOCKED for command in command_records):
        execution_status = STATUS_BLOCKED
    elif any(command.get("status") != STATUS_PASS for command in command_records):
        execution_status = STATUS_FAIL
    else:
        execution_status = STATUS_PASS
    status = STATUS_INSTRUMENT if preflight_errors else ("EVIDENCE_READY_REQUIRES_REVIEW" if execution_status == STATUS_PASS else execution_status)
    fixture_artifacts = [command.get("target_candidate") for command in command_records if command.get("target_candidate")]
    build_artifacts: list[dict[str, Any]] = []
    seen_build: set[str] = set()
    for command in registry[group]:
        target = command.get("target")
        if not isinstance(target, str):
            continue
        cwd = command.get("cwd")
        if cwd == "apps/desktop":
            candidates = ["apps/desktop/package.json", "apps/desktop/package-lock.json"]
        elif cwd == "third_party/t3code":
            candidates = ["third_party/t3code/package.json", "third_party/t3code/pnpm-lock.yaml"]
        else:
            candidates = ["apps/desktop/src-tauri/Cargo.toml", "apps/desktop/src-tauri/Cargo.lock"] if command.get("framework") == "rust_libtest" else []
        for relative in candidates:
            if relative not in seen_build:
                seen_build.add(relative)
                build_artifacts.append(candidate_blob_record(candidate.get("commit") or "HEAD", relative))
    return {
        "schema": "gogoke.s1-r4.check-run.v2",
        "recorded_utc": utc_now(),
        "runner": {"path": "tools/gogoke-s1-r4/run_checks.py", "registry_path": REGISTRY_REL, "registry_sha256": sha256_bytes(REGISTRY_PATH.read_bytes()), "self_acceptance": False},
        "group": group,
        "status": status,
        "execution_status": execution_status,
        "candidate": candidate,
        "build": {"candidate_artifacts": build_artifacts, "generated_machine_artifacts": [command.get("machine_artifact") for command in command_records if command.get("machine_artifact")]},
        "fixture": {"candidate_targets": fixture_artifacts},
        "plan": identity,
        "authorization": identity.get("authorization"),
        "os": os_identity(),
        "evidence_policy": {
            "gate_acceptance": False,
            "review_required": True,
            "accepted_layers": sorted(EVIDENCE_LAYERS),
            "source_diagnostic_fake_is_not_native_or_owner_machine": True,
        },
        "network_control": {"mode": "instruction_policy_only", "runtime_sandbox_proof": "not_available", "egress_claim": "not_verified"},
        "invocation": {"argv": invoked_argv, "cwd": str(Path.cwd()), "exit_code": None},
        "commands": command_records,
        "checks": check_map,
        "summary": {"discovered": sum(item.get("discovered") or 0 for item in command_records), "executed": sum(item.get("executed") or 0 for item in command_records), "passed": sum(item.get("passed") or 0 for item in command_records), "failed": sum(item.get("failed") or 0 for item in command_records), "skipped": sum(item.get("skipped") or 0 for item in command_records)},
        "obligation_verification": obligations,
        "registry_verification": registry_check,
        "registry_binding": registry_binding,
        "execution_temp_root": str(temp_root),
        "instrument_errors": preflight_errors,
        "observations": [
            "Plan bytes and plan hashes were read from the public candidate Git objects; PLAN_COMMIT is provenance only.",
            "Candidate source/fixture/build identities are candidate Git blobs; generated machine output is evidence-temp output.",
            "Only framework-owned machine results are accepted; prose, exit code, or a status field cannot create a pass.",
            "Network status is instruction-policy-only because this runner has no runtime sandbox proof.",
        ],
        "next_action": "Controller performs independent review; this runner does not accept a product or gate.",
    }


def output_path(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        raise RunnerError("--out must be an absolute OS temporary-root .json path")
    resolved = path.resolve()
    if resolved.suffix.lower() != ".json":
        raise RunnerError("--out must be a .json path")
    temp_root = Path(tempfile.gettempdir()).resolve()
    try:
        resolved.relative_to(temp_root)
    except ValueError as exc:
        raise RunnerError("--out must be inside the OS temporary root and never the repository") from exc
    return resolved


def write_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary_name = tempfile.mkstemp(prefix=f".{path.stem}.", suffix=".tmp", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(handle, "w", encoding="utf-8", newline="\n") as stream:
            json.dump(report, stream, indent=2, sort_keys=True, ensure_ascii=False)
            stream.write("\n")
        temporary.replace(path)
    finally:
        if temporary.exists():
            temporary.unlink()


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan-root", required=True)
    parser.add_argument("--group", required=True, choices=GROUPS)
    parser.add_argument("--out", required=True, help="absolute .json path under the OS temporary root")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        plan_root = fixed_plan_root(args.plan_root)
        plan = load_fixed_plan(plan_root)
        registry = read_registry()
        destination = output_path(args.out)
        invoked = [sys.executable, *(sys.argv[1:] if argv is None else argv)]
        report = build_report(plan, args.group, registry, invoked_argv=invoked)
        report["invocation"]["exit_code"] = 0 if report["status"] == "EVIDENCE_READY_REQUIRES_REVIEW" else 1
        write_report(destination, report)
        print(json.dumps({"status": report["status"], "group": args.group, "out": str(destination)}, ensure_ascii=False))
        return report["invocation"]["exit_code"]
    except RunnerError as exc:
        print(f"FAIL_INSTRUMENT: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
