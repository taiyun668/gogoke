# Prepare one private M2 F stop-worktree fixture

This is a standalone preparation entry for `m2-stop-worktree-win11.mjs`. Run it with signed Node only after the existing candidate, original logged-in Codex instance, private `gogokeSeatTestbed` source registration, and an eligible template are already available:

```powershell
node tools/e2e/m2-stop-worktree-prepare.mjs PRIVATE_PREP_CONFIG.json
```

The private JSON config uses the installed candidate fields consumed by `product-cdp.mjs`: `installed`, `version`, `sourceCommit`, `installedSha256` (pins for `gogoke.exe`, `gogoke-native-host.exe`, and `resource-index.json`), `registryKey`, signed `pwsh` and `python`, `stateRoot`, `testbedSource`, `evidenceDirectory`, and a fresh `result` journal path directly inside that evidence directory. It also requires the actual `domainId`, an existing already logged-in `instanceId`, `repositoryId: "gogokeSeatTestbed"`, and:

```json
"stopWorktreePrep": {
  "lifecycleOwnership": "EXCLUSIVE_M2_STOP_WORKTREE_PREP",
  "templateId": "<existing eligible template id>"
}
```

Use private, real values; do not put credentials or account content in the config. The evidence directory must be new/private and outside the installed candidate, repository source, state root, and this repository. The result journal path must not exist. The driver does not read an existing seat card. It generates new UUID-based seat and worktree IDs; it first confirms those exact IDs were absent in an immutable baseline. It then invokes original `K-SEAT create-from-template`, `bind-instance`, and `state-card`, followed by original User `K-WORKTREE create`, `register`, and `graph-query`, through the installed `__TAURI_INTERNALS__` bridge. It starts no session and sends no model input. The new User seat must remain IDLE and be bound to the configured existing logged-in instance; the graph must report exactly one registered SINGLE worktree for it.

The driver normally closes the original candidate before each Python SQLite `mode=ro&immutable=1` readback. The reader binds the proof to the latest exact candidate launch and exit-0 normal close, installed byte pins, state-root identity, evidence path, registered testbed source and real Git executable pin. It compares DB/WAL/SHM bytes around each observation and stores only receipt hashes and sanitized receipt identities, never template settings or credentials. It writes `m2-stop-worktree-prepare-baseline.json` and `m2-stop-worktree-prepare-final.json` alongside the journal. Every artifact has `acceptance: false`.

The native receipt identifies its schema, family, operation, request and target; it does not echo `domainId`. Domain binding comes from the original request bytes and the closed ledger/E/F records. Seat revision is the outer receipt field, while generation is in its result. The preparer checks those actual native fields rather than requiring absent echoes.

After the final proof, the journal's `fixture.stopWorktree` contains the generated `seatId` and `worktreeId`. Only after reviewing the private proof, carry those generated IDs into a fresh `m2-stop-worktree-win11.mjs` config as `stopWorktree.seatId` and `stopWorktree.worktreeId`, with `lifecycleOwnership: "EXCLUSIVE_M2_STOP_WORKTREE"`. Do not rerun this preparer or reuse its config after any mutation or failure. Failure retains the original request/journal and does not replay it; preserve and resolve the exact remaining state before any separately authorized follow-up.

This package prepares one fixture only. It does not run the stop denial/cleanup case, residual-child census, restart case, or any Owner-machine/native/model/authentication step. Those remain `NOT_RUN`; preparation is not M2/V07 acceptance. The module has no import-time runner side effect. Development validation is signed Node syntax and repository hygiene, not a product run.
