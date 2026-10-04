# M2 real Win11 driver (development artifact)

`m2-win11.mjs` is an ordinary-view driver for one already installed candidate.
It imports the M1 `ActualProduct` launcher: installed image/resource hashes,
HKCU registration, bootstrap version, WebView2 page identity, endpoint ancestry,
`e2e` hard page locator, telemetry disabled, no `agent.act`, formal snapshots
and normal caption close remain the same. It never signs in, opens an auth
page, enters a credential, changes an installation, or runs against the formal
database. It has **not been run**; this file does not assert M2 acceptance.

The Controller prepares a new private JSON config in an ordinary Interactive
/ Limited task. It contains the M1 installed candidate fields (`installed`,
`version`, `sourceCommit`, `installedSha256`, `registryKey`, `pwsh`,
`evidenceDirectory`, `result`, `domainId`, `seatId`, `instanceId`,
`repositoryId`, `worktreeId`, and `observers`), plus `python`, `stateRoot`,
`testbedSource`, `templateId`, `childSeatId`, `childInstanceId`,
`takeoverQuestionId`, `takeoverPrompt`, `takeoverOption`, and `providerCases`.
The test repository must be the private `gogokeSeatTestbed`; the real lead
instance is `codexTestM1`; the main child uses an already admitted Codex
instance so its native turn completion can be read through A. `providerCases`
may contain any subset of Claude, OpenCode and Grok, including an empty array.
Each supplied row identifies the previously admitted seat/instance/worktree
plus its fixed version and SHA-256. An omitted provider is recorded
`NOT_RUN_NOT_CONFIGURED`; a supplied provider not already `LOGGED_IN` remains
`NOT_RUN_NOT_LOGGED_IN`.
Antigravity remains `NOT_RUN_OWNER_DECISION_PENDING` regardless of status.

The config's `result` and observer outputs must be fresh paths under a private
`evidenceDirectory`, outside the repository, installed candidate, state root
and testbed source. The mandatory `formal` observer compares `formal`,
`registeredFormal`, `formalData`, `formalRegistry` and `shortcuts`; `memory`
and `ledger` retain the M1 read-only measurement contract. No file here
contains a real local path, account name, credential or token.

The private fixture helper accepts `fixtureSetup.providerFixtureSelection`
`DEFER_ALL` to create only the authorized Codex lead and three history E/F
fixtures. It leaves the model-created child unused, preserves the existing
Codex cap of four, emits `providerCases: []`, and records each provider as
`NOT_RUN_DEFERRED_TO_CODEX_M2_FIRST`. Provider pins, settings, seats and
worktrees are not synthesized in that mode.

## Exact case path

1. Confirm pre-existing installed bytes, the real Codex login state, the
   lead's existing seat/worktree and a fresh marker absent from the source.
   Reserve, commit and open one H lead session through the original User pipe.
2. Ask the real lead Codex model to read its native state card, ask the
   template's one prespecified non-secret takeover question, consume the
   original C answer through `gogoke_takeover`, create a direct child from the
   existing template and dispatch it through `gogoke_seat`. The driver answers
   only that exact native question. It never sends a User-origin equivalent of
   a model-authorized create, dispatch, stop or merge.
3. The native tool's original H send receipt supplies a child session locator
   when the fixed CLI exposes it. A model text locator is rejected. The child's real `output-stream`
   must carry an original completed turn. The lead then invokes its own native
   `gogoke_seat stop`; the driver observes the child Idle fact. It stops the
   lead without releasing its admission and closes the product normally.
4. `m2-readback.py ... capture` refuses a nonempty WAL and opens the closed
   candidate with `mode=ro&immutable=1`. It binds the original A `item/tool/call`
   bytes, physical custody and RPC ID to F's registered worktree request,
   the H child reservation/send/stop/release and the E direct-child operation.
   It hashes the actual marker file and its local Git commit. Its worktree ID
   is the sole merge target for the next phase.
5. Restart the same installed candidate and resume the same lead session.
   Only the lead model invokes `gogoke_worktree merge` for the verified ID.
   A User graph query may observe its state, then H stops/releases the lead.
   Configured Codex history cases complete before any provider session. Each
   configured already logged-in provider is visited sequentially through its own
   existing H seat/worktree. It receives one actual User send with non-secret test content;
   original H input receipts and raw A protocol are exported for the existing
   golden-sample tool. No provider login or model substitution is performed.
6. Close normally, run the final immutable readback, then compare the
   original formal/memory/ledger protection snapshots. The final readback
   verifies original model call bytes, H RPC writes and send receipts, E
   takeover/child facts, F lifecycle/graph, actual marker hashes and Git merge
   ancestry/provenance. It exports raw frames, RPC commands, pending/unknown
   frame methods and provider session IDs privately. A script `FLOW_COMPLETE`
   state still requires independent evidence review and Owner acceptance.

Every User request ID and exact JSON wire frame must be saved **before** its
single invocation in `ActualProduct.operation` (shared `product-cdp.mjs`
integration owned by Controller). UNKNOWN, disconnect and timeout preserve
the original request; the driver never generates a replacement ID or silently
resends. Only read-only output, state-card and graph requests are polled.
On failure the original product is held for Controller disposition; no force
kill, implicit stop, credential reset or fresh login occurs.
The Controller's native tool integration must return the dispatch-created
logical `worktreeId` in that original H send ACK, with `seatId` and
`generation`, while preserving H's original receipt bytes. The driver
checks that ACK selector against the later immutable A/F/H facts. If the
installed candidate lacks this API, the case fails with its original output;
the model's final prose cannot supply a missing authoritative selector.

## Scope and references

The M2 main path exercises a subset of V00b, V06 and V11. The other M2
checks, including V03b, V04b, V08, V10, V12 and V13, require their own
real-case evidence; this driver does not mark them PASS. The separate V12
side-chat module owns its own source/side sessions and attaches its case IDs
only when explicitly configured; it must never stop or restart a live main-chain
parent. Its two User-authorized test worktrees need actual F create/register
receipts in this run. After normal close, `m2-readback.py ... side-worktrees`
returns both registered physical paths and Python `stat` identities, bound to
the original F rows. The side case stores that private artifact's basename and
SHA-256 and the two worktree records. Final readback compares the original
artifact, current F rows and another Python `stat`, and exports its own H/A
session IDs without promoting the side module's status to PASS. Neither a
fixture nor a model's self-report is a persistent fact.

References read: `tools/e2e/m1-win11.mjs`, `product-cdp.mjs`,
`m1-readback.py`; `docs/design/gogoke-37-plan-v1/PLAN.md` and `PLAN.json`;
GOGO PARTY's `packages/room/src/accounts.ts` and `packages/seat-runtime/`;
`docs/research/reuse-blueprint.md` and the source teardown; H's
`session_transport/ACP_ASYNC_REFERENCES.md`, the Codex native tool codec, and
the [official ACP prompt-turn contract](https://agentclientprotocol.com/protocol/v1/prompt-turn).
Room's process and seat precedent informed the sequence, while the native
A/C/E/F/H journals supply authority here. The provider ACP prompt response is
the end-turn fact; an intermediate `session/update` or write alone is not.
The existing golden-sample importer consumes this readback's original private
`frames` and `commands`, never a hand-written provider transcript.

After the original `final` readback, the runner invokes that existing importer
once per completed Claude, OpenCode and Grok session. It selects the actual H/F
session identity, fixed version and program SHA-256; both input file hashes
must equal the retained original readback. Outputs remain private and sanitized,
with `REVIEW_REQUIRED` and `acceptance=NOT_ASSESSED`. Missing or not-run sessions
cannot create a baseline. Codex's already recorded M1 baseline is separate;
this does not invent a helper digest or promote a CLI label from config alone.

## Cross-project history integration

Optional `historyBoundary` follows `m2-history-boundaries.md`: each case has
`projectA`, `projectB` and `sideBinding`, each with an explicit actual domain,
the authorized `gogokeSeatTestbed` repository, seat and original registered F
worktree. A and B use different domains on the same fixed instance; the third
side/review binding belongs to A's domain. These exclusive objects must already
be registered and must not overlap main, provider, rules or V12 objects.

The runner completes the earlier readers before these cross-domain sessions
enter the journal. It then owns normal-close, immutable `before-refusal`
readback, same-candidate restart, actual formal refusal operations, normal-close
and immutable `final` readback. Both require original H/A/D/F facts and unchanged
database bytes. No reader opens vendor histories or credentials. Effective
vendor memory/instruction provenance and the OwnerLead exact-history object
remain explicit NOT_RUN dependencies; these flow facts cannot close V04b/V10.
Without configured original test bindings, history records NOT_RUN, not PASS.

With `historyBoundary.peerRead=true`, the original marker and formal-refusal
flow is read back before opening two new peer-read sessions. Their normally
closed `peer-final` readback requires the original tool command and its native
access-denied reason for the exact previously verified test history object.
Missing objects, no tool execution or unqualified refusal stay NOT_RUN. This
separate result never promotes the old flow or overall V04b/V10 to acceptance.

The current script package has received only signed Node syntax and Python
parser checks. Native code builds remain cloud-only; actual installed Win11,
Smart App Control, browser and account outcomes are **NOT_RUN**.

## Provider boundary module integration

Preserve the existing `providerCase` behavior and every supplied
`journal.providerCases` result:
`m2-readback.py` binds their real User send and stopped H/A session for the
private provider golden source. After that loop and its normal-close golden readback, the runner calls
`runProviderBoundaryCases(product, {...config, providerBoundary:{cases}}, journal)`
to open separate H sessions on the same already logged-in provider rows.
The caller supplies each fixed row's `instanceId`, `seatId`, `worktreeId`,
`version`, and SHA-256; Claude additionally receives its `worktreeRoot` from
the prior normally closed F reader, not an Owner-typed path. Optional
`crossProject` and `reviewSource` are F/E binding preflights only. Preserve
the module's `providerBoundaryCases` beside the old `providerCases` rather
than replacing the golden source or treating a second case as replay of the
first request.

The existing boundary module requires exactly three provider rows. With a
provider subset, those boundary checks remain `NOT_RUN_REQUIRES_THREE_PROVIDER_BOUNDARY_CASES`.
After a configured three-row boundary flow's final ordinary caption close, the runner invokes signed Python
`m2-provider-readback.py STATE_ROOT NEW_PRIVATE_OUTPUT ORIGINAL_JOURNAL`
once with a fresh output path under the same private evidence directory.
The normal-close golden reader exports the configured provider F registrations;
the runner supplies their physical paths automatically and the boundary
reader checks them again against the original F identities. The old golden
reader runs before these distinct sessions, preserving its original
send/capture contract.
Record its basename and SHA-256 in the journal and retain it for independent
review. Its Claude direct case requires the real C/H/A answer-to-Result and
marker chain; `SOURCE_PRESENT_NATIVE_ASK_USER_BEHAVIOUR_NOT_RUN` and the
module's `FLOW_*` state are not a PASS. A pending original send or uncertain
answer may only be observed through read-only native output, without a new
send/answer request. V04b and V10 remain NOT_RUN until their documented
same-instance second project and same-seat side-to-formal source chains are
real and independently read back.
