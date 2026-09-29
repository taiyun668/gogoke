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
Instances are global: K-INSTANCE uses the native `global` domain and only a
global management caller. A project seat consumes its bound instance through
H's trusted internal lookup; it cannot create a project-local shadow instance
or address another project's credentials by changing a request domain.

`APPLIED` means the durable owner committed the operation. For writes, `REPLAYED`
returns the same committed result for the same request ID and identical request bytes;
reuse of that ID with different bytes is `CONFLICT`. A competing expected
revision is `STALE`. `UNKNOWN` means commit outcome could not be established:
the caller must query by request ID, not blindly retry with a new ID. The owning
native journal must retain the unresolved intent, prevent a second send or
cleanup under a new ID, and resolve it only from a trustworthy stop, delivery,
or filesystem receipt. Resolution binds the original request ID to the final
receipt. A denied
or failed operation does not advance a revision. Read operations do not change
their source revision. A real implementation must serialize the version check,
write, and receipt on its owning store connection. A process restart must
recover request IDs and committed receipts before accepting new writes.
The test fakes compare the received wire bytes, including JSON key order;
parsing and re-encoding cannot decide request identity.
Current scoped reads such as `state-card` and `call-permission-table` replay
only while their source revision and observed context are unchanged. An exact
read retry after either changes is `STALE`, so an old ready state or permission
table cannot masquerade as a current observation. A committed write receipt
remains historical and replayable while the caller retains current authority.

The catalog in `catalog.ts` is the closed operation list. The following is the
operation-specific contract that implementations and the shared conformance
suite must enforce. The generic fake checks only the common
envelope/revision/replay rules for non-UI operations; its K-UI path returns
`UNSUPPORTED`. The separate test-only `uiFake.ts` forwards a host-mapped
source operation and returns its receipt bytes exactly, without a UI store.
`coreFake.ts` additionally exercises the
operation-specific H/A/C paths used by `coreConformance.ts`. `m1Fake.ts` and
`m2Fake.ts` add test-only state machines exercised by the matching reusable
conformance cases. Neither fake is product behavior or acceptance. The status
of every planned shared file is in `SHARED_STATUS.json`.

Current fake behavioral coverage is deliberately narrower than the catalog:

| Family | Fake behavior exercised | Still unsupported or untested |
| --- | --- | --- |
| K-SESSION | reserve/commit/release, open/stop/stop receipt, send/append, capability/output reads, compact/renew generation receipts, reconnect including unknown generation; resume from confirmed STOPPED with held admission, trusted binding/custody/capability and same-ID UNKNOWN reconciliation | real custody, admission and adapter proof |
| K-LEDGER | record, scoped query, subscribe/resume/end with source epoch, cursor gap and scope checks | real projection of old events and native subscription store |
| K-INBOX | enqueue/edit/cancel, delivery and unknown check, steer for exact live turn, confirmed-failure requeue into a new checked target | real H delivery chain and native target eligibility |
| K-QCARD | raise, answer, expire, recover; native capability pass-through refusal | real adapter capability and native storage |
| K-SEAT | create from stored template, tune, bind, busy change refusal, reclaim, short-to-long, state card, takeover answers tied to host-provided opaque epoch, current question set, taker and instance binding; cited/unknown answers; currentness rechecked on state-card; binding and reclaim clear active answers, and a reclaimed seat is never ready | real host option facts, **H dispatch admission NOT_RUN**, user bounds and occupancy |
| K-INSTANCE | register, observed reads, repin after upgrade; temporary SESSION/CALL create/close/cleanup fake with opaque directory reference and trusted callbacks | real native directory identity, recovery, memory-off and pin measurements |
| K-SIDE | create, resume, archive, restore, delete, scoped reads | real D store and A tier deletion |
| K-POLICY | current scoped permission-table read from a native observation with current authority check, raw-byte replay/collision and immutable stored receipt; gate submit/decide/stage, trigger register/recover/cancel | escalate UNSUPPORTED: configured route, cap/stall trigger, intent, delivery and ledger completion need the existing coordinator plus C/H receipts; real scheduler |
| K-WORKTREE | create/register, classify/graph, merge, cleanup and unknown merge | real Git and OS confinement |
| K-UI | separate stateless forwarding fake checks host-mapped read/action source, outer and source grants, full mapped request and exact receipt bytes/correlation including UNKNOWN | production G mapping and all A1-A4 read models remain NOT_IMPLEMENTED; generic fake still UNSUPPORTED |

All real paths, including the shared files listed as pending, are `NOT_RUN` or
`NOT_IMPLEMENTED`. The fake callbacks model native observations but do not
establish that a same-user child is confined to an allowed worktree.

| Family / owner | State and operation rules | Cross-line seam |
| --- | --- | --- |
| K-SESSION / H | `admission-reserve` creates one pending reservation across E cap and F instance capacity; `admission-commit` consumes it exactly once; `admission-release` releases only an uncommitted or stopped claim. `open` requires a committed reservation and pinned executable; `send` and `append-without-turn` require an active generation; `stop` enters stopping and only `exit-and-stop-receipt` with native custody proof makes it stopped. `resume` differs from `reconnect`: it may start a new generation only after durable STOPPED, with an unreleased admission and the same trusted driver, instance and pinned binary, current capability, and confirmed custody. Its wire payload carries only the old generation; H obtains the continuation reference internally. A native start of uncertain outcome records `UNKNOWN`, prevents a new-ID start, and resolves from the original request ID and trusted receipt. `reconnect` only reconciles an existing or unknown generation. `compact` and `renew-session` bind old/new generations explicitly and never hide an unknown stop. A capability miss is `UNSUPPORTED`, not success. | B emits tagged vendor-raw output; A normalizes. E requests compact/renew here rather than constructing vendor commands. |
| K-LEDGER / A | `record` appends once by source event ID and owning source cursor. `scoped-query`, `subscribe`, `resume-subscription`, and `end-subscription` expose one cursor/epoch; duplicates are suppressed by source ID and gaps are explicit. A project reader never receives GLOBAL entries. | Existing `store/orchestration.rs` event IDs and ordering are projected into this **same** ledger. Do not copy those events to an independently authoritative history. Side-chat turns have their own tier; deletion targets that tier only. |
| K-INBOX / C | `enqueue` creates pending message ID/revision/seat/turn/generation. `edit` and `cancel` compare the message revision, so exactly one racing mutation wins. `steer` inserts a queued message into the exact live turn; if that turn ended, it stays queued and cannot move to the next turn. `deliver` and `steer` recheck the current grant and use H prepare → beginCommitted → completion; delivered requires that receipt. If a prepared steer finds its turn ended, H must confirm abort before the item remains queued; unconfirmed abort is UNKNOWN. `check-unknown` is read only. `requeue` only follows confirmed failure: a new ID and explicitly checked seat/turn/generation create a new pending item; the old item stays FAILED, advances one revision and records the new ID, while its original failure receipt remains available. | E and G use the same message identity and receipt. |
| K-QCARD / C | `raise` creates a card bound to request/seat/turn/generation with options, one recommended option, and a free answer. `answer` wins once; duplicates conflict. `expire` closes unanswered cards; `recover` restores only a still-open card after restart. | B native cards pass through; C owns a fallback card only for an adapter whose capability report says no native card. |
| K-SIDE / D | `create` records its registry in D's native store in the coordinator domain. `resume`, `archive`, `restore`, and `delete` make active → archived → active/deleted transitions; deleted is terminal. `pending-delta` and `read-thread` use an A ledger source cursor and scoped grant. | A stores side-chat turns in a separate tier. A native fork is permitted only if it reproduces ledger replay behavior. |
| K-SEAT / E | `create-from-template` copies the template, never aliases it. `tune`, `bind-instance`, `change-instance`, `reclaim`, `short-to-long`, `state-card`, and `takeover-answers` compare seat revision. A busy seat cannot change instance. Lead callers can touch lead-layer seats only within user bounds and never their own binding; the user may tune or reclaim the lead. | F supplies instance references; H holds active admission; G reads E state. |
| K-POLICY / E | `call-permission-table` is a current scoped read. `gate-submit`, `gate-decide`, `stage-transition`, and `escalate` compare current policy and stage revisions; a gate decision records pass or rejection plus reason. `escalate` carries only a trigger ID from the caller: E derives the current configured route, destination and cap/stall reason, rejects an inactive trigger or an unresolved attempt, and never lets a subordinate address Owner directly. A first native transaction rechecks trigger, policy and stage, records one intent and one send authority; C/H delivers outside that transaction; a second transaction accepts only a trusted delivery receipt, then commits the ledger event and final receipt. Missing delivery outcome is `UNKNOWN` and never starts a second delivery. `trigger-register`, `trigger-recover`, and `trigger-cancel` use the existing coordinator with stable trigger IDs and de-duplication. | Caller identity is the native issuer, never Node/model input. C delivery checks this current authority. No database transaction is claimed to include an external delivery. |
| K-INSTANCE / F | `register` selects only a controlled driver ID on the wire; F establishes the global persistent instance home, program digest and version from native observation, not caller-provided path or digest. Login begins `UNKNOWN` until a trusted adapter observation settles it. `install-state`, `login-state`, `version-and-new-version`, and `concurrency-input` read observed facts. `repin-after-manual-upgrade` requires a new measured binary identity before any launch. `home-lifecycle` manages only temporary SESSION/CALL directories, not the persistent instance home: `CREATE` carries instance ID, kind, owner ID and generation that F validates against the native owner binding; `CLOSE` requires H's stop/call-completion proof; `CLEANUP` additionally requires zero active admission and the recorded directory identity and instance ownership. Its receipt carries opaque `directoryRef` and native receipt ID, never a path or credential. An unresolved native intent is `UNKNOWN` and prevents reuse or deletion. | H consumes the global pin and capacity through the single K-SESSION admission boundary; a project seat never writes the global instance registry. |
| K-WORKTREE / F | `create` and `register` bind an isolated worktree to a seat and verified repository identity. `classify-single-or-mixed` and `graph-query` are scoped reads. `merge` requires the seat's current merge grant and records decision/reason. `cleanup` requires the native stop fact and zero active admission reservations; directory names alone are insufficient. | Existing Tauri worktree commands remain legacy Codex-thread commands; new seats use the host F implementation. |
| K-UI / integrator | `read-models` maps each view to one current source operation; `actions` forwards an allowed operation and returns its exact receipt. UI has no durable state authority. | G does not add operations. Unknown and unconfirmed states remain visible and do not become success. |

All real lines must run the shared conformance cases for their operations and
add family-specific tests for these transitions. Fake PASS only proves that
the shared test instrument can exercise an implementation. Real results are
settled at M1/M2/M3 per the plan, and M3 repeats every family on the candidate.
