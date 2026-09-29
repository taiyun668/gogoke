# Claude heterogeneous review: PR #27 pre-merge (head 04f4906f)

Reviewer: Claude (Opus 5.5), outside the Codex controller's context. Read-only.

Verdict: **one small pre-merge fix (governance wording), otherwise ready for the Owner's merge.**

## Checked
- PR #27: ready (not draft), MERGEABLE, head 04f4906f; origin/main is an ancestor of the head. Desktop CI 36530234173, native-host CI 36530238420 and hygiene 36530234132 all succeed on 04f4906f.
- Product bytes. Accepted delivery 161448a4 against 04f4906f under apps/, third_party/, tools/, .github/ differs in exactly two files: `third_party/t3code/apps/server/package.json` (`@opencode-ai/sdk` ^1.3.15 → 1.18.32) and `third_party/t3code/pnpm-lock.yaml` (1.15.13 → 1.18.32).
- Logic check on that exception: it is correct. The accepted product actually bundled SDK 1.18.32, because the production deploy re-resolved the caret range, while the lockfile said 1.15.13. The source therefore did not describe the shipped thing. Pinning the exact shipped version makes source match the real product and removes a moving input. This is not a new dependency choice. The 81 SDK file hashes match (MC-216), and a new full build is correctly not claimed byte-identical to the signed release.
- Outside product dirs and artifacts/, the head differs from main only in AGENTS.md, one line: the Owner-approved local-signing authorization. Correct.

## Pre-merge fix (small, do before the Owner merges)
The merge took main's version of three docs, and that dropped the Owner-approved signing wording the construction branch had already added (compare 161448a4):
- `docs/governance/gogoke-build-and-release.md`: still says the Owner signs "offline on the Owner's own machine";
- `docs/design/gogoke-s1-r4-plan-v2/PLAN.md`;
- `docs/design/gogoke-s1-r4-plan-v2/R2-06-EXECUTABLE-BYTE-STABILITY.md`.

As merged, AGENTS.md says the Controller may sign locally under the 2026-09-25 authorization, while the long-term build/release rule says only the Owner signs offline. Main would carry a contradiction. Restore the 161448a4 wording in these three files, keeping any main-side edits. This is docs only; no product byte or CI identity changes, and it needs no re-review beyond the hygiene check.

## Not findings
- The blocked recursive cleanup of the local temp download directory: correctly left alone, not bypassed. It is private, outside the repo, and not a merge concern.

This review is not an Owner acceptance and not a merge.
