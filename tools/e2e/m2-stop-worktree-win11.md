# M2 installed F stop-gate entry

Run signed Node with `node tools/e2e/m2-stop-worktree-win11.mjs PRIVATE_CONFIG.json`.
The private config pins the already installed candidate (`installed`, `version`, `sourceCommit`, `installedSha256`, `registryKey`), signed `pwsh`/`python`, original candidate `stateRoot`, `domainId`, `instanceId`, main `seatId`, registered `testbedSource`, and `repositoryId: "gogokeSeatTestbed"`. Use a fresh `result` directly inside an existing evidence directory outside product, testbed, source and state roots. Credentials never belong in config or evidence.

`stopWorktree` supplies `lifecycleOwnership: "EXCLUSIVE_M2_STOP_WORKTREE"`, an exclusively owned registered SINGLE `worktreeId`, and its distinct IDLE User `seatId`. The entry delegates to the existing original User `runStopWorktreeCase`: H open, F cleanup denied, real H stop/release, same-tree cleanup applied. It injects normal-close, signed immutable Python readback and same-candidate restart for `stopped` and `final`. It verifies proof bytes and exact candidate/root/case bindings; existing module verifies original H/F facts. Failure keeps the journal and calls `preserveFailure`; do not replay the same config after writes.

This entry starts a fixed H CLI process through the product but sends no model prompt, performs no authentication and installs nothing. Residual descendant census and Owner-host restart remain NOT_RUN. `FLOW_COMPLETE_REVIEW_REQUIRED` is evidence for this case, never V07-wide acceptance; `acceptance` remains false.

Reference: the existing installed `m2-seat-management-win11.mjs` normal-close/readback/restart pattern and `product-cdp.mjs` real ingress; the original F producer and exact H StopFact chain are referenced in `m2-stop-worktree.md`. No new substitute bridge, direct database write or artificial stop record is introduced. Signed Node syntax is a development check; actual cloud and installed E2E results must be recorded separately.
