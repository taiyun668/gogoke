# S1-R4 public qualification binding — Owner decision record

Status: **Owner decided the public rebind sequence; implementation remains subject to current-candidate review.** This does not promote any gate or due check or change GN.
Cloud build/test placement and public-source hygiene remain governed by [gogoke build and release governance](../governance/gogoke-build-and-release.md).

## Why a byte-copy is insufficient

The public plan still names `tools/gogoke-s1-r4/run_checks.py`, but the public tree does not contain the tool package. Private `7ff6f3fb` is a codec-test import change, not a tool export. Private `eca956db` contains five tool files; the later `ca7660fd` tree adds `build_qualification_manifest.py`, making six files the candidate source set.

The private runner pins `taiyun668/gogo-party`, its old execution branch/source head, and an authorization receipt at `artifacts/s1-r4/intake/AUTHORIZATION_RECEIPT.json` anchored in private history. That receipt is absent from the public repository and must not be copied or fabricated. Replacing only machine paths would leave the public runner with a false repository and authorization identity. Generated machine receipts may contain runner-local absolute paths; they are artifacts, not public source commits.

## Owner-decided public rebind sequence

1. Keep `PLAN_COMMIT=cbdc6ad592947370941024a87dbb9168a5b59055` and historical source/donor SHAs as provenance anchors. Add a separate, explicit **public execution binding** for `taiyun668/gogoke`, the current candidate SHA/ref, and the public carryover lineage. Do not reinterpret the private `SOURCE_HEAD` as the public candidate.
2. Claude's draft `artifacts/s1-r4/intake/PUBLIC_AUTHORIZATION_RECEIPT.json` records only the Owner-authorized scope and binds the applicable public plan MANIFEST blob. The plan and governance retain their existing constraints; the authorization file does not duplicate them or import private `*_authorized=false` fields. The runner must fail closed on a missing, stale, inconsistent, or non-Owner-merged receipt.
3. Carry the six source files from private `ca7660fd` as a candidate, then adjust their runner and traceability plan lookup only as needed for the approved public repository/source/authorization binding. Keep each check's stable ID, planned tag, real selector, nonzero framework count, negative control and candidate/source/toolchain hashes. A missing check target remains `FAIL_INSTRUMENT`; a missing public authorization remains blocked.
4. First merge the public plan binding after self-test and Claude review. Then rebind the runner with missing/non-Owner/plan-mismatch negative controls and Claude review. Finally open a separate PR containing only the authorization file with the merged plan MANIFEST blob; only Owner merges that PR. WP01/S1-01-V remain BLOCKED until then. Run the runner's controls, source-hygiene scan, cloud platform tests, and fresh independent review before any due-check status changes.

This decision record is the plan-revision route for the blocked WP01/S1-01-V tool export. It authorizes no runtime action, imports no private receipt, and does not claim those two scopes complete.
