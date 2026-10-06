# M2 installed seat-management entry

Run the standalone entry with the signed Node runtime and one private JSON config:

```powershell
node tools/e2e/m2-seat-management-win11.mjs PRIVATE_CONFIG.json
```

Replace `PRIVATE_CONFIG.json` with the path to the private run config.

The config pins the already installed candidate (`installed`, `version`,
`sourceCommit`, `installedSha256`, `registryKey`), signed `pwsh` and `python`,
`stateRoot`, the original `domainId`/`instanceId`/`seatId`/`childSeatId`, and an
isolated `evidenceDirectory` plus a fresh `result` path. `repositoryId` must be
`gogokeSeatTestbed`; `testbedSource` is the actual registered testbed checkout.
The evidence directory must exist outside the installed candidate, testbed,
source checkout, and candidate state root. `providerCases` lists the existing
M2 provider seat IDs solely to keep the two new targets disjoint; use `[]` when
there are none. This entry starts no provider process. No credentials belong
in this file.

`seatManagement` follows the private fixture described in
[`m2-seat-management.md`](m2-seat-management.md):
`lifecycleOwnership: "EXCLUSIVE_M2_SEAT_MANAGEMENT"`, exactly two distinct real
project domains with the same existing `templateId`, distinct unused USER
`seatId` values, a non-secret setting/value, and optional `leadSeat` facts. The
entry supplies the callback that normally closes the installed app, invokes
`m2-seat-management-readback.py` with the signed Python runtime, verifies the
proof hash and candidate binding, then relaunches the same installed bytes for
the next phase. Baseline proves both target seats were absent; final readback
checks the original native receipts and persisted rows without writing the
database.

No original-H model callback is injected by this independent entry. It does not
start or resume a session or ask a model for a result, so `MODEL_DENIAL` remains
explicitly `NOT_RUN`. Omitting `leadSeat` likewise leaves
`USER_LEAD_TUNE_AND_RECLAIM` as `NOT_RUN_NOT_CONFIGURED`. These absences cannot
produce V06 acceptance; the journal and immutable proof both keep
`acceptance: false`.

Implementation follows the installed-candidate custody, normal-close, and
restart pattern in `m2-extra-win11.mjs` and `m2-win11.mjs`; it delegates the
K-SEAT facts to the existing `m2-seat-management.mjs` and immutable reader.
The original H/User pipe and producer-bound fixture contract are documented in
`m2-seat-management.md`. This entry adds no product behavior and does not build,
sign, install, launch provider CLIs, or run locally during the frozen 0.1.35
candidate hold. Syntax is checked with signed Node; installed-candidate E2E and
cloud CI are separate Controller actions. This evidence package is not V06
acceptance.
