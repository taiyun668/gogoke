# Installed V06 remaining axes

Run `m2-v06-remaining-ordinary.ps1` in the existing ordinary Interactive/Limited
task after Root has reviewed one fresh private fixture. The wrapper checks the
signed Node bytes and keeps TEMP/TMP on the private D: evidence area. It must
start with no live gogoke or native-host process. The entry starts the installed
candidate once, uses its actual User bridge, closes by the product caption, then
reads the closed SQLite database with `mode=ro&immutable=1`. It never logs in or
reads credential contents.

The config has the existing `product-cdp.mjs` installed candidate fields,
`observerTools` and exactly three `observers` from the original V06 fixture.
Pin the installed exe, native host and resource index hashes to the candidate
actually installed at dispatch. Each observer's script and companion hashes
must match the original source, and its `{output}` argument must be unique.
`signedNodeSha256` pins the Node executable passed to the wrapper. Use fresh
`evidenceDirectory` and its exact `result.json` path outside the state root.
The additional `v06Remaining` block is:

```json
{
  "lifecycleOwnership": "EXCLUSIVE_M2_V06_REMAINING",
  "domainId": "<existing authorized test domain>",
  "instanceId": "<same logged-in fixed Codex instance>",
  "templateId": "<existing exact User template>",
  "parentSeatId": "<fresh disjoint User ID>",
  "childSeatId": "<fresh disjoint Lead ID>",
  "parentTreeId": "<fresh disjoint F ID>",
  "childTreeId": "<fresh disjoint F ID>",
  "model": "<template model>",
  "effort": "<template reasoning effort>",
  "permissionTier": "<template permission tier>",
  "maxConcurrent": 1,
  "takeover": {
    "questionId": "m2PrivateScope",
    "prompt": "Which scope is authorized for this M2 product test?",
    "option": "Private testbed only"
  },
  "priorV06": {
    "path": "<original candidate49 closed V06 proof path>",
    "sha256": "<exact original proof SHA-256>"
  }
}
```

`priorV06` is optional and remains labeled historical candidate49 coverage.
The runner performs the existing non-secret USER takeover and sends one new
parent model child-create turn. The child LEAD
`thread/start` does not register native dynamic tools under the current
orchestration-scope gate, so model-origin bounds denial is `NOT_RUN`; the runner
does not send a prompt that asks an unavailable tool to run. The prior
second-child capacity denial is not dispatched again. It creates and registers
two fresh F trees, binds the new User seat to the existing instance, and answers
exact non-secret native C cards. After a real child H open it reads BUSY,
requests the BUSY reclaim refusal, then stops and releases that same H before
User reclaim. `changeInstanceId` may name a separate already registered
logical instance to exercise BUSY change-instance; otherwise that axis is
`NOT_RUN`. SHORT-to-LONG, independent H admission capacity and host-choice axes
are `NOT_RUN` without separate real sources.

The closed reader correlates the original H sends, registered A tool calls,
H-written RPC replies, E writes, Busy state cards, StopFacts and physical
custody. For each completed A turn it joins the raw source row to the normalized
event by `resolved_event_id` and verifies `_meta.rawSourceCursor`; the normalized
ledger ordinal and global ledger cursor remain separate values. A live
timeout or UNKNOWN preserves the original request and product custody; neither
is replayed. The final state is a V06 slice for Controller review, never full
V06 or milestone acceptance.

```powershell
& $ReviewedPwsh -NoProfile -NonInteractive -File tools/e2e/m2-v06-remaining-ordinary.ps1 `
  -Config $FreshPrivateConfig -SignedNode $PinnedSignedNode
```
