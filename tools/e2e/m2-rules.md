# M2 V08 installed-product rules and Host cases

TASK / RESULT / CURRENT_STATE: real E2E preparation from `349ac3983720d780f86db32ca61c1c9ae94865bd`. Actual app/model/CLI/native/cloud/Win11 execution and acceptance are **NOT_RUN**. No product API, DB fixture, unit/mutation framework, hidden probe or altered CLI is added.
FILES_CHANGED: only `m2-rules.mjs`, `m2-rules-readback.py`, this reference; Root owns the shared runner.

## Runner context

Import `runRulesCase(product, config, journal)` with the existing ActualProduct and `gogoke.37.m2-win11-e2e.v1` journal. Importing has no effects. Insert after main lead merge/stop/release and runSideChat has released its own admissions, before provider cases. Every V08 lifecycle and the policy domain must belong exclusively to this testcase; no unrelated process may remain under the normal-close checkpoint.

Only authorized gogokeSeatTestbed worktrees and already qualified/logged-in Codex instances are used. The runner creates/registers missing test worktrees through real E/F User APIs. Four distinct seats/worktrees are needed: source, reviewer, destination, alternate destination. Targets open sequentially; at most three native processes coexist. Actual Owner project cap and host capacity still govern H admission; this module cannot supply a default or change either.

Base config.rules fields remain lifecycleOwnership=`EXCLUSIVE_V08_SUBMITTER_AND_REVIEWER`, policyOwnership=`EXCLUSIVE_V08_POLICY_DOMAIN`, submitterSession, reviewerSession and baselineReadback `{file,sha256}`. Each session is the actual mutable object in journal.sessions with `{id,seatId,instanceId,worktreeId,generation,revision,cursor,events,turns,threadId}`. Add:

```js
host: {
  lifecycleOwnership: 'EXCLUSIVE_V08_HOST_RECIPIENTS',
  destination: { seatId, instanceId, worktreeId },
  alternateDestination: { seatId, instanceId, worktreeId },
  busyQuestion: { questionId, optionLabel }, // explicit nonsecret test values
},
hostCheckpoint, openHostSession, resumeRulesSession,
stopRulesSession, releaseStoppedRulesSession,
```

Callbacks are injected by the runner, not serialized in JSON. host may be omitted; then delivery remains NOT_RUN and the existing business cases still run.

The shared `m2-win11.mjs` runner now injects this context from private JSON `rules.submitter` and `rules.reviewer` selections (`seatId`, `instanceId`, `worktreeId`) plus the optional host selections. It refuses overlap with the main or side-chat seats/worktrees, verifies actual logged-in Codex instances, creates/registers fresh case-owned testbed worktrees, and takes the normal-close baseline before opening the sources. Its five callbacks use original H receipts and the immutable checkpoint's STOPPED revisions; no cached revision is guessed. It preserves old event/turn history while resetting only the output cursor for a new physical generation. Baseline/checkpoint are explicitly not results; final requires the rules reader's direct evidence. Without rules configuration the journal records V08 NOT_RUN, not a pass. Actual app/model execution remains NOT_RUN at this SOURCE stage.

| Callback | Required actual behavior |
| --- | --- |
| hostCheckpoint() | product.closeNormally(); signed Python rules reader phase checkpoint with actual state root/original saved journal/fresh private output; save artifact metadata; product.launch(); return `{file:basename,sha256}`. No implicit session resume/open. checkpoint is a snapshot, so directCaseEvidence is false. |
| openHostSession(selection) | New normal User H admission-reserve/commit/open for this selected case-owned seat and registered worktree. Read current native card/revision; return actual WORK Codex object in journal.sessions. No invented generation or mutation retry. |
| resumeRulesSession(session) | Normally resume this exact stopped source/reviewer; update the same object from original H receipt generation/thread/revision; reset cursor for the new physical generation. No new logical source or input resend. |
| stopRulesSession(session) | Stop only this actual recipient; release only after its physical StopFact. Preserve original frames/receipts, update same object, no force kill. |
| releaseStoppedRulesSession(session) | After checkpoint normally stopped the unanswered busy target, read/reconcile its actual STOPPED claim and issue its one admission-release. Do not resume that unanswered vendor turn or issue another stop/input. |

Before opening sources: normal close -> `m2-rules-readback.py STATE_ROOT FRESH_PRIVATE_OUTPUT M2_JOURNAL before` -> normal launch -> real source/reviewer open -> module. Existing native Owner-initialized policy head is required. Inject the original baseline basename/hash; private configured revision/stage/path cannot replace it. before returns BASELINE_ONLY_NOT_A_V08_RESULT; the main reader helper requiring directCaseEvidence=true is inappropriate for this baseline. Module and reader SHA256 must agree with baseline/checkpoints/final; Root also registers both loaded hashes in journal.driverBytes.

After FLOW_COMPLETE_DIRECT_LEDGER_READBACK_REQUIRED: stop/release the two sources -> normal close -> same reader final. Flow state is not evidence. Reader returns IMPLEMENTED_CASES_HAVE_DIRECT_EVIDENCE_V08_INCOMPLETE, never whole V08/M2 acceptance.

## Preserved business and independent Host cases

All 12 existing Model calls remain: submit/reject reason/resubmit/cap state and refusal, gate bypass, actual expired REVIEW grant, wrong reviewer, empty reject reason, PASS and legal ADVANCED stage. The User UNSUPPORTED control stays separate from native Model negative evidence. Raw mutating bytes are saved before one real invoke. Readonly STALE observations get fresh IDs/current revisions; failures preserve original error/request, never resend. No question card is auto-answered.

Four independent cap-1 Host gates follow the legal transition, from the actual new stage. Each cause is the existing real native source gate-submit/reviewer gate-decide, with original ask/H stdin/A tool/native reply/CLI completion. There is no User gate decision substitute, hand-built caller/turn, fabricated sealed proof or E.3 REGISTERED coordinator row.

| Kind | Original precondition and outcome |
| --- | --- |
| DELIVERED | Destination has no retained process. Actual cap produces E INTENT/C HOST_RULE PENDING with empty turn/generation. Closed checkpoint supplies actual IDs and proves no delivery/H command. New normal User H recipient admission creates a retained idle WORK Codex; native Host safe point sends ordinary H input. C/E DELIVERED must bind original H createdTurn receipt, custody-bound RPC turn/start, typed raw A ACK and separate CLI completion. Extra readonly views preserve one original logical send/command/turn. |
| BUSY_QUEUED | Normal WORK destination performs one real nonsecret request_user_input. Two original K-QCARD recover receipts before/after cap show the same OPEN/availableForAnswer turn and custody. Original A raised-card descriptor and C pending checkpoint prove busy queue. Normal close supplies physical stop; User cancel cleans up the original notice, then original stopped session is released. No busy-to-idle delivery is claimed. |
| ROUTE_CHANGED | Separate cap queues. After original checkpoint, actual Owner route CAS selects alternate same-domain target before either opens. Real new H opens of old/new targets occur sequentially. Original C stays PENDING, E INTENT stays unchanged, and no second C identity/H input/command/turn may appear for the cause. This is route change; existing API exposes no route deletion. |
| CANCELLED | Separate cap queues; User K-INBOX/cancel uses actual checkpoint ID/current revision and stores original C cancellation frame. Subsequent normal recipient open cannot revive the message or create input. C CANCELLED and original E INTENT remain. |

Normal close/resume changes physical generations. Each action saves its immutable original binding; reader uses original H episode rather than mutable final generation. BUSY table text or a card screenshot alone is insufficient: the native read's current-custody availability and original raised A bytes must agree. Busy turn is unanswered and performs no extra tools.

checkpoint exports scoped original E cause/INTENT/event, C enqueue/message/delivery and matching H commands, supplying actual IDs without a new list/probe. K-INBOX/check-unknown remains only an ID-specific state/requeuedAs view; it cannot prove actor/body or delivery. Host internal sends are not invented User entries in Node's journal.

## Direct readback and history boundaries

Reader opens only normally closed state.sqlite with mode=ro&immutable=1/query_only, rejects nonempty WAL, binds normal-close PID/root identity, and checks DB/WAL/SHM bytes before/after. It exports scoped E/C/H/A evidence, never credentials or a whole DB copy. Original checkpoint file hashes and loaded reader/module hashes must match.

Owner configuration sequence stays exact: the original five business CAS operations, then each prescribed Host route/gate and one route-change CAS. Final head is derived from these verified operations plus the one actual legal transition. All original gate/state/reason/count/revision and expired/restored grant assertions remain. Unchanged baseline events/routes/triggers/escalations/C rows remain exact; extra/missing operations fail. No fixed count is relaxed to accept unrelated results.

Hard delivery assertions bind original C HOST_RULE actor, source/cause/Owner policy+route revisions/mechanical body; original E cause and INTENT; exact H request/receipt/ticket/nonce/session/generation; one RPC command/source epoch/cursor to raw A ACK; numeric/string RPC ID type; one actual created recipient turn; actual WORK registration/normal H open/immutable seat incarnation/Codex program-custody digest/physical StopFact. ACK proves accepted native delivery; CLI completion is separate. PENDING/UNKNOWN or model prose cannot prove either.

No persisted physical OS pipe-write counter exists. This reader asserts one original logical C/H send and matching RPC command with unchanged IDs/bytes, no second turn. It never reports pipeWrites=1 from row count. Actual late ACK/UNKNOWN after route change needs a real occurrence; no fault/ACK synthesis or mutation replay is introduced.

## Explicit NOT_RUN boundaries

STALLED's enum/table and old trigger requirement are not a producer: current HostHealthProof recognizes only context-window/repeated-failure terminal signals; Host rule observer consumes reject-cap gates. V08_STALL_CHAIN remains NOT_RUN. No retained destination means queueing with no automatic Host admission/open; this case's normal User admission does not prove autonomous activation. Busy queue does not prove unanswered vendor state becomes idle on resume. Genuine forged-sender/cross-domain/subordinate-to-Owner Model caller paths are unavailable from the advertised schema. OWNER has no retained Codex recipient and must not be represented by a fabricated model session/popup.

VALIDATION: signed Node syntax, Python in-memory source compilation and static diff/ownership only; no candidate database or runtime is executed. All app/model/CLI/native/cloud/Win11 results remain NOT_RUN. FAILURES / RISKS: real question availability, fixed installed CLI pin, capacity/admission, lifecycle callbacks and same-byte installed candidate require integration/runtime verification. No self acceptance or cleanup of Root trees.

## References, adoption and difference

- origin/main AGENTS and docs/model-routing: light work within phase, actual installed subject/direct sources, E2E primary, no authority expansion.
- Historical gogo-party docs/design/06-context-session-governance.md and packages/seat-runtime/src/seat.ts / seat-runtime.ts: source facts, CLI lifecycle and delivery ownership; none grants design37 policy authority or creates STALLED.
- Repository upstream-reference-map, source-audit/06-runtime-protocol-and-adapter-sources, adapter-spike/02-mvp-adapter-decision, 2026-09-26-kernel-parts-harvest: pinned typed protocol/terminal fences and rule-chain references; no imported scheduler/threshold/harness permission.
- Frozen gogoke-37 PLAN E.2/R-ESC/V08 vs E.3, native seat/host_rule, inbox/host_rule, product_database/v37_host_rule: current E/C/H mechanical rule and historical ACK recovery. Model K-POLICY escalation remains unsupported; automatic Host effect has its distinct native path.
- Existing M1/M2 ActualProduct, hard locator, real question-card flow, caption close and immutable readers. Installed exe/native/resource hashes, candidate registration/process ancestry/bootstrap/resource page and physical CLI pin bind the actual subject; config.sourceCommit or journal state alone cannot prove compiled bytes. No telemetry/agent.act/browser auth/credential read is added.

OPEN_QUESTIONS / RECOMMENDED_NEXT_ACTION: Root supplies callbacks, integrates owned scripts and runs affected same-byte cloud plus actual ordinary installed-product E2E. Retain worker checkout until integration/validation and custody-aware cleanup.
