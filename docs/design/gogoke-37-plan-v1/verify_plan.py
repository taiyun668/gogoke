#!/usr/bin/env python3
"""Offline checks for the design-37 construction plan.

Checks plan structure only: no product execution, authorization or Git writes.
A pass proves the plan is internally consistent, not that any code works.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import subprocess
import sys
from pathlib import Path, PurePosixPath

FILES = ("PLAN.md", "PLAN.json", "verify_plan.py")
MILESTONE_ORDER = ["PRE", "M1", "M2", "M3"]
REQUIRED_INVARIANTS = {
    "fact_ledger": "Git/GitHub",
    "native_build": "CLOUD",
    "code_signing": False,
    "win11_actual_installer_required": True,
    "product_semantics_do_not_authorize_construction": True,
    "infrastructure_only_no_fixed_usage": True,
    "v1_quota_integration": False,
    "fake_is_not_acceptance": True,
}


def _unique(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise ValueError(f"duplicate JSON key: {k}")
        out[k] = v
    return out


def _no_const(v):
    raise ValueError(f"non-finite JSON: {v}")


def load_json(text: str):
    return json.loads(text, object_pairs_hook=_unique, parse_constant=_no_const)


def _parts(p: str) -> tuple[str, ...]:
    return PurePosixPath(p.rstrip("/")).parts


def _within(child: str, parent: str) -> bool:
    """True when child equals parent or lies under it, by path component."""
    c, p = _parts(child), _parts(parent)
    return len(c) >= len(p) and c[: len(p)] == p


def check(plan: dict) -> list[str]:
    errors: list[str] = []
    if plan.get("status") != "DESIGN_PROPOSAL_NOT_AUTHORIZED":
        errors.append("status must stay DESIGN_PROPOSAL_NOT_AUTHORIZED")
    inv = plan.get("invariants", {})
    for k, v in REQUIRED_INVARIANTS.items():
        if inv.get(k) != v:
            errors.append(f"invariant {k} must be {v!r}")

    lines = {l["id"]: l for l in plan.get("lines", [])}
    if len(lines) != len(plan.get("lines", [])):
        errors.append("duplicate line id")

    # write scopes: no overlap between lines, no shared file inside any scope
    scopes = [(lid, s) for lid, l in lines.items() for s in l.get("write_scope", [])]
    for i, (la, sa) in enumerate(scopes):
        for lb, sb in scopes[i + 1:]:
            if la != lb and (_within(sa, sb) or _within(sb, sa)):
                errors.append(f"write scopes overlap: {la}:{sa} and {lb}:{sb}")
    shared = plan.get("shared_files", {})
    if shared.get("owner") != "INTEGRATOR":
        errors.append("shared files must have the single owner INTEGRATOR")
    paths = [f["path"] for f in shared.get("files", [])]
    if len(paths) != len(set(paths)):
        errors.append("shared file listed twice")
    for f in shared.get("files", []):
        for lid, s in scopes:
            if _within(f["path"], s):
                errors.append(f"shared file {f['path']} lies inside write scope of {lid}")
        for lid in f.get("serves", []):
            if lid not in lines:
                errors.append(f"shared file {f['path']} serves unknown line {lid}")

    # dependencies: known and acyclic
    for lid, l in lines.items():
        for d in l.get("depends", []):
            if d not in lines:
                errors.append(f"{lid} depends on unknown {d}")
    state: dict[str, int] = {}

    def visit(n: str, stack: tuple[str, ...]) -> None:
        if state.get(n) == 2:
            return
        if state.get(n) == 1:
            errors.append("dependency cycle: " + " -> ".join(stack + (n,)))
            return
        state[n] = 1
        for d in lines.get(n, {}).get("depends", []):
            if d in lines:
                visit(d, stack + (n,))
        state[n] = 2

    for lid in lines:
        visit(lid, ())

    # milestones
    ms = {m["id"]: m for m in plan.get("milestones", [])}
    if list(ms) != MILESTONE_ORDER:
        errors.append(f"milestones must be {MILESTONE_ORDER}")
    for mid, m in ms.items():
        for lid in m.get("lines", []):
            if lid not in lines:
                errors.append(f"milestone {mid} names unknown line {lid}")
    for a, b in zip(MILESTONE_ORDER[1:], MILESTONE_ORDER[2:]):
        if a in ms and b in ms and not set(ms[a]["lines"]) <= set(ms[b]["lines"]):
            errors.append(f"milestone {b} must include every line of {a}")
    for lid, l in lines.items():
        m = l.get("milestone")
        if m not in ms:
            errors.append(f"line {lid} has unknown milestone {m}")
        elif lid not in ms[m]["lines"]:
            errors.append(f"line {lid} is not included in its own milestone {m}")
        for d in l.get("depends", []):
            dm = lines.get(d, {}).get("milestone")
            if dm in MILESTONE_ORDER and m in MILESTONE_ORDER and MILESTONE_ORDER.index(dm) > MILESTONE_ORDER.index(m):
                errors.append(f"{lid} ({m}) depends on {d} which only lands at {dm}")

    # requirements <-> checks
    reqs = {r["id"] for r in plan.get("requirements", [])}
    covered: set[str] = set()
    check_ids = set()
    for c in plan.get("checks", []):
        if c["id"] in check_ids:
            errors.append(f"duplicate check {c['id']}")
        check_ids.add(c["id"])
        if c.get("milestone") not in ("M1", "M2", "M3"):
            errors.append(f"check {c['id']} must be verified on a real milestone, not {c.get('milestone')}")
        for r in c.get("requirements", []):
            if r not in reqs:
                errors.append(f"check {c['id']} names unknown requirement {r}")
            covered.add(r)
        for lid in c.get("lines", []):
            if lid not in lines:
                errors.append(f"check {c['id']} names unknown line {lid}")
            elif c.get("milestone") in ms and lid not in ms[c["milestone"]]["lines"]:
                errors.append(f"check {c['id']} needs line {lid} which is not in {c['milestone']}")
        if not c.get("observe", "").strip():
            errors.append(f"check {c['id']} has no observable result")
        if c.get("evidence") != "REAL":
            errors.append(f"check {c['id']} must be settled on the real implementation (evidence REAL), never on a fake")
    for r in sorted(reqs - covered):
        errors.append(f"requirement {r} has no check")
    for lid, l in lines.items():
        if lid in ("T00",):
            continue
        if not any(lid in c.get("lines", []) for c in plan.get("checks", [])):
            errors.append(f"line {lid} is verified by no check")

    # contracts: each has a durable owner that exists
    for k in plan.get("contracts", {}).get("operations", []):
        o = k.get("durable_owner")
        if o != "INTEGRATOR" and o not in lines:
            errors.append(f"contract {k['id']} has unknown durable owner {o}")

    # owner touchpoints
    ot = plan.get("owner_touchpoints", {})
    for t in ot.get("planned", []) + ot.get("conditional", []):
        if not t.get("why_only_owner", "").strip():
            errors.append(f"touchpoint {t.get('id')} does not say why only the Owner can do it")
    if not ot.get("conditional"):
        errors.append("conditional Owner touchpoints must be listed")

    # governance
    gov = plan.get("governance", {})
    if "docs/governance/gogoke-build-and-release.md" not in gov.get("references", []):
        errors.append("the build-and-release governance must be referenced")
    auth = plan.get("authorization", {})
    seq = auth.get("sequence", [])
    gate = "separate authorization PR"
    if "L0" not in seq or gate not in seq or seq.index("L0") < seq.index(gate):
        errors.append("construction (L0) must come after the separate authorization PR")
    return errors


def blob_oid(data: bytes) -> str:
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def check_manifest(root: Path) -> list[str]:
    errors = []
    mpath = root / "MANIFEST.json"
    if not mpath.exists():
        return ["MANIFEST.json missing"]
    man = load_json(mpath.read_text(encoding="utf-8"))
    for name in FILES:
        want = man.get("sha256", {}).get(name)
        got = hashlib.sha256((root / name).read_bytes()).hexdigest()
        if want != got:
            errors.append(f"MANIFEST sha256 mismatch for {name}")
    return errors


def check_sources(plan: dict, repo: Path, ref: str) -> list[str]:
    errors = []
    for s in plan.get("sources", []):
        try:
            got = subprocess.run(["git", "-C", str(repo), "rev-parse", f"{ref}:{s['path']}"],
                                 capture_output=True, text=True, check=True).stdout.strip()
        except subprocess.CalledProcessError:
            errors.append(f"source {s['path']} not found at {ref}")
            continue
        if got != s["blob"]:
            errors.append(f"source {s['path']} blob is {got} at {ref}, plan pins {s['blob']}")
    return errors


def self_test(plan: dict) -> list[str]:
    """Each mutation must be caught; returns the mutations that were not."""
    missed = []

    def expect(name, mutate):
        p = copy.deepcopy(plan)
        mutate(p)
        if not check(p):
            missed.append(name)

    def overlap(p):
        p["lines"][3]["write_scope"].append(p["lines"][4]["write_scope"][0] + "sub/")
    expect("overlapping write scopes", overlap)
    expect("shared file inside a scope", lambda p: p["shared_files"]["files"].append(
        {"path": p["lines"][3]["write_scope"][0] + "x.ts", "extension": "x", "serves": ["A"]}))
    expect("dependency cycle", lambda p: p["lines"][1]["depends"].append("H"))
    expect("uncovered requirement", lambda p: p["requirements"].append({"id": "R-NEW", "source": "x", "text": "x"}))
    expect("check on a fake milestone", lambda p: p["checks"][0].update(milestone="PRE"))
    expect("check settled on a fake", lambda p: p["checks"][0].update(evidence="FAKE"))
    expect("check needs a line not in its milestone", lambda p: p["checks"][0]["lines"].append("G"))
    expect("touchpoint without reason", lambda p: p["owner_touchpoints"]["planned"][0].update(why_only_owner=""))
    expect("no conditional touchpoints", lambda p: p["owner_touchpoints"].update(conditional=[]))
    expect("status changed", lambda p: p.update(status="AUTHORIZED"))
    expect("quota slipped into v1", lambda p: p["invariants"].update(v1_quota_integration=True))
    expect("code signing introduced", lambda p: p["invariants"].update(code_signing=True))
    expect("governance reference dropped", lambda p: p["governance"].update(references=[]))
    expect("construction before authorization", lambda p: p["authorization"].update(
        sequence=["L0", "separate authorization PR"]))
    return missed


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=str(Path(__file__).resolve().parent))
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--manifest", action="store_true", help="also verify MANIFEST.json hashes")
    ap.add_argument("--sources-ref", help="also verify pinned source blobs at this Git ref, e.g. origin/main")
    args = ap.parse_args()
    root = Path(args.root)
    plan = load_json((root / "PLAN.json").read_text(encoding="utf-8"))
    errors = check(plan)
    if args.manifest:
        errors += check_manifest(root)
    if args.sources_ref:
        errors += check_sources(plan, root.parents[2], args.sources_ref)
    result = {"schema": "gogoke.37.plan-validation.v1", "errors": errors}
    if args.self_test:
        missed = self_test(plan)
        result["self_test_missed"] = missed
        if missed:
            errors = errors + [f"self-test mutation not caught: {m}" for m in missed]
    result["ok"] = not errors
    print(json.dumps(result, ensure_ascii=False, indent=1))
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())
