# M2 provider boundary cases: current executable preflight and explicit gaps

`runProviderBoundaryCases(product, config, journal)` is import-only. The caller
must already hold the M1 `ActualProduct` connection to the exact installed
candidate's unique WebView2 Home, with telemetry disabled and no `agent.act`.
The function never starts login, an authentication page, a model turn, a
browser, or another executable. It uses only the current User pipe and the
registered fixed CLI instances. This script has **not been run**.

The private `config.providerBoundary.cases` lists exactly one case each for
`claude`, `opencode`, and `grok`: `instanceId`, `seatId`, `worktreeId`, fixed
`version`, and lower-case executable `sha256`. The caller's `domainId` and
`repositoryId` must identify the private `gogokeSeatTestbed`, with the
already admitted `codexTestM1` lead context. Seats and worktrees must have
been created by the real product and be Idle/REGISTERED; this function never
creates or repairs them. A missing or logged-out provider is `NOT_RUN`; no
automated login follows. Antigravity `1.2.11` is always
`NOT_RUN_OWNER_DECISION_PENDING` because its current shared Windows credential
and memory-close contract are not qualified.

For each already logged-in provider, the driver checks the actual instance
status, E seat card and F graph, then reserves, commits and opens one new H
session, reads its own CLI-owned `capability-probe`, stops the process and
releases the admission. `ActualProduct.operation` saves the exact JSON frame
and request ID before each single User invocation. Unknown, disconnect or
timeout is an error and never triggers a new request ID or implicit resend.
The driver does not ask a model to raise a question it cannot answer.

After the candidate **normally closes**, run signed Python
`m2-provider-readback.py STATE_ROOT NEW_PRIVATE_OUTPUT ORIGINAL_JOURNAL`.
It rejects nonempty WAL and opens `state.sqlite` in `mode=ro&immutable=1` with
`query_only`. It rechecks F instance digest/version, E seat, F worktree path,
H original request bytes and STOPPED custody, the original capability
request/receipt bytes, A raw frames, H RPC commands and normalized events.
It reports pending raw frames by their real method and verifies that no model
`send` occurred in this preflight. Existing before/after memory snapshot
references are rehashed if present, but their general memory counters are
not vendor-specific memory-store proof. Both scripts keep every V03b, V04b
and V10 outcome `NOT_RUN`; a complete preflight is not an acceptance result.

## Current executable boundaries

| Check | Direct current source | Current outcome |
| --- | --- | --- |
| V03b | `v37_capability.rs` reports `nativeQuestionCard: UNSUPPORTED_REPLY_ENCODER` for fixed Claude/OpenCode/Grok. `v37_qcard_user.rs` exposes `recover`/`answer` only for an already captured Codex-native card; `v37_output.rs` raises only Codex's `requestUserInput`. | `NOT_RUN_NO_PROVIDER_BOUND_HOST_CARD`: no original asking-turn host-card raise/answer transport to test. Codex's M1 native card does not satisfy this check. |
| V04b | Codex loaded features provide `memories:false`; fixed Claude/OpenCode/Grok capability returns `memoryOffLaunch: NOT_RUN`. H/F select the actual home and workspace, but there is no per-provider observed memory-store write ledger or instruction-load manifest, nor a completed same-instance/two-project original-input case. | `NOT_RUN_NO_EFFECTIVE_MEMORY_AND_CROSS_PROJECT_INPUT_PROOF`. A launch flag, distinct HOME, model statement, or general snapshot cannot prove the requirement. |
| V10 | At this package's `fb5ceb2d` baseline, public `K-SESSION/open` registers WORK and only the side composition registers SIDE_CHAT. Controller's pending shared change adds User-only `open` with the sole optional `purpose: "FORMAL_REVIEW"` value, persists that purpose, starts a fresh native session and denies resume/reconnect/compact/renew before process effects. | `NOT_RUN_NO_SAME_SEAT_PROVIDER_SIDE_SOURCE`. The current V12 side case uses Codex seats; its marker cannot stand in for a Claude, OpenCode or Grok side history. Even after the formal ingress lands, fresh identity, rejected recovery, no side/private marker in original H input, and worker/lead privacy need their own direct same-seat evidence. |

The exact needed product interfaces belong to Controller's shared scope:
provider-bound host question cards tied to the captured A asking turn and
current H session, with C answer and H exact write receipt; an observed
effective memory/instruction provenance source plus an authorized second
project's original H input; and provider-specific same-seat SIDE_CHAT facts
before a fresh `FormalReview` case can test native identity and recovery
refusals. The optional formal purpose is a Controller shared-product change,
not evidence of V10 behavior until the exact installed binary and H/A facts
are read back.
The importer must select each real provider session by exact session ID and
F/H pin, not combine all providers under one CLI version.

## References and differences

Read the active `docs/design/gogoke-37-plan-v1/PLAN.json` checks V03b/V04b/V10;
the fixed `K-INSTANCE`, `K-SESSION`, `K-QCARD`, `K-SEAT`, E/F/H/A/ledger source;
`tools/e2e/m1-win11.mjs`, `product-cdp.mjs`, `m1-readback.py`, and M2's
immutable reader. GOGO PARTY `packages/room/src/accounts.ts` and
`packages/seat-runtime/src/isolation.test.ts` show historical per-seat homes
and negative isolation controls; they do not certify the current native
provider. NaveHQ's isolation fit note explicitly separates profiles,
credentials and runtime evidence; LoomOS's design remains a planning source,
not an executable provider proof. Repository `docs/research/reuse-blueprint.md`
recommends source-bound context manifests. The existing fixed-provider
`COMMANDS_REFERENCES.md` records Claude `--safe-mode` and OpenCode `--pure`
as launch controls, not effective memory proof. The
[ACP prompt-turn contract](https://agentclientprotocol.com/protocol/v1/prompt-turn)
distinguishes interim `session/update` from the correlated prompt response;
no stream chunk is promoted to a delivered answer or completed check.

Only signed Node syntax, signed Python compile parsing and `git diff --check`
are in this package. Native build, installed Win11, actual providers, account
state, memory isolation, cards and formal review remain **NOT_RUN**.
