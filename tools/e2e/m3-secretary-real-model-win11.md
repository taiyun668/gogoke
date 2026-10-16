# Secretary real model V14 slice (Windows)

`m3-secretary-real-model-win11.mjs` exercises one already configured, resident Secretary through the installed product's authenticated USER ingress. It does not designate a seat, log in, change credentials, or claim full V14 acceptance. Cross-project and schedule delivery remain `NOT_RUN`.

Use a private config outside the repository. It has the `ActualProduct` installed-candidate fields (`installed`, `installedSha256`, `version`, `sourceCommit`, `registryKey`, `pwsh`, `python`, `stateRoot`, `evidenceDirectory`, `result`, `testerArmy: true`, `domainId: "global"`) and the three `formal`, `memory`, `ledger` observers from the M2 provider config. Each observer has `runtime`, `args` (substitute `{output}`), and `equalFields`. The fields must total exactly 13. Add `m3SecretaryRealModel` with the current `seatId`, `incarnation`, `instanceId`, `model`, `effort`, `permissionTier`, and `globalParallelCap: "4"`. The evidence directory must exist and the result and observer outputs must not exist. Use a fresh directory for each run.

Run with the signed Node executable for the candidate environment:

```powershell
& $SignedNode 'tools/e2e/m3-secretary-real-model-win11.mjs' $PrivateConfig
```

The runner pins installed bytes through `ActualProduct`, uses the real tester-army WebView bridge with telemetry disabled, reads the configured Secretary through `secretary-configuration-read`, and requires a live `COMMITTED`, idle, global H on the exact seat and instance. It sends one fresh, non-secret marker question through `K-SESSION/send`. Since the preflight turn is `IDLE`, this is a **new turn**; `steered: false` is an observed classification, not steer coverage. The original request is persisted before dispatch. An `UNKNOWN` is observed only through `output-stream` for that request ID. No second send is issued. An unresolved request leaves the product and evidence for Controller disposition.

On success, the runner observes the original H `turn/start` receipt, then the same turn's `turn/completed` and model answer in normalized A. It clicks the actual window caption to close and requires shell and native exit zero. The host's resident Secretary close path performs the H stop. The immutable closed-DB reader binds the original USER send bytes, start receipt, raw producer completion, claim, process episode, process custody, stop request, and StopFact. The formal, memory, and ledger observers must show all 13 protected fields unchanged. The journal state `REAL_MODEL_SLICE_READBACK_COMPLETE_REQUIRES_REVIEW` is evidence for review, not acceptance.

Keep the existing `m3-secretary-win11.mjs` read-only slice separate. It requires a stopped Secretary H and cannot serve this live model run.
