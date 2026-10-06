# M2 F cleanup stop-gate case

This exported case adds one original-User flow to the existing `ActualProduct` driver. Importing the module has no side effects. The configured testbed must provide one exclusively owned, already registered `SINGLE` F worktree and a distinct bound `IDLE` User seat on an already logged-in instance. The case opens H against that exact F tree through the installed `window.__TAURI_INTERNALS__` bridge, requires the original User cleanup operation to return `DENIED`, stops H and records its returned StopFact, releases the claim, performs a normal-close immutable baseline readback, then cleans the same F tree through the original User API and performs the final readback.

An integrating M2 runner can import `runStopWorktreeCase` and call `runStopWorktreeCase(product, config, journal)` after the existing M2 setup, passing:

```js
config.stopWorktree = {
  lifecycleOwnership: 'EXCLUSIVE_M2_STOP_WORKTREE',
  worktreeId: '<precreated registered F worktree id>',
  seatId: '<precreated idle seat id>',
  normalCloseReadbackRestart: async phase => {
    // Normally close the actual candidate; update journal.closes, then run the
    // signed reader below into a fresh file in config.evidenceDirectory.
    // Return { file: basename, sha256: SHA256(file bytes) }, then relaunch the
    // same candidate/root and refresh journal.launches/currentEndpoint.
  },
};
```

The callback runs for `stopped` and `final`. It must use the same candidate process lineage and `config.stateRoot`, record actual launch and normal-close facts before readback, and return the proof file and its actual SHA-256. Invoke the signed Python reader after close and before relaunch:

```text
python m2-stop-worktree-readback.py STATE_ROOT OUTPUT JOURNAL stopped|final
```

`OUTPUT` must be a new basename in the evidence directory. The runner independently hashes and parses the returned file and binds it to the case, phase, domain, exact F/seat/instance, reader hash, candidate commit/version/set/generation/installed-byte pins, root/database physical identity, and normal-close PID/exit status. The reader opens `state.sqlite` read-only and immutable, checks DB/WAL/SHM hashes before and after, reads the original H operation bytes and StopFact custody chain, and checks the original F registration, cleanup request hash/APPLIED lifecycle operation, and actual directory presence or absence.

This is a prepared real-product E2E export, not a substitute host or a self-acceptance gate. It does not send model input. It reports acceptance false. Residual child-process cleanup and host restart stay `NOT_RUN`; no V07-wide claim follows. Configure private testbed IDs and the normal-close/restart callback only in the private runner; do not put credentials or authorization content in the journal.

## Producer basis

The mechanism follows the original User `K-WORKTREE/cleanup` path in `worktree/f2.rs`: the stop gate checks overlapping non-released H claims and original H process episodes before writing F cleanup intent; after a valid StopFact it writes `CLEANUP` intent, removes the registered F worktree, verifies the path is absent, and records lifecycle `CLEANED` / operation `APPLIED`. The original H reserve/commit/open/stop/release operations and process custody StopFact are produced through the H operations recorded in `gogoke_v37_h_operation` and related ledger, claim, episode, and custody tables.

The direct harness precedent is `tools/e2e/product-cdp.mjs` for the installed WebView and original User ingress, with the H lifecycle shape in `tools/e2e/m2-extra-win11.mjs`. Immutable close-bound readback follows `tools/e2e/m2-history-boundaries-readback.py`. Historical GOGO PARTY seat-runtime close artifacts were reviewed read-only; no equivalent direct installed-Tauri F cleanup harness was found in NaveHQ or LoomOS during this bounded research.
