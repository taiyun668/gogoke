# S1-R4 public qualification binding — proposal only

Status: **PROPOSED, NOT AUTHORIZED OR IMPLEMENTED**. This does not change the fixed S1-R4 plan, any gate, any due check, or GN.
Cloud build/test placement and public-source hygiene remain governed by [gogoke build and release governance](../governance/gogoke-build-and-release.md).

## Why a byte-copy is insufficient

The public plan still names `tools/gogoke-s1-r4/run_checks.py`, but the public tree does not contain the tool package. Private `7ff6f3fb` is a codec-test import change, not a tool export. Private `eca956db` contains five tool files; the later `ca7660fd` tree adds `build_qualification_manifest.py`, making six files the candidate source set.

The private runner pins `taiyun668/gogo-party`, its old execution branch/source head, and an authorization receipt at `artifacts/s1-r4/intake/AUTHORIZATION_RECEIPT.json` anchored in private history. That receipt is absent from the public repository and must not be copied or fabricated. Replacing only machine paths would leave the public runner with a false repository and authorization identity. Generated machine receipts may contain runner-local absolute paths; they are artifacts, not public source commits.

## Proposed contract revision for Owner/GPT decision

1. Keep `PLAN_COMMIT=cbdc6ad592947370941024a87dbb9168a5b59055` and historical source/donor SHAs as provenance anchors. Add a separate, explicit **public execution binding** for `taiyun668/gogoke`, the current candidate SHA/ref, and the public carryover lineage. Do not reinterpret the private `SOURCE_HEAD` as the public candidate.
2. Define an Owner-approved public authorization anchor and its exact immutable fields before enabling qualification. It must state the public repository and authorized scope, preserve GN=false and all forbidden operations, and bind the applicable plan revision. The runner must fail closed when this anchor is missing, stale or inconsistent. No agent may generate an authoritative receipt on the Owner's behalf.
3. Carry the six source files from private `ca7660fd` as a candidate, then adjust only the runner's repository/source/authorization lookup to the approved public binding. Keep each check's stable ID, planned tag, real selector, nonzero framework count, negative control and candidate/source/toolchain hashes. A missing check target remains `FAIL_INSTRUMENT`; a missing public authorization remains blocked.
4. Update the public plan and its manifest hashes only after the authority binding is decided. Run `verify_plan.py --root . --self-test`, the runner's own failing positive controls, a complete source-hygiene scan, cloud platform tests, and fresh independent review before any due-check status changes.

This proposal is the plan-revision route for the blocked WP01/S1-01-V tool export. It authorizes no runtime action, imports no private receipt, and does not claim those two scopes complete.
