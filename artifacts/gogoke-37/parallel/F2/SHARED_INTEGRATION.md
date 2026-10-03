# F.2 shared integration handoff

Source branch: `codex/gogoke-37-m2-f2`; base `546d4e9d573d90ecbfc5be6250aabbed6afbe45f`. F owns only the native `instance/` and `worktree/` modules. This note does not grant product acceptance.

## Existing native sources

- `instance::known_new_version(driver_id, registered_version)` compares the persisted version to the **running product's pinned catalog**. It returns only a known newer version, never a network claim or auto-upgrade. The shared `K-INSTANCE/version-and-new-version` read can include this optional `newVersion` while preserving the stored `version` and digest.
- Existing `instance::repin_program` verifies the manually installed CLI through native catalog identity, retains the original home and credentials, and sets login state to `UNKNOWN`. No capability assertion is carried over from the prior digest.
- After an exact repin, the shared product flow must use the new pinned CLI to read account state, open a properly bound native session, call its existing `K-SESSION/capability-probe`, and pass the **original applied request ID** to `instance::read_current_capability_reprobe`. This reader accepts only the durable original receipt bound to the current instance pin, H claim, generation, process custody and loaded feature flags. A missing receipt is `None`, not successful reprobe. The capability receipt explicitly keeps model behaviour `NOT_RUN`.

## Worktree calls to connect

- Existing User `K-WORKTREE/create` can keep using `worktree::create_worktree`; `worktree::create_mixed_worktree` is for a trusted mixed-space identity derived by the host's project graph, never from a model path. Both create physically distinct, host-generated directories under the appropriate `single` or `mixed` parent. Existing single rows remain readable after the exact four-table F.1 schema is extended.
- `worktree::graph_query` projects registered repository, project, worktree, seat, instance, baseline and merge result. `register` must replay the already registered native creation without another Git command; Controller owns the public receipt mapping and revision sequence.
- `worktree::merge_worktree` accepts an exact `K-WORKTREE/merge` request and a mandatory closure that checks the **current E.2 merge grant and authenticated caller seat**, returning the bound turn ID. Never wire an unconditional grant. It checks stopped and released work, clean source and linked trees, records INTENT, performs only local `git merge --no-ff --no-commit` then a provenance-trailered commit, and records APPLIED or MERGE_UNKNOWN. It never fetches, pushes, reads a credential helper, or retries an uncertain merge.
- `worktree::cleanup_worktree` requires the exact original native StopFact for every process episode of the tree's seat incarnation and zero active reservations. It records INTENT before plain `git worktree remove` (without `--force`), verifies physical absence, then settles APPLIED; any uncertain effect remains CLEANUP_UNKNOWN. It never removes a caller-selected path.
- `worktree::resolve_for_launch` refuses non-REGISTERED F.2 lifecycle states. A mixed space uses separate linked trees even when one repository serves several projects.

## Validation scope

Native implementation, Git effects, merged main-tree bytes, actual vendor capability reprobe, Windows 11 product UI and the Owner's real directory are separate evidence. This branch changes no shared dispatch, Tauri UI, CI workflow, fixed CLI, Owner concurrency cap, installed product or formal data.

## 参照

Read the earlier gogo-party `packages/room/src/accounts.ts` provider-login and instance routes, NaveHQ's per-worker worktree/runtime-profile history, and the available LoomOS history; those paths do not provide this product's exact native StopFact and E.2 permission binding. Read `docs/research/reuse-blueprint.md`, `docs/research/upstream-reference-map.md`, and the parts/substrate source teardowns. Reused this repository's verified `worktree::git` native custody, physical pointer identity, request intent and `instance::repin_program` instead of introducing another runner or version source. Git's official [worktree documentation](https://git-scm.com/docs/git-worktree) says ordinary `remove` refuses unclean trees; its [merge documentation](https://git-scm.com/docs/git-merge) explains why `--no-ff` with `--no-commit` prevents an implicit fast-forward update. The [Git configuration reference](https://git-scm.com/docs/git-config) documents hooks and credential helpers; F's existing native Git launcher disables hooks, system config and terminal prompts and invokes no remote operation.
