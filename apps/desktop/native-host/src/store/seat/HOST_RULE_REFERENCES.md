# E.2 reject-cap Host rule

TASK: M2 E.2 / R-ESC / V08, base `d083912abe8543b1ffa5084f609faff24581df4c`.
RESULT / CURRENT_STATE: sealed native E observation and original escalation INTENT reservation prepared; integration, native execution and acceptance are NOT_RUN.
FILES_CHANGED: only `host_rule.rs`, `host_rule_tests.rs`, this reference.

## Direct authority and identity

`policy::gate_decide` verifies the original native caller and writes the gate count/state and `gate-decide` event in one transaction. Its fingerprint binds the reviewer, gate, decision, reason, policy/gate revisions and original request bytes. The Host observer consumes the committed `ESCALATION_REQUIRED` event, not model prose, a client state claim, a scheduled trigger or a pending tool reply. It does not repeat the rejection or mint a `NativeSeatCall`.

Controller selected the existing immutable logical identity closure: seat creation refuses an existing seat ID, generates one incarnation and saves its original revision-1/generation-1 operation snapshot; production reclaim changes state/generation/revision without replacing identity. Observer compares the current incarnation/layer/parent with that original create snapshot. Source idle, new physical generations and reclaim do not erase a committed cause or grant authority over a source process. Destination must still be a usable same-domain seat (or permitted `OWNER`). If seat ID reuse or incarnation replacement becomes supported, the original gate event must gain a stronger source binding before this rule can remain enabled.

An existing same-connection Owner transaction is mandatory at observation, revalidation and reservation. Current gate, cause fingerprint/reason, stage, Owner policy head and configured route must agree with the sealed proof for a new effect. Lead cannot address `OWNER`; foreign-project destinations are refused. Notice bytes contain only native mechanical identities/count/cap/policy facts; the model reason stays in the original E event as a reference.

`trigger_id` and `request_id` derive only from domain plus original cause event. Route changes cannot produce a new ID for the same cause. Reservation uses the existing escalation/event tables and unique keys; no coordinator registration is fabricated. A replay returns the original INTENT/UNKNOWN/DELIVERED state with `replayed=true`, which is not delivery permission. Caller must roll back the whole transaction on any error. No pipe write or nested transaction occurs here.

## Integration contract

Root declares this file as a child of `seat::policy` and exports the sealed type and three frozen transaction functions. Proof string getters return `&str`; `policy_revision()` and `route_revision()` return `i64`. The proof has no wire/JSON constructor or editable sender/body/target. C records the original proof facts and E INTENT before physical delivery. `revalidate_host_escalation_in_transaction` is the current-policy check for a **new** effect; original UNKNOWN/receipt settlement must verify historical C/E/H request evidence without turning it into a fresh send or requiring the old route to remain current. Existing E delivery settlement already checks original intent/receipt identities separately.

## References: adoption and difference

- Historical gogo-party `docs/design/06-context-session-governance.md`: adopt project-scoped facts and immutable provenance; do not let raw instructions or derived context become commands.
- Repository `docs/research/upstream-reference-map.md`, `docs/research/source-audit/06-runtime-protocol-and-adapter-sources.md`, `docs/research/adapter-spike/02-mvp-adapter-decision.md`: retain native ID/source separation and evidence boundaries; no upstream scheduler, model approval or harness permission is imported as Host authority.
- Frozen `docs/design/gogoke-37-plan-v1/PLAN.json` E.2 / R-GATE / R-ESC / V08, `seat/policy.rs`, `seat/mod.rs` and existing `seat/tests.rs`: E.2 rule execution is distinct from E.3 scheduled coordinator triggers. Reuse the actual gate writer, seat create identity snapshot, policy event, escalation INTENT and native Owner transaction guard.
- Private Specialist report `gogoke-v08-host-escalation-specialist-9f303ad6-20261003.md`: architecture input only. Adopt fact-based Host authority after a genuine native gate decision; do not require a live model sender or MESSAGE grant. This report is not acceptance.

IMPORTANT_DIFF / INVARIANTS_CHECKED: sealed current-route rule authority, source logical identity retained after stop/reclaim, stable cause IDs, original E INTENT/event atomicity; no new schema, scheduler, MCP, CLI, grant or model caller conversion.
VALIDATION: static diff/ownership review only; Rust compilation, native controls, cloud CI, real CLI/model/pipe, Win11/SAC and acceptance are NOT_RUN until Root integrates and executes the affected cloud batch.
FAILURES / RISKS / DEVIATIONS_FROM_PLAN: kernel controls use synthetic instance/login metadata and H turn admission; actual production gate-submit/gate-decide and Host factory/reservation produce the tested E facts. They do not prove real caller ingress, physical delivery or V08 E2E. STALL has no qualifying source producer in this package and remains unimplemented.
OPEN_QUESTIONS / RECOMMENDED_NEXT_ACTION: Root integrates module/exports and C/H historical receipt reconciliation, then performs same-byte cloud checks and independent review. Retain the isolated worker checkout for that integration; no self acceptance or cleanup of Controller worktrees.

## Current recipient readiness

Controller read the actual official `rust-v0.160.0` tag: commit
`a956835d020762cb2b570053af06f643a11c0ecc`. Its
`codex-rs/app-server-protocol/src/protocol/v2/thread.rs` defines
ThreadStartResponse/ThreadResumeResponse and the tagged ThreadStatus; `thread_data.rs`
contains the thread's current status and original turns. The retained new process's
empty in-memory turn ID alone is therefore not readiness. The native Host idle
reader uses the current physical episode's original observed H command/A reply or
original thread/status/changed, rejects unfinished resumed turns, and invalidates
the old idle observation after a new input or active status. Outstanding original
questions, generation changes and unexplained pending requests/items grant no input.
It reuses the existing source and safe point; there is no new RPC, timer or probe.
Official source shapes are not a real-model golden or Win11 execution result.
