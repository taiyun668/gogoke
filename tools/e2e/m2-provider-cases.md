# M2 real provider boundary cases

`runProviderBoundaryCases(product, config, journal)` uses the already installed,
logged-in native product and its original User operation pipe. Importing the
module starts nothing. Run it after the existing `m2-win11.mjs` provider capture
loop in a distinct H session; `m2-readback.py` depends on that loop's unchanged
`journal.providerCases` and original golden-source export. The new module writes
`journal.providerBoundaryCases` and never reuses or replays an old send.

The private `config.providerBoundary.cases` contains exactly one fixed
`claude`, `opencode`, and `grok` row with `instanceId`, `seatId`, `worktreeId`,
`version`, and executable `sha256`. Claude also needs `worktreeRoot`, obtained
automatically from a prior normally closed F worktree readback. The driver
checks that root is local under the private candidate state root and that its
random new marker filename is absent from both F's worktree and the testbed
source before the model prompt. The final reader independently compares the
configured root with F's persisted `worktree_path` and identity. These are
machine facts, not Owner-entered paths. A missing/login-required instance is
`NOT_RUN`; no login or external browser is opened.

## Claude V03b direct case

The actual fixed `2.1.196` instance/seat/F worktree is reserved, committed,
opened and probed. The static capability
`SOURCE_PRESENT_NATIVE_ASK_USER_BEHAVIOUR_NOT_RUN` only licenses this trial;
it is never a PASS. One original H User send asks the model to call
`AskUserQuestion` with one non-secret `Format` choice, `JSON` or `Plain`, and
to create a new prescribed JSON marker only after the User selects `JSON`.
The User request frame and ID are saved before its sole invocation. An
`UNKNOWN` send is an in-flight original request: the driver polls only
`output-stream`, never resends it or substitutes a new request ID.

The original output's `nativeCardRefs` must expose one `OPEN` card whose
`turnId` equals the original H send request ID. `K-QCARD/recover` must return
the complete source-bound `originalInput` and every question with
`idOrigin=HOST_DERIVED_ARRAY_INDEX`, `hostIndex`, `hostQuestionId`, original
options and `multiSelect`; the prescribed case requires one `host0` question.
The User sends one `K-QCARD/answer` with `answers.host0=["JSON"]`. An
`ANSWERED`/`NATIVE_EXACT_WRITE_RECEIPT` result means only H wrote the exact
`control_response`; it explicitly reports no vendor consumption. If that
answer is `UNKNOWN`, there is no retry. The driver only polls the original
H send receipt until the actual CLI Result is observed, then stops/releases.
Timeout or uncertain custody preserves the original run for Controller
disposition. Model output is never executed by the host.

Only after normal product close does `m2-provider-readback.py` verify the
original User bytes, F/E/H fixed instance and physical worktree, H User echo
UUID/text/session, the A `can_use_tool/AskUserQuestion` frame, complete C
card/source descriptor, original User answer, H no-ACK exactwrite step,
subsequent matching Claude `tool_result`, same-session successful Result,
later assistant marker output, and the actual new JSON marker file. Its
`DIRECT_ORIGINAL_CLAUDE_QUESTION_ANSWER_CONTINUATION_REQUIRES_REVIEW` is a
direct evidence label, never milestone acceptance. The static source shape
or successful pipe write alone cannot produce this label. All raw frames and
commands remain in the private normal-close readback for independent review.

OpenCode and Grok still run their real fixed capability preflight without a
question send because the current pinned codecs expose
`UNSUPPORTED_REPLY_ENCODER`. Antigravity remains
`NOT_RUN_OWNER_DECISION_PENDING`. An ordinary ACP permission prompt is never
called a question card.

## V04b and V10 preparation and limits

Optional `crossProject` on a provider row names a *different* registered F
repository/worktree and E seat on the *same* instance. The driver checks the
original F graph and Idle E seat; the immutable reader checks those persisted
bindings again. This is readiness for two concurrent project sessions, not
evidence that vendor memory was off. V04b requires those two original H input
streams plus effective vendor memory-store and loaded-instruction provenance
that excludes auth/key/token material. The current provider capability says
`memoryOffLaunch=NOT_RUN`, and the product has no qualified vendor instruction
load or memory write readback. V04b remains **NOT_RUN** even if F bindings exist;
launch flags, separate HOME and general snapshots are insufficient.

Optional `reviewSource` names a distinct F source seat/worktree. The product's
actual side open is `gogoke.37.owner-side-open.v1` with original source
session/cursor, nested K-SESSION open and K-SIDE create; the provider seat
would be the SIDE_CHAT seat and then reopen as fresh `FORMAL_REVIEW` after a
physical stop. Current rows do not provide that original source session,
side marker, D create, or subsequent formal H/A input. An ordinary WORK
session and the Codex V12 side case cannot replace them. V10 remains
**NOT_RUN** until a same-provider-seat side source, fresh formal native
identity, refusal of continuation, and independent immutable H/A/D/F
readback exist. No model self-report proves zero inheritance.

The historical `packages/seat-runtime/src/claude-seat.ts` informs the question
shape; current fixed codec/H/C source is authoritative. The actual source
chain is `provider_evidence/claude_question.rs`, `v37_qcard.rs`,
`rpc_journal.rs`, `v37_output.rs`, `journal.rs`, and F's registered rows.
`m2-sidechat.mjs` supplies the real D composition precedent. The fixed SDK
0.3.196 source and CLI 2.1.196 source are recorded in
`CLAUDE_QUESTION_REFERENCES.md`. The code here has received only signed Node
syntax, Python AST parsing, and `git diff --check`; installed candidate,
model, browser, login, native build, and Win11/SAC are **NOT_RUN**.
