# Independent M2 V08 entry

`m2-extra-win11.mjs` runs the existing installed-product V08 rules module without repeating the M2 lead, child dispatch, marker merge, or history flow. It creates a fresh M2 journal using the existing `gogoke.37.m2-win11-e2e.v1` schema so `m2-rules.mjs` and `m2-rules-readback.py` retain their original wire receipts and close/readback checks.

The private JSON uses the normal installed candidate fields, fresh `result`, the three existing read-only observers, and:

```js
rules: {
  lifecycleOwnership: 'EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER',
  policyOwnership: 'EXCLUSIVE_V08_POLICY_DOMAIN',
  submitter: { seatId, instanceId, worktreeId },
  reviewer: { seatId, instanceId, worktreeId },
  // Optional Host delivery cases:
  host: {
    lifecycleOwnership: 'EXCLUSIVE_V08_HOST_RECIPIENTS',
    destination: { seatId, instanceId, worktreeId },
    alternateDestination: { seatId, instanceId, worktreeId },
    busyQuestion: { questionId, optionLabel },
  },
}
```

The selected E seats must already exist, be idle, and bind to logged-in Codex instances. Their worktree IDs are fresh inputs; the runner creates and registers the corresponding F worktrees through the original User operations. The Owner-initialized policy head must already exist in the configured domain. This entry rejects `policyInitialization`; it reads the effective head only through the normal-close `before` reader and never invents a revision. If Host cases are configured, they need two more distinct seats and worktree IDs.

The runner records V12 as `NOT_RUN_ORIGINAL_M2_SIDE_WORKTREE_READBACK_REQUIRED`. The current original `m2-readback.py side-worktrees` path also verifies the M2 main child dispatch, stopped/released child, and committed marker worktree before exporting physical roots. This entry does not bypass those checks, synthesize paths, or rerun the main flow. No D session or side file operation is started.

On the V08 path, fresh formal/memory/ledger snapshots bracket the run; the formal observer must compare `formal`, `registeredFormal`, `formalData`, `formalRegistry`, and `shortcuts`. The runner normally closes before each rules readback, preserves original source errors, and stops without replaying an uncertain request. Final flow labels remain non-acceptance and require independent evidence review.

No installed-product execution, model request, login, native build, or acceptance is implied by source inspection or syntax validation.
