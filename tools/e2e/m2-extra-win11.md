# Independent M2 V08 entry

`m2-extra-win11.mjs` runs the existing installed-product V08 rules module without repeating the M2 lead, child dispatch, marker merge, or history flow. It creates a fresh M2 journal using the existing `gogoke.37.m2-win11-e2e.v1` schema so `m2-rules.mjs` and `m2-rules-readback.py` retain their original wire receipts and close/readback checks.

The private JSON uses the normal installed candidate fields, fresh `result`, the three existing read-only observers, and:

```js
rules: {
  lifecycleOwnership: 'EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER',
  policyOwnership: 'EXCLUSIVE_V08_POLICY_DOMAIN',
  submitter: { seatId, instanceId, worktreeId },
  reviewer: { seatId, instanceId, worktreeId },
  // Optional actual NativeUser-created B fixture, existing before baseline:
  foreignProject: { domainId, gateId, ownerGate: { rawFrame, rawReceipt } },
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

Optional B is created through the original NativeUser Owner APIs by Root before this entry, without another CLI/account login. Its domain must differ from A and its gate ID must be absent from A. `ownerGate` preserves the actual `policy-gate` input frame and APPLIED native receipt as strings. The entry saves the exact configuration in the journal before the normal-close baseline; the reader binds those bytes to B's real READY gate and persisted Owner event, then requires all seven B policy tables and original bytes unchanged at final. B's registered objects remain in place. No B setup, domain selection or caller privilege is granted to the model. Without this configuration CROSS_PROJECT stays NOT_RUN.

The rules module always performs the parameter FORGED_SENDER case through one actual submitter call against the SUBMITTED pass gate. With B configured it also performs one actual cross-project target call from A. Both require original H/A DENIED, success=false, a unique typed RPC/tool call, tool and turn completion, and no policy event. Parameter caller impersonation does not claim outer-envelope replacement coverage. SUBORDINATE_OWNER and STALL_CHAIN remain NOT_RUN for their absent original reachable operation/producer chain. Root records the installed ProductSourceCommit separately from loaded tool DriverSHA; these source changes are not installed candidate acceptance.

The runner records V12 as `NOT_RUN_ORIGINAL_M2_SIDE_WORKTREE_READBACK_REQUIRED`. The current original `m2-readback.py side-worktrees` path also verifies the M2 main child dispatch, stopped/released child, and committed marker worktree before exporting physical roots. This entry does not bypass those checks, synthesize paths, or rerun the main flow. No D session or side file operation is started.

On the V08 path, fresh formal/memory/ledger snapshots bracket the run; the formal observer must compare `formal`, `registeredFormal`, `formalData`, `formalRegistry`, and `shortcuts`. The runner normally closes before each rules readback, preserves original source errors, and stops without replaying an uncertain request. Final flow labels remain non-acceptance and require independent evidence review.

Each Host checkpoint first uses actual live output-stream revision reads and original User H stop-only for both sources and the current busy/automatic target. `stopRulesSession` returns the original APPLIED stop receipt, updates that session and retains its claim. Only after genuine StopFacts exist does the checkpoint callback close for immutable readback and relaunch. The readback verifies original read/stop bytes, CAS revisions, physical custody and STOPPED claims; it never invents StopFacts from caption close. The target releases after checkpoint; sources resume their retained claims, and final sources still stop/release before final close. Automatic targets must have actual E generation, live H output/native send ACK and CLI completion before any stop; unavailable binding remains NOT_RUN with original failure custody preserved. No automatic failure cleanup or profile mutation is added. Instance capacity numbers alone cannot establish cold-home launch readiness.

No installed-product execution, model request, login, native build, or acceptance is implied by source inspection or syntax validation.
