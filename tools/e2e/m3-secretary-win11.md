# M3 V14 installed secretary read slice

`m3-secretary-win11.mjs` uses the existing `ActualProduct` installed WebView driver. It sends the same authenticated `gogoke_design37_user_operation` frame used by the product's secretary UI. The only frame in this slice is `secretary-configuration-read`; the runner records its exact request and reply, normally closes the actual candidate, reads its closed SQLite database with signed Python, restarts the same installed bytes, reads the original User model again, then closes normally. The Python reader verifies the global singleton, original USER/LONG seat and its four persisted selections without opening credential or conversation-content tables. It opens `state.sqlite` as `mode=ro&immutable=1` only after an exit-code-zero caption close, requires an empty or absent WAL, and hashes the database before and after reading.

The private JSON configuration follows `m2-seat-management-win11.mjs`: `installed`, `version`, `sourceCommit`, `installedSha256` for `gogoke.exe`, `gogoke-native-host.exe` and `resource-index.json`, `registryKey`, signed `pwsh` and `python`, `stateRoot`, `testbedSource`, an existing separate `evidenceDirectory`, a fresh `result` file within it, and `testerArmy: true`. Add `m3Secretary.expected` with either `{ "state": "UNSET" }` or a `DESIGNATED` state plus exact `seatId`, `incarnation`, `instanceId`, `model`, `effort` and `permissionTier` read from the already authorized test root. These expected values select a case; they do not establish the facts. The actual User reply and immutable database must independently agree. Paths and values belong only in private run configuration, never in the repository.

Run only when the Controller has already established that the installed candidate points to the intended isolated test state root and has settled every original live H session. This entry refuses to close if its secretary read reports live or unresolved H custody. It creates no H session, so it has no stop/release request to issue. On an unknown operation, failed read, or unconfirmed close, it saves the original evidence and retains the product for Controller custody instead of killing or retrying it.

Example invocation from the repository root, using signed runtimes and a private config path:

```powershell
node tools/e2e/m3-secretary-win11.mjs <private-config.json>
```

Coverage is the original User configuration read, producer-matched closed SQLite read, and cold User read. `UNSET` and `DESIGNATED` are separate observations of whatever the actual root already holds; this script does not change one into the other. `secretary-designate`, configuration changes/restoration, non-User caller refusal, live global conversation, scheduling, delivery scope, search, and full V14 remain `NOT_RUN`. A malformed User request would not prove non-User refusal. The existing `secretary-designate` producer writes a global singleton and has no User un-designate operation; `secretary-configure` requires four nonempty selections and has no User unconfigure operation. These facts prevent a safe unset-to-configured-and-restored cycle in an existing Owner root. A true non-User refusal needs the original non-User H/model origin, which this read-only slice does not manufacture.

References checked: `product-cdp.mjs`, `m2-seat-management-win11.mjs` and its immutable reader; native `v37_seat.rs` secretary read/configure dispatch and `seat/secretary.rs` global designation/source-bound settings; product `services/tauri.ts` secretary User frame. The plan's R-SEC/E.3/V14 requires the full lifecycle and scope checks; this package reports only the three listed observations. No acceptance is asserted.

Syntax-only checks, without launching the product:

```powershell
node --check tools/e2e/m3-secretary-win11.mjs
python -c "import ast,pathlib; ast.parse(pathlib.Path('tools/e2e/m3-secretary-readback.py').read_text(encoding='utf-8'))"
```
