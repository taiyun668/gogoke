# Design 37 L0 wire contract

`gogoke.37.operations.v1` is a separate major schema from the existing
`gogoke.s1-r4.objects.v1` stored-object schema. The old decoder remains the
authority for old objects. A v37 request or receipt cannot be decoded as an
old object. Unknown v37 major versions and operation names fail closed.

Every request has `family`, `operation`, `requestId`, `targetId`, `domainId`,
`expectedRevision`, and `payload`. Every receipt has the same correlation
fields, `status`, `previousRevision`, `revision`, and `result`. Revisions are
canonical decimal uint64 strings. Requests never contain a caller or grant.
The host supplies principal, seat, role, current policy revision and
revocation head from its native issuer; it rechecks the grant in the same
transaction as a write and on **every** scoped read and delivery. A Node
object, model output, directory path, environment variable, or UI state cannot
issue authority. Unwired production calls fail with `UNWIRED`.

`APPLIED` means the durable owner committed the operation. `REPLAYED` returns
the same committed result for the same request ID and identical request bytes;
reuse of that ID with different bytes is `CONFLICT`. A competing expected
revision is `STALE`. `UNKNOWN` means commit outcome could not be established:
the caller must query by request ID, not blindly retry with a new ID. A denied
or failed operation does not advance a revision. Read operations do not change
their source revision. A real implementation must serialize the version check,
write, and receipt on its owning store connection. A process restart must
recover request IDs and committed receipts before accepting new writes.

The catalog in `catalog.ts` is the closed operation list. The following is the
operation-specific contract that implementations and the shared conformance
suite must enforce. The generic fake checks only the common
envelope/revision/replay rules for non-UI operations; K-UI returns
`UNSUPPORTED` until an integrator can forward an underlying exact receipt.
`coreFake.ts` additionally exercises the
five session operations, two ledger operations, and inbox enqueue/edit/cancel/
deliver/check-unknown paths used by `coreConformance.ts`. `m1Fake.ts` and
`m2Fake.ts` add test-only state machines exercised by the matching reusable
conformance cases. Neither fake is product behavior or acceptance. The status
of every planned shared file is in `SHARED_STATUS.json`.

Current fake behavioral coverage is deliberately narrower than the catalog:

| Family | Fake behavior exercised | Still unsupported or untested |
| --- | --- | --- |
| K-SESSION | reserve, commit, open, stop, stop receipt | other nine operations; real custody and admission |
| K-LEDGER | record, scoped query | subscriptions; real projection of old events |
| K-INBOX | enqueue, edit, cancel, deliver, check unknown; unknown requeue refusal | steer and resolved requeue; real H delivery chain |
| K-QCARD | raise, answer, expire, recover; native capability pass-through refusal | real adapter capability and native storage |
| K-SEAT | create from stored template, tune, bind, busy change refusal, reclaim, short-to-long, state card | takeover answers; real user bounds and occupancy |
| K-INSTANCE | register, observed reads, repin after upgrade | home lifecycle; real memory-off and pin measurements |
| K-SIDE | create, resume, archive, restore, delete, scoped reads | real D store and A tier deletion |
| K-POLICY | gate submit/decide/stage, trigger register/recover/cancel | permission-table read, escalate, real coordinator scheduler |
| K-WORKTREE | create/register, classify/graph, merge, cleanup and unknown merge | real Git and OS confinement |
| K-UI | none; both operations return UNSUPPORTED | exact forwarding and read models |

All real paths, including the shared files listed as pending, are `NOT_RUN` or
`NOT_IMPLEMENTED`. The fake callbacks model native observations but do not
establish that a same-user child is confined to an allowed worktree.

| Family / owner | State and operation rules | Cross-line seam |
| --- | --- | --- |
| K-SESSION / H | `admission-reserve` creates one pending reservation across E cap and F instance capacity; `admission-commit` consumes it exactly once; `admission-release` releases only an uncommitted or stopped claim. `open` requires a committed reservation and pinned executable; `send` and `append-without-turn` require an active generation; `stop` enters stopping and only `exit-and-stop-receipt` with native custody proof makes it stopped. `resume`, `reconnect`, `compact`, and `renew-session` bind old/new generations explicitly and never hide an unknown stop. A capability miss is `UNSUPPORTED`, not success. | B emits tagged vendor-raw output; A normalizes. E requests compact/renew here rather than constructing vendor commands. |
| K-LEDGER / A | `record` appends once by source event ID and owning source cursor. `scoped-query`, `subscribe`, `resume-subscription`, and `end-subscription` expose one cursor/epoch; duplicates are suppressed by source ID and gaps are explicit. A project reader never receives GLOBAL entries. | Existing `store/orchestration.rs` event IDs and ordering are projected into this **same** ledger. Do not copy those events to an independently authoritative history. Side-chat turns have their own tier; deletion targets that tier only. |
| K-INBOX / C | `enqueue` creates pending message ID/revision/seat/turn/generation. `edit` and `cancel` compare the message revision, so exactly one racing mutation wins. `steer` targets a live turn. `deliver` rechecks current grant and uses H prepare → beginCommitted → completion; delivered requires that receipt. `check-unknown` is read only; `requeue` changes a resolved eligible item, never duplicates an uncertain delivery. | E and G use the same message identity and receipt. |
| K-QCARD / C | `raise` creates a card bound to request/seat/turn/generation with options, one recommended option, and a free answer. `answer` wins once; duplicates conflict. `expire` closes unanswered cards; `recover` restores only a still-open card after restart. | B native cards pass through; C owns a fallback card only for an adapter whose capability report says no native card. |
| K-SIDE / D | `create` records its registry in D's native store in the coordinator domain. `resume`, `archive`, `restore`, and `delete` make active → archived → active/deleted transitions; deleted is terminal. `pending-delta` and `read-thread` use an A ledger source cursor and scoped grant. | A stores side-chat turns in a separate tier. A native fork is permitted only if it reproduces ledger replay behavior. |
| K-SEAT / E | `create-from-template` copies the template, never aliases it. `tune`, `bind-instance`, `change-instance`, `reclaim`, `short-to-long`, `state-card`, and `takeover-answers` compare seat revision. A busy seat cannot change instance. Lead callers can touch lead-layer seats only within user bounds and never their own binding; the user may tune or reclaim the lead. | F supplies instance references; H holds active admission; G reads E state. |
| K-POLICY / E | `call-permission-table` is a current scoped read. `gate-submit`, `gate-decide`, `stage-transition`, and `escalate` compare current policy and stage revisions; a gate decision records pass or rejection plus reason. `trigger-register`, `trigger-recover`, and `trigger-cancel` use the existing coordinator with stable trigger IDs and de-duplication. | Caller identity is the native issuer, never Node/model input. C delivery checks this current authority. |
| K-INSTANCE / F | `register` establishes persistent instance home and pinned program identity. `install-state`, `login-state`, `version-and-new-version`, and `concurrency-input` read observed facts. `repin-after-manual-upgrade` requires a new measured binary identity before any launch. `home-lifecycle` cannot expose credentials to Node or other instances; session/call directories are temporary. | H consumes pin and capacity through the single K-SESSION admission boundary. |
| K-WORKTREE / F | `create` and `register` bind an isolated worktree to a seat and verified repository identity. `classify-single-or-mixed` and `graph-query` are scoped reads. `merge` requires the seat's current merge grant and records decision/reason. `cleanup` requires the native stop fact and zero active admission reservations; directory names alone are insufficient. | Existing Tauri worktree commands remain legacy Codex-thread commands; new seats use the host F implementation. |
| K-UI / integrator | `read-models` maps each view to one current source operation; `actions` forwards an allowed operation and returns its exact receipt. UI has no durable state authority. | G does not add operations. Unknown and unconfirmed states remain visible and do not become success. |

All real lines must run the shared conformance cases for their operations and
add family-specific tests for these transitions. Fake PASS only proves that
the shared test instrument can exercise an implementation. Real results are
settled at M1/M2/M3 per the plan, and M3 repeats every family on the candidate.
