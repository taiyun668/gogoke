#!/usr/bin/env python3
"""Offline revision-plan checks. No product execution, authorization, or Git writes."""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import sys
from pathlib import Path, PurePosixPath

FILES = {"PLAN.md", "PLAN.json", "verify_plan.py"}
REPO_FILES = {
    "docs/design/gogoke-s1-r4-plan-v1/PLAN.md",
    "docs/design/gogoke-s1-r4-plan-v1/MANIFEST.json",
}
LEGACY_T = set("""T56.L T57.L T05.L T61.L T68.D T10.L T39.L T55.L
T21.L T31.L T32.L T41.L T50.L T51.L T52.L T55.H T60.L T63.L
T06.L T58.L T11.L T13.L T14.L T15.L T16.L T17.L T18.L T19.L T20.L
T24.L T49.L T53.L T67.L""".split())
LEGACY_DUE = LEGACY_T | {f"R4-{n:02}" for n in range(1, 27)}
INVARIANTS = {
    "fact_ledger": "Git/GitHub",
    "local_database": "ACTIVE_COORDINATION_REF_CACHE_ONLY",
    "route_b": "TYPED_NATIVE_COORDINATION",
    "dispatch": ["prepare", "beginCommitted", "completion"],
    "unknown_no_blind_resend": True,
    "dream_candidate_only": True,
    "native_build": "CLOUD",
    "code_signing": False,
    "owner_manifest_signature": "OFFLINE_SHA256SUMS",
    "win11_actual_installer_required": True,
    "ci_candidate_only": False,
    "ci_unavailable_no_retry_rule": False,
}


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def no_constant(value):
    raise ValueError(f"non-finite JSON: {value}")


def loads(text):
    return json.loads(text, object_pairs_hook=unique_pairs, parse_constant=no_constant)


def load(path):
    return loads(path.read_text(encoding="utf-8"))


def blob(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def safe_path(base, name):
    path = PurePosixPath(name)
    if not name or "\\" in name or path.is_absolute() or ".." in path.parts:
        raise ValueError("unsafe relative path")
    target = base.joinpath(*path.parts)
    # Do not permit a symlink anywhere between the declared root and the file.
    current = base
    for part in path.parts:
        current = current / part
        if current.is_symlink():
            raise ValueError("symlink in plan path")
    if not target.resolve().is_relative_to(base.resolve()):
        raise ValueError("path escapes root")
    return target


def semantic_errors(plan):
    errors = []
    def need(condition, code):
        if not condition:
            errors.append(code)
    try:
        need(plan["schema"] == "gogoke.s1-r4.revision-plan.v2", "SCHEMA")
        need(plan["status"] == "DESIGN_PROPOSAL_NOT_AUTHORIZED", "NONCLAIM")
        need(plan["repository"] == "taiyun668/gogoke", "REPOSITORY")
        need(plan["plan_path"] == "docs/design/gogoke-s1-r4-plan-v2/", "PLAN_PATH")
        need(bool(re.fullmatch(r"[0-9a-f]{40}", plan["baseline"])), "BASELINE")
        need(plan["invariants"] == INVARIANTS, "OWNER_INVARIANTS")
        indexes = {}
        for name in ("requirements", "tasks", "checks", "assets"):
            rows = plan[name]
            need(isinstance(rows, list) and bool(rows), "EMPTY_" + name)
            indexes[name] = {row["id"]: row for row in rows}
            need(len(indexes[name]) == len(rows), "DUPLICATE_" + name)
        req, tasks, checks = (indexes[n] for n in ("requirements", "tasks", "checks"))
        need(set(checks) == {f"V{n:02}" for n in range(1, 9)}, "CHECK_SCOPE")
        need(set(tasks) == {f"R2-{n:02}" for n in range(1, 6)}, "TASK_SCOPE")
        need(all(r.get("source") and r.get("need") for r in req.values()), "REQ_SOURCE")
        used_checks, used_requirements = set(), set()
        for task in tasks.values():
            need(bool(task.get("requirements")) and set(task["requirements"]) <= req.keys(), "UNTRACED_TASK")
            need(bool(task.get("checks")) and set(task["checks"]) <= checks.keys(), "TASK_CHECK_LINK")
            need(task["status"] == "NOT_STARTED", "TASK_EXECUTION_CLAIM")
            need(bool(task.get("deliver")) and bool(task.get("areas")) and bool(task.get("stop")), "TASK_COMPLETENESS")
            need(task["integration_owner"] == "Construction Controller", "INTEGRATION_OWNER")
            need(set(task["depends_on"]) <= tasks.keys(), "UNKNOWN_DEPENDENCY")
            used_checks.update(task["checks"])
            used_requirements.update(task["requirements"])
        need(used_checks == checks.keys(), "ORPHAN_CHECK")
        need(used_requirements == req.keys(), "ORPHAN_REQUIREMENT")
        visiting, visited = set(), set()
        def visit(key):
            if key in visiting:
                raise ValueError("DAG_CYCLE")
            if key not in tasks:
                return
            if key in visited:
                return
            visiting.add(key)
            for parent in tasks[key]["depends_on"]:
                visit(parent)
            visiting.remove(key)
            visited.add(key)
        for key in tasks:
            visit(key)
        for check in checks.values():
            need(check["status"] == "NOT_RUN", "CHECK_PROMOTION")
            need(bool(check.get("requirements")) and set(check["requirements"]) <= req.keys(), "UNTRACED_CHECK")
            need(bool(check.get("observe")) and bool(check.get("negative")) and bool(check.get("evidence")), "CHECK_COMPLETENESS")
        need(checks["V07"]["evidence"] == "OWNER_MACHINE", "WIN11_EVIDENCE")
        need(checks["V06"]["evidence"] == "CLOUD_INSTALLER", "PACKAGING_EVIDENCE")
        for asset in indexes["assets"].values():
            need(bool(asset.get("reason")) and bool(asset.get("disposition")), "ASSET_REASON")
            need(set(asset["tasks"]) <= tasks.keys(), "ASSET_TASK")
            need(bool(asset["tasks"]) or asset["disposition"] == "DEFER_CURRENT_SCOPE", "UNASSIGNED_ASSET")
        rows = plan["legacy_due_disposition"]
        need(len(rows) == len(LEGACY_DUE) and {r["id"] for r in rows} == LEGACY_DUE, "LEGACY_COVERAGE")
        for row in rows:
            need(row["historical_status"] == "UNPROMOTED_MC035", "LEGACY_PROMOTION")
            need(bool(row.get("reason")) and bool(row.get("disposition")), "LEGACY_REASON")
            need(bool(row.get("targets")) and set(row["targets"]) <= checks.keys(), "LEGACY_TARGET")
        future = plan["legacy_future_checks"]
        need(future["expected_count"] == 124 and bool(future["source"]) and bool(future["selector"]) and bool(future["reason"]), "LEGACY_FUTURE_PRESERVATION")
        old = plan["legacy_schedule"]
        need(old["expected_tasks"] == 32 and set(old["replacement_tasks"]) == tasks.keys() and bool(old["reason"]), "OLD_SCHEDULE")
        legacy_tasks = plan["legacy_task_disposition"]
        expected_old_tasks = set("""S1-10A-P S1-10A-U S1-10A-T S1-01-C S1-01-G S1-01-V
S1-02-R S1-02-S S1-02-T S1-03-H S1-03-P S1-03-W S1-03-T S1-09A-P S1-09A-T
S1-04-D S1-04-E S1-04-A S1-04-T R4-Q0 R4-O-SPI R4-O-PI R4-O-NOVEL R4-C-STORE
R4-C-ASSEMBLE R4-C-LINEAGE R4-D-ENGINE R4-D-BACKENDS R4-E-SCORING R4-E-DREAM R4-V-COGNITION R4-V-FINAL""".split())
        need(len(legacy_tasks) == 32 and {r["id"] for r in legacy_tasks} == expected_old_tasks, "LEGACY_TASK_COVERAGE")
        for row in legacy_tasks:
            need(bool(row["reason"]) and bool(row["replacement_tasks"]) and set(row["replacement_tasks"]) <= tasks.keys(), "LEGACY_TASK_TARGET")
        auth = plan["authorization"]
        need(auth["receipt_path"] == "artifacts/s1-r4/intake/PUBLIC_AUTHORIZATION_RECEIPT.json", "AUTH_PATH")
        need(auth["receipt_schema"] == "gogoke.s1-r4.public-authorization.v1", "AUTH_SCHEMA")
        need(auth["required_merger"] == "taiyun668" and auth["separate_receipt_only_pr"] is True and auth["resume_before_owner_merge"] is False, "OWNER_AUTH")
        need(auth["old_manifest_blob"] == "c9efd380bb9b52a1be18b96bfc3cac4c5f5ff84b", "OLD_AUTH_ANCHOR")
        need(auth["new_blob_source"] == "git rev-parse origin/main:docs/design/gogoke-s1-r4-plan-v2/MANIFEST.json", "AUTH_BINDING_SOURCE")
    except (KeyError, TypeError, ValueError) as error:
        errors.append(str(error))
    return errors


def manifest_errors(manifest, root, repo_root):
    errors = []
    try:
        if manifest["schema"] != "gogoke.s1-r4.plan-manifest.v2":
            errors.append("MANIFEST_SCHEMA")
        if set(manifest["sha256"]) != FILES or set(manifest["repo_sha256"]) != REPO_FILES:
            errors.append("MANIFEST_INVENTORY")
        for base, key in ((root, "sha256"), (repo_root, "repo_sha256")):
            for name, digest in manifest[key].items():
                target = safe_path(base, name)
                if hashlib.sha256(target.read_bytes()).hexdigest() != digest:
                    errors.append("MANIFEST_HASH:" + name)
        old = load(repo_root / "docs/design/gogoke-s1-r4-plan-v1/MANIFEST.json")
        if old["status"] != "SUPERSEDED_NOT_EXECUTABLE" or old["construction_authorized"] is not False:
            errors.append("OLD_PLAN_NOT_RETIRED")
        if old["replacement_manifest"] != "docs/design/gogoke-s1-r4-plan-v2/MANIFEST.json":
            errors.append("OLD_PLAN_POINTER")
        if blob((repo_root / "docs/design/gogoke-s1-r4-plan-v1/MANIFEST.json").read_bytes()) == old["previous_manifest_blob"]:
            errors.append("OLD_AUTH_NOT_STALE")
    except (OSError, KeyError, TypeError, ValueError) as error:
        errors.append("MANIFEST:" + str(error))
    return errors


def self_test(plan, manifest, root, repo_root):
    variants = [
        ("untraced_task", lambda p: p["tasks"][0].update(requirements=[])),
        ("unknown_requirement", lambda p: p["tasks"][0].update(requirements=["O999"])),
        ("unknown_check", lambda p: p["tasks"][0]["checks"].append("V99")),
        ("unknown_dependency", lambda p: p["tasks"][0]["depends_on"].append("R2-99")),
        ("dependency_cycle", lambda p: p["tasks"][0]["depends_on"].append("R2-05")),
        ("duplicate_task", lambda p: p["tasks"].append(copy.deepcopy(p["tasks"][0]))),
        ("product_pass_claim", lambda p: p["checks"][0].update(status="PASS")),
        ("server_as_owner_machine", lambda p: p["checks"][6].update(evidence="CLOUD_INSTALLER")),
        ("legacy_task_dropped", lambda p: p["legacy_task_disposition"].pop()),
        ("legacy_check_dropped", lambda p: p["legacy_due_disposition"].pop()),
        ("legacy_check_promoted", lambda p: p["legacy_due_disposition"][0].update(historical_status="PASS")),
        ("unexplained_disposition", lambda p: p["assets"][0].update(reason="")),
        ("sqlite_fact_authority", lambda p: p["invariants"].update(fact_ledger="SQLite")),
        ("code_signing_reintroduced", lambda p: p["invariants"].update(code_signing=True)),
        ("candidate_only_ci_reintroduced", lambda p: p["invariants"].update(ci_candidate_only=True)),
        ("no_retry_rule_reintroduced", lambda p: p["invariants"].update(ci_unavailable_no_retry_rule=True)),
        ("resume_without_owner", lambda p: p["authorization"].update(resume_before_owner_merge=True)),
        ("auth_not_separate", lambda p: p["authorization"].update(separate_receipt_only_pr=False)),
        ("wrong_auth_schema", lambda p: p["authorization"].update(receipt_schema="unverified")),
    ]
    outcomes = []
    for name, mutate in variants:
        mutated = copy.deepcopy(plan)
        mutate(mutated)
        outcomes.append({"case": name, "rejected": bool(semantic_errors(mutated))})
    for name, mutate in [
        ("manifest_hash", lambda m: m["sha256"].update({"PLAN.md": "0" * 64})),
        ("manifest_omission", lambda m: m["sha256"].pop("PLAN.json")),
        ("manifest_traversal", lambda m: m["sha256"].update({"../outside": "0" * 64})),
    ]:
        mutated = copy.deepcopy(manifest)
        mutate(mutated)
        outcomes.append({"case": name, "rejected": bool(manifest_errors(mutated, root, repo_root))})
    for name, text in [("duplicate_json_key", '{"a": 1, "a": 2}'), ("nonfinite_json", '{"a": NaN}')]:
        try:
            loads(text)
            rejected = False
        except ValueError:
            rejected = True
        outcomes.append({"case": name, "rejected": rejected})
    return outcomes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).parent)
    parser.add_argument("--repo-root", type=Path)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    repo_root = args.repo_root.resolve() if args.repo_root else root.parents[2]
    try:
        plan = load(root / "PLAN.json")
        manifest = load(root / "MANIFEST.json")
        errors = semantic_errors(plan) + manifest_errors(manifest, root, repo_root)
        controls = self_test(plan, manifest, root, repo_root) if args.self_test and not errors else []
        if any(not item["rejected"] for item in controls):
            errors.append("NEGATIVE_CONTROL_SURVIVED")
        result = {
            "status": "FAIL_PLAN" if errors else "PASS_PLAN_STRUCTURE_ONLY",
            "errors": errors,
            "tasks": len(plan["tasks"]),
            "required_checks": len(plan["checks"]),
            "legacy_due_dispositions": len(plan["legacy_due_disposition"]),
            "asset_dispositions": len(plan["assets"]),
            "legacy_task_dispositions": len(plan["legacy_task_disposition"]),
            "manifest_git_blob": blob((root / "MANIFEST.json").read_bytes()),
            "negative_controls": {"total": len(controls), "rejected": sum(r["rejected"] for r in controls), "cases": controls},
            "runtime_verified": False,
            "owner_machine_verified": False,
            "independent_review": False,
            "authorization_granted": False,
            "github_source_pins": "coordinates recorded from connector reads; not a runtime or whole-repository byte audit",
        }
    except (OSError, ValueError, TypeError, KeyError, IndexError) as error:
        result = {"status": "FAIL_PLAN", "errors": [str(error)]}
    output = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(output, encoding="utf-8")
    print(output, end="")
    return 0 if result["status"] == "PASS_PLAN_STRUCTURE_ONLY" else 2


if __name__ == "__main__":
    sys.exit(main())
