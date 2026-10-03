# F.2 Task Result

## CURRENT_STATE

F.2 native implementation is pushed on `codex/gogoke-37-m2-f2`; the code-bearing SHA is `c7dd46c2852937e3dace169ef75e102fb1c183ba`. This is a construction handoff, not acceptance. Overall validation state is `CI_PENDING`: Root owns the remaining exact-SHA cloud result. Real Owner Win11 product, manual CLI upgrade, installed app, real login, and real model operation are `NOT_RUN` by this seat.

## FILES_CHANGED

- `apps/desktop/native-host/src/store/instance/catalog.rs`
- `apps/desktop/native-host/src/store/instance/reprobe.rs`
- `apps/desktop/native-host/src/store/instance/mod.rs`
- `apps/desktop/native-host/src/store/worktree/mod.rs`
- `apps/desktop/native-host/src/store/worktree/f2.rs`
- `artifacts/gogoke-37/parallel/F2/SHARED_INTEGRATION.md`
- This Task Result.

## IMPORTANT_DIFF

The instance module exposes a pinned-catalog known-newer-version comparison and a read of the original durable H capability-probe receipt after a manual repin. It does not discover a version from the network or carry a prior digest's capabilities across a repin.

The worktree module creates and registers M2 single/mixed linked trees under host-generated paths, projects their graph and lifecycle, checks the current E.2 grant for a local provenance-bearing merge, and records intent before merge or cleanup. Cleanup uses the original H open request and current graph-resolved physical tree identities. It blocks overlapping parent/child workspace roots even when worktree IDs or seats differ, and allows a distinct sibling only after verified physical disjointness. Each intersecting model process must have its native StopFact and released reservation. Fixed account/read/login records have no workspace write authorization and are not queried as H process episodes.

## VALIDATION

- `git diff --check`: PASS before the final push.
- Native Windows cloud CI: [run 37125112897](https://github.com/taiyun668/gogoke/actions/runs/37125112897), `PENDING` overall at handoff, exact code SHA `c7dd46c2`. Its server job and focused native probes had completed successfully; full native library tests and later mutation gates had not settled.
- Desktop cloud CI: [run 37125112598](https://github.com/taiyun668/gogoke/actions/runs/37125112598), `PENDING` overall at handoff, exact code SHA `c7dd46c2`. Browser build/tests and dependency notices had completed successfully; clean Windows build/installer jobs had not settled.
- Prior SHA runs were dispatched per push and superseded; they are not evidence for current bytes.
- Real Owner product and CLI: `NOT_RUN`.

## FAILURES

No complete current-code-SHA workflow result yet. Do not treat queued, skipped, or cancelled checks as passing.

## INVARIANTS_CHECKED

F.1 stored instance identity, home and credentials are not rewritten by F.2. The original four worktree tables migrate to the exact expanded schema. No model-selected cwd, arbitrary path, remote fetch/push, forced worktree removal, fabricated StopFact, released UNKNOWN, or host override of Owner cap4 is added. The current H `LaunchEvidence` grants only the graph-resolved `worktree.path` as a workspace root; common Git metadata is not a workspace grant.

## RISKS

Shared product dispatch has not wired the new F.2 calls, and this seat cannot validate the real manual upgrade or Win11 UI. If shared H begins granting a mixed parent or any extra workspace write root, Controller must pass each sealed physical root and identity to F's cleanup gate; cleanup must remain denied for that launch shape until connected.

## DEVIATIONS_FROM_PLAN

None in F.2 scope. The server instances path did not need a change; native custody is the direct source for this milestone.

## OPEN_QUESTIONS

Controller must settle public request/receipt mapping, actual E.2 merge-grant closure, and H's future workspace root contract in shared files outside this branch's write ownership.

## RECOMMENDED_NEXT_ACTION

Root should continue the already-running code-SHA cloud checks and the new Task Result commit check, then let independent audit inspect F.2 and Controller integrate the explicitly listed shared calls. Preserve `NOT_RUN` for Owner verification until the real product exercises them.

## 参照

Read `origin/main:AGENTS.md`, `docs/model-routing.md`, Design37 PLAN/MANIFEST F.2, K-INSTANCE, K-WORKTREE and L0 fake contracts before editing; compared this repository's F.1 native instance/worktree and H LaunchEvidence implementation. Historical gogo-party provider-login/instance routing and NaveHQ worker worktrees were read for mechanism only; they do not provide this product's native stop facts. Research maps: `docs/research/reuse-blueprint.md`, `docs/research/upstream-reference-map.md`, and the parts/substrate teardown. Official Git references: [worktree](https://git-scm.com/docs/git-worktree), [merge](https://git-scm.com/docs/git-merge), [config](https://git-scm.com/docs/git-config). Detailed integration and evidence premises are in `SHARED_INTEGRATION.md`.
