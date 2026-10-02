#!/usr/bin/env python3
"""Named structural checks for the design-37 construction plan.

It checks the plan's structure against sets fixed in this script, independent of the
plan's own lists: required requirement IDs, required contracts, required sources and
the authorization sequence. A pass means these named checks passed. It does not read
natural language, cannot confirm that a check's evidence is real, and proves nothing
about code, cloud runs or the Owner's machine.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

FILES = ("PLAN.md", "PLAN.json", "verify_plan.py")
MILESTONES = ["PRE", "M1", "M2", "M3"]
REAL_MILESTONES = {"M1", "M2", "M3"}

# From design 37 §13a, the fixed rules it cites, and appendix A.
REQUIRED_REQUIREMENTS = {
    "R-SEAT", "R-INST", "R-SESS", "R-SIDE", "R-INBOX", "R-SEC", "R-ORCH", "R-ISO", "R-FIVE",
    "R-CALL", "R-GATE", "R-ESC", "R-CLEAN", "R-GLOBAL", "R-QCARD", "R-TAKEOVER", "R-WT",
    "R-MEMOFF", "R-HEALTH", "R-REPO", "R-CONTRACT",
    "A1-QUEUED", "A1-STEERING", "A1-TURN-ENDED", "A1-DELIVERED", "A1-UNKNOWN", "A1-CANCELLED", "A1-FAILED",
    "A2-IDLE", "A2-WORKING", "A2-SWITCHING", "A2-STOP-REQ", "A2-STUCK", "A2-RECLAIMING", "A2-RECLAIMED",
    "A3-NEW", "A3-ACTIVE", "A3-LEAD-PROGRESS", "A3-KEPT", "A3-ARCHIVED", "A3-DELETED",
    "A4-NOT-INSTALLED", "A4-NOT-LOGGED-IN", "A4-LOGGED-IN", "A4-NEW-VERSION", "A4-EXHAUSTED-OR-ERROR",
}
APPENDIX_STATES = {r for r in REQUIRED_REQUIREMENTS if r[:2] in ("A1", "A2", "A3", "A4")}
REQUIRED_CONTRACTS = {"K-SESSION", "K-LEDGER", "K-INBOX", "K-QCARD", "K-SIDE", "K-SEAT", "K-POLICY",
                      "K-INSTANCE", "K-WORKTREE", "K-UI"}
REQUIRED_SOURCES = {
    "docs/design/37-seat-session-architecture-draft.md",
    "docs/governance/gogoke-build-and-release.md",
    "docs/design/gogoke-s1-r4-plan-v2/PLAN.md",
    "AGENTS.md",
    "docs/model-routing.md",
}
REQUIRED_SEQUENCE = [
    "draft review",
    "T00 re-check at the authorization base commit against the mapping snapshot",
    "machine plan and MANIFEST finalised and merged",
    "separate authorization PR",
    "exact read-back of the merged authorization",
    "L0",
    "parallel lines",
]
REQUIRED_INVARIANTS = {
    "fact_ledger": "Git/GitHub",
    "native_build": "CLOUD",
    "code_signing": False,
    "win11_actual_installer_required": True,
    "product_semantics_do_not_authorize_construction": True,
    "infrastructure_only_no_fixed_usage": True,
    "v1_quota_integration": False,
    "fake_is_not_acceptance": True,
    "single_coordination_domain": True,
}
SEGMENT = re.compile(r"^[A-Za-z0-9_.][A-Za-z0-9_.@+-]*$")  # "." and ".." are rejected separately


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


def path_problem(p: str) -> str | None:
    """Only canonical repository-relative POSIX paths are allowed."""
    if not isinstance(p, str) or not p:
        return "empty path"
    if "\\" in p:
        return "backslash"
    if p.startswith("/") or re.match(r"^[A-Za-z]:", p):
        return "absolute path"
    body = p[:-1] if p.endswith("/") else p
    for seg in body.split("/"):
        if seg in ("", ".", ".."):
            return "empty, '.' or '..' segment"
        if not SEGMENT.match(seg):
            return f"unexpected segment {seg!r}"
    return None


def _key(p: str) -> tuple[str, ...]:
    # case-folded, because the Windows working tree is case-insensitive
    return tuple(s.casefold() for s in p.rstrip("/").split("/"))


def within(child: str, parent: str) -> bool:
    c, q = _key(child), _key(parent)
    return len(c) >= len(q) and c[: len(q)] == q


def _ids(items, kind, errors):
    ids = [i.get("id") for i in items]
    for x in {i for i in ids if ids.count(i) > 1}:
        errors.append(f"duplicate {kind} id {x}")
    return set(ids)


def check(plan: dict) -> list[str]:
    errors: list[str] = []
    if plan.get("status") != "DESIGN_PROPOSAL_NOT_AUTHORIZED":
        errors.append("status must stay DESIGN_PROPOSAL_NOT_AUTHORIZED")
    inv = plan.get("invariants", {})
    for k, v in REQUIRED_INVARIANTS.items():
        if inv.get(k) != v:
            errors.append(f"invariant {k} must be {v!r}")

    # sources
    srcs = plan.get("sources", [])
    for missing in sorted(REQUIRED_SOURCES - {s.get("path") for s in srcs}):
        errors.append(f"required source missing: {missing}")
    for s in srcs:
        if not re.fullmatch(r"[0-9a-f]{40}", s.get("blob", "")):
            errors.append(f"source {s.get('path')} has no 40-hex blob OID")

    # requirements
    reqs = _ids(plan.get("requirements", []), "requirement", errors)
    for missing in sorted(REQUIRED_REQUIREMENTS - reqs):
        errors.append(f"required requirement missing: {missing}")

    # lines and phases
    lines_list = plan.get("lines", [])
    if not lines_list:
        errors.append("no lines")
    lines = {l.get("id"): l for l in lines_list}
    _ids(lines_list, "line", errors)
    phase_ms: dict[str, str] = {}
    start: dict[str, str] = {}
    land: dict[str, str] = {}
    for lid, l in lines.items():
        phases = l.get("phases", [])
        if not phases:
            errors.append(f"line {lid} has no phases")
            continue
        order = []
        for ph in phases:
            pid, m = ph.get("id"), ph.get("milestone")
            if not isinstance(pid, str) or not pid.startswith(f"{lid}."):
                errors.append(f"phase id {pid} must start with {lid}.")
            if pid in phase_ms:
                errors.append(f"duplicate phase id {pid}")
            if m not in MILESTONES:
                errors.append(f"phase {pid} has unknown milestone {m}")
                continue
            if not ph.get("deliverables"):
                errors.append(f"phase {pid} has no deliverables")
            phase_ms[pid] = m
            order.append(MILESTONES.index(m))
        if order:
            if order != sorted(order):
                errors.append(f"phases of {lid} must not go back in time")
            start[lid], land[lid] = MILESTONES[min(order)], MILESTONES[max(order)]

    def when(ref: str) -> str | None:
        if ref in lines:
            return land.get(ref)
        return phase_ms.get(ref)

    # write scopes
    scopes = []
    for lid, l in lines.items():
        if not l.get("write_scope"):
            errors.append(f"line {lid} has no write scope")
        for s in l.get("write_scope", []):
            prob = path_problem(s)
            if prob:
                errors.append(f"write scope {lid}:{s} is not canonical ({prob})")
            scopes.append((lid, s))
    for i, (la, sa) in enumerate(scopes):
        for lb, sb in scopes[i + 1:]:
            if la != lb and (within(sa, sb) or within(sb, sa)):
                errors.append(f"write scopes overlap: {la}:{sa} and {lb}:{sb}")

    # shared files and their single writer
    owners = [lid for lid, l in lines.items() if l.get("owns_shared_files")]
    if owners != ["L0"]:
        errors.append(f"exactly one line, L0, must own the shared files; found {owners}")
    shared = plan.get("shared_files", {})
    if shared.get("owner") != "INTEGRATOR":
        errors.append("shared files must have the single owner INTEGRATOR")
    sfiles = shared.get("files", [])
    if not sfiles:
        errors.append("no shared files")
    seen = set()
    for f in sfiles:
        p = f.get("path", "")
        prob = path_problem(p)
        if prob:
            errors.append(f"shared file {p} is not canonical ({prob})")
        if _key(p) in seen:
            errors.append(f"shared file listed twice: {p}")
        seen.add(_key(p))
        for lid, s in scopes:
            if within(p, s):
                errors.append(f"shared file {p} lies inside write scope of {lid}")
        if not f.get("serves"):
            errors.append(f"shared file {p} serves no line")
        for lid in f.get("serves", []):
            if lid not in lines:
                errors.append(f"shared file {p} serves unknown line {lid}")
        if f.get("conditional") and not shared.get("conditional_rule"):
            errors.append("conditional shared files need a conditional_rule")

    # dependencies: known, acyclic, every implementation line behind L0, L0 behind T00
    for lid, l in lines.items():
        for d in l.get("depends", []):
            if d not in lines:
                errors.append(f"{lid} depends on unknown {d}")
    state: dict[str, int] = {}

    def visit(n, stack):
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

    def ancestors(n, acc=None):
        acc = set() if acc is None else acc
        for d in lines.get(n, {}).get("depends", []):
            if d in lines and d not in acc:
                acc.add(d)
                ancestors(d, acc)
        return acc

    if "T00" not in ancestors("L0"):
        errors.append("L0 must depend on T00")
    for lid in lines:
        if lid not in ("T00", "L0") and "L0" not in ancestors(lid):
            errors.append(f"line {lid} must depend on L0")
    for lid, l in lines.items():
        for d in l.get("depends", []):
            if d in start and lid in start and MILESTONES.index(land.get(d, "PRE")) > MILESTONES.index(start[lid]) and d == "L0":
                errors.append(f"{lid} starts before L0 lands")

    # milestones are fixed; membership is derived from phases
    if [m.get("id") for m in plan.get("milestones", [])] != MILESTONES:
        errors.append(f"milestones must be exactly {MILESTONES}")

    # contracts
    ops = plan.get("contracts", {}).get("operations", [])
    cids = _ids(ops, "contract", errors)
    for missing in sorted(REQUIRED_CONTRACTS - cids):
        errors.append(f"required contract missing: {missing}")
    for k in ops:
        cid = k.get("id")
        if not k.get("families"):
            errors.append(f"contract {cid} has no operation families")
        o = k.get("durable_owner")
        if o != "INTEGRATOR" and o not in lines:
            errors.append(f"contract {cid} has unknown durable owner {o}")
        if not k.get("consumers"):
            errors.append(f"contract {cid} has no consumers")
        for c in k.get("consumers", []):
            if c not in lines:
                errors.append(f"contract {cid} names unknown consumer {c}")
        rs = k.get("real_stage")
        if rs not in REAL_MILESTONES:
            errors.append(f"contract {cid} needs a real_stage in M1-M3")
        elif o in start and MILESTONES.index(rs) < MILESTONES.index(start[o]):
            errors.append(f"contract {cid} is settled at {rs} before its owner {o} delivers anything")

    # checks
    checks = plan.get("checks", [])
    if not checks:
        errors.append("no checks")
    _ids(checks, "check", errors)
    covered: set[str] = set()
    settled: dict[str, set[str]] = {m: set() for m in REAL_MILESTONES}
    for c in checks:
        cid, m = c.get("id"), c.get("milestone")
        if m not in REAL_MILESTONES:
            errors.append(f"check {cid} must be settled on M1-M3, not {m}")
            continue
        if c.get("evidence") != "REAL":
            errors.append(f"check {cid} must declare evidence REAL (settled on the real implementation, never a fake)")
        if not c.get("observe", "").strip():
            errors.append(f"check {cid} has no observable result")
        if not c.get("requirements"):
            errors.append(f"check {cid} covers no requirement")
        for r in c.get("requirements", []):
            if r not in reqs:
                errors.append(f"check {cid} names unknown requirement {r}")
            covered.add(r)
        if not c.get("lines"):
            errors.append(f"check {cid} names no line")
        for ref in c.get("lines", []):
            w = when(ref)
            if w is None:
                errors.append(f"check {cid} names unknown line or phase {ref}")
            elif MILESTONES.index(w) > MILESTONES.index(m):
                errors.append(f"check {cid} at {m} needs {ref}, which lands at {w}")
        for k in c.get("contracts", []):
            if k not in cids:
                errors.append(f"check {cid} names unknown contract {k}")
            settled[m].add(k)
    for r in sorted(reqs - covered):
        errors.append(f"requirement {r} has no check")
    for k in ops:
        cid, rs = k.get("id"), k.get("real_stage")
        if rs in settled and cid not in settled[rs]:
            errors.append(f"contract {cid} is not settled by a check at its real_stage {rs}")
    for cid in sorted(cids - settled["M3"]):
        errors.append(f"contract {cid} is not settled again at M3")
    for lid in lines:
        if lid == "T00":
            continue
        refs = {ref for c in checks for ref in c.get("lines", [])}
        if lid not in refs and not any(r.startswith(f"{lid}.") for r in refs):
            errors.append(f"line {lid} is verified by no check")
    if not any(c.get("milestone") == "M3" and APPENDIX_STATES <= set(c.get("requirements", [])) for c in checks):
        errors.append("one M3 check must cover all 25 appendix-A states")

    # evidence carry-over
    co = plan.get("evidence_carryover", {})
    if not co.get("rule") or set(co.get("result_states", [])) != {"PASS", "FAIL", "NOT_RUN"}:
        errors.append("evidence_carryover needs a rule and the result states PASS, FAIL, NOT_RUN")

    # owner touchpoints
    ot = plan.get("owner_touchpoints", {})
    for t in ot.get("planned", []) + ot.get("conditional", []):
        if not t.get("why_only_owner", "").strip():
            errors.append(f"touchpoint {t.get('id')} does not say why only the Owner can do it")
    if not ot.get("planned") or not ot.get("conditional"):
        errors.append("planned and conditional Owner touchpoints must both be listed")

    # governance and authorization
    if "docs/governance/gogoke-build-and-release.md" not in plan.get("governance", {}).get("references", []):
        errors.append("the build-and-release governance must be referenced")
    if plan.get("authorization", {}).get("sequence") != REQUIRED_SEQUENCE:
        errors.append("authorization sequence must be exactly " + " -> ".join(REQUIRED_SEQUENCE))
    if not str(plan.get("authorization", {}).get("amendment_rule", "")).strip():
        errors.append("authorization needs an amendment_rule")
    return errors


def scope_digest(plan: dict) -> str:
    """SHA-256 over the parts of the plan that the Owner authorizes.

    Requirements, lines with their write scopes and dependencies, milestones, shared-file
    ownership and paths, invariants, Owner touchpoints, governance, authorization and the
    non-claims. Phase deliverables, contracts, checks, assets and source pins are details:
    amending them leaves the digest, and so the Owner's authorization, unchanged.
    """
    scope = {
        "requirements": sorted((r.get("id"), r.get("text")) for r in plan.get("requirements", [])),
        "lines": sorted(
            (l.get("id"), sorted(l.get("write_scope", [])), sorted(l.get("depends", [])), bool(l.get("owns_shared_files")))
            for l in plan.get("lines", [])),
        "milestones": [m.get("id") for m in plan.get("milestones", [])],
        "shared_files": [plan.get("shared_files", {}).get("owner"),
                         sorted(f.get("path") for f in plan.get("shared_files", {}).get("files", []))],
        "invariants": plan.get("invariants"),
        "owner_touchpoints": plan.get("owner_touchpoints"),
        "governance": plan.get("governance"),
        "authorization": plan.get("authorization"),
        "scope_nonclaims": plan.get("scope_nonclaims"),
    }
    blob = json.dumps(scope, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(blob).hexdigest()


def check_manifest(root: Path) -> list[str]:
    mpath = root / "MANIFEST.json"
    if not mpath.exists():
        return ["MANIFEST.json missing"]
    man = load_json(mpath.read_text(encoding="utf-8"))
    errors = []
    for name in FILES:
        if man.get("sha256", {}).get(name) != hashlib.sha256((root / name).read_bytes()).hexdigest():
            errors.append(f"MANIFEST sha256 mismatch for {name}")
    plan = load_json((root / "PLAN.json").read_text(encoding="utf-8"))
    if man.get("scope_digest") != scope_digest(plan):
        errors.append("MANIFEST scope_digest does not match PLAN.json")
    return errors


def check_sources(plan: dict, repo: Path, ref: str) -> list[str]:
    errors = []
    if not plan.get("sources"):
        return ["no sources to verify"]
    for s in plan["sources"]:
        r = subprocess.run(["git", "-C", str(repo), "rev-parse", f"{ref}:{s['path']}"], capture_output=True, text=True)
        if r.returncode:
            errors.append(f"source {s['path']} not found at {ref}")
        elif r.stdout.strip() != s["blob"]:
            errors.append(f"source {s['path']} blob is {r.stdout.strip()} at {ref}, plan pins {s['blob']}")
    return errors


def self_test(plan: dict) -> list[str]:
    missed = []

    def expect(name, mutate):
        p = copy.deepcopy(plan)
        mutate(p)
        if not check(p):
            missed.append(name)

    L = {l["id"]: l for l in plan["lines"]}
    idx = {l["id"]: i for i, l in enumerate(plan["lines"])}

    def setl(p, lid, **kw):
        p["lines"][idx[lid]].update(**kw)

    expect("empty plan", lambda p: p.update(lines=[], requirements=[], checks=[], sources=[],
                                            contracts={"operations": []}))
    expect("overlapping write scopes", lambda p: p["lines"][idx["A"]]["write_scope"].append(L["B1"]["write_scope"][0] + "sub/"))
    expect("'..' alias overlap", lambda p: p["lines"][idx["A"]]["write_scope"].append(
        "apps/desktop/native-host/src/store/ledger/../seat/"))
    expect("absolute write scope", lambda p: p["lines"][idx["A"]]["write_scope"].append("/tmp/x/"))
    expect("case alias overlap", lambda p: p["lines"][idx["A"]]["write_scope"].append(L["B1"]["write_scope"][0].upper()))
    expect("shared file inside a scope", lambda p: p["shared_files"]["files"].append(
        {"path": L["A"]["write_scope"][0] + "x.ts", "extension": "x", "serves": ["A"]}))
    expect("two shared-file writers", lambda p: setl(p, "A", owns_shared_files=True))
    expect("line not behind L0", lambda p: setl(p, "A", depends=[]))
    expect("dependency cycle", lambda p: p["lines"][idx["L0"]]["depends"].append("H"))
    expect("duplicate requirement", lambda p: p["requirements"].append(dict(plan["requirements"][0])))
    expect("required requirement dropped", lambda p: p.update(
        requirements=[r for r in p["requirements"] if r["id"] != "A2-STUCK"],
        checks=[dict(c, requirements=[r for r in c["requirements"] if r != "A2-STUCK"] or ["R-SESS"]) for c in p["checks"]]))
    expect("uncovered requirement", lambda p: p["requirements"].append({"id": "R-NEW", "source": "x", "text": "x"}))
    expect("required contract dropped", lambda p: p["contracts"].update(
        operations=[k for k in p["contracts"]["operations"] if k["id"] != "K-SIDE"]))
    expect("contract without families", lambda p: p["contracts"]["operations"][0].update(families=[]))
    expect("contract settled before its owner", lambda p: next(
        k for k in p["contracts"]["operations"] if k["id"] == "K-SIDE").update(real_stage="M1"))
    expect("contract not settled at M3", lambda p: [c.update(contracts=[k for k in c.get("contracts", []) if k != "K-UI"])
                                                    for c in p["checks"] if c["milestone"] == "M3"])
    expect("check needs a later phase", lambda p: p["checks"][0]["lines"].append("E.2"))
    expect("check on PRE", lambda p: p["checks"][0].update(milestone="PRE"))
    expect("check not declared REAL", lambda p: p["checks"][0].update(evidence="FAKE"))
    expect("UI check misses a state", lambda p: [c.update(requirements=[r for r in c["requirements"] if r != "A3-ARCHIVED"])
                                                 for c in p["checks"] if c["milestone"] == "M3"])
    expect("phases go back in time", lambda p: setl(p, "E", phases=list(reversed(L["E"]["phases"]))))
    expect("touchpoint without reason", lambda p: p["owner_touchpoints"]["planned"][0].update(why_only_owner=""))
    expect("no conditional touchpoints", lambda p: p["owner_touchpoints"].update(conditional=[]))
    expect("status changed", lambda p: p.update(status="AUTHORIZED"))
    expect("quota slipped into v1", lambda p: p["invariants"].update(v1_quota_integration=True))
    expect("code signing introduced", lambda p: p["invariants"].update(code_signing=True))
    expect("governance reference dropped", lambda p: p["governance"].update(references=[]))
    expect("truncated authorization sequence", lambda p: p["authorization"].update(sequence=["separate authorization PR", "L0"]))
    expect("required source dropped", lambda p: p.update(sources=[s for s in p["sources"] if s["path"] != "AGENTS.md"]))
    expect("milestone list changed", lambda p: p["milestones"].pop())
    expect("amendment rule dropped", lambda p: p["authorization"].pop("amendment_rule", None))
    return missed


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=str(Path(__file__).resolve().parent))
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--manifest", action="store_true")
    ap.add_argument("--sources-ref", help="verify pinned source blobs at this Git ref, e.g. origin/main")
    ap.add_argument("--scope-digest", metavar="PLAN_JSON", help="print the scope digest of this PLAN.json and exit")
    args = ap.parse_args()
    if args.scope_digest:
        print(scope_digest(load_json(Path(args.scope_digest).read_text(encoding="utf-8"))))
        return 0
    root = Path(args.root)
    plan = load_json((root / "PLAN.json").read_text(encoding="utf-8"))
    errors = check(plan)
    if args.manifest:
        errors += check_manifest(root)
    if args.sources_ref:
        errors += check_sources(plan, root.parents[2], args.sources_ref)
    result = {"schema": "gogoke.37.plan-validation.v1",
              "meaning": "named structural checks only; no evidence, code or Owner-machine result is confirmed",
              "errors": errors}
    if args.self_test:
        missed = self_test(plan)
        result["self_test_mutations_missed"] = missed
        errors = errors + [f"self-test mutation not caught: {m}" for m in missed]
    result["structural_checks_passed"] = not errors
    print(json.dumps(result, ensure_ascii=False, indent=1))
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())
