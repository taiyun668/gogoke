# WCS-05 Result — independent skill PR audit

## Scope and non-claim

- Task: `WCS-05` from `codex/web-chatgpt-subagent-channel@3c65372c4842ed01291f1caefb205334f4d0e1b6`.
- Repository: `taiyun668/gogoke`.
- Pull request: `#48`.
- Exact reviewed commit: `29f7f434c7d24893edc25861f3c8de7b5f4d7193`.
- PR base used for the diff: `main@5ec3d643a6015de34ef582b5d047e7cc7e46da43`.
- Result branch base: `3c65372c4842ed01291f1caefb205334f4d0e1b6`; the one commit after the reviewed SHA changed only `docs/trials/web-chatgpt-subagent/TASKS.md`, so no later implementation was included in the findings.
- Role: independent, read-only reviewer. This result is trial evidence only. It is not route acceptance, a high-risk audit, formal enablement, merge approval, or release authorization.

Read at the reviewed commit:

- `.codex/skills/web-chatgpt-subagent/SKILL.md`
- `.codex/skills/web-chatgpt-subagent/scripts/ledger.py`
- both ledger test files and `.github/workflows/web-chatgpt-subagent.yml`
- the other files in the PR diff, including the license attribution and existing trial records
- `docs/directions/2026-09-26-web-chatgpt-subagent-channel.md`
- root `AGENTS.md`
- `docs/model-routing.md`

No candidate code, workflow, routing document, `main`, R2-06a, release object, secret, or signing material was changed.

## CI evidence read

For exact head SHA `29f7f434c7d24893edc25861f3c8de7b5f4d7193`, Actions run `36209199913` and ledger job `108312044358` were readable through the GitHub connector. Run/job metadata ended with conclusion `cancelled`, but every listed job step completed successfully. The raw validation log directly shows five tests, all `ok`, followed by `Ran 5 tests in 1.740s` and `OK`.

That is useful execution evidence for the five existing cases. It is not converted here into acceptance, and the overall cancelled conclusion is not represented as a successful final-candidate run.

## Actionable findings

### F1 — High — switched and limit outcomes are not represented consistently

**Requirement:** the counter must track usage by tier, verify the actual reply model after every send, exhaust the requested tier on a switch or visible limit, and stop using it until reset (`docs/directions/2026-09-26-web-chatgpt-subagent-channel.md:69-72`; `.codex/skills/web-chatgpt-subagent/SKILL.md:24,32`).

**Evidence:**

- `active_events`, `count_day`, and `count_week` classify completed events only by `event["tier"]`, which is the reserved/requested tier; they never count `actual_tier` (`ledger.py:71-87`).
- `complete` rejects only the combination `--switched` plus an unchanged tier. A different `--actual-tier` without `--switched` is accepted and recorded (`ledger.py:146-157`).
- The requested tier is blocked only when the caller supplied `--switched` (`ledger.py:158-159`).
- The CLI has no terminal operation for “a limit appeared after Send but no fallback reply supplied an actual tier”; the available commands are only status, reserve, release, complete, reset, and enable-6-pro (`ledger.py:107-127`).

**Impact:** a High-to-Medium fallback can block High while recording no Medium usage, so Medium status and caps remain understated. Omitting one flag can leave a visibly switched requested tier available. A limit-only response cannot be recorded without inventing an actual tier or leaving the reservation unresolved.

**Action:** make contradictory model facts impossible to persist. Derive or strictly validate switch state from requested versus actual tier; represent requested-attempt and actual-served usage explicitly; add a terminal `limit/exhausted` outcome that does not require a fabricated reply tier; and cover all three paths in Windows tests.

### F2 — High — a fresh or unreconciled shared ledger fails open

**Requirement:** all Chat seats must feed the same ledger, and unknown usage since the last reconciliation must make allowance unknown and prohibit dispatch (`SKILL.md:30`; direction document `:69-72`).

**Evidence:** `fresh_state` initializes zero events and no reconciliation/known-state marker (`ledger.py:36-37`). `available` returns `True` for an otherwise empty GPT-5.6 tier while merely returning the advisory text “reconcile other Chat seats separately” (`ledger.py:90-102`). `reserve` then creates the reservation without any reconciliation gate (`ledger.py:134-140`).

**Impact:** first use, deletion/loss of `state.json`, or unreported use by another Chat seat is interpreted as zero use rather than unknown use. The accounting control therefore permits dispatch in precisely the state the skill says must fail closed.

**Action:** add an explicit reconciliation epoch and per-seat known/unknown state. A missing or fresh state file must be unavailable by default until an authorized local reconciliation command records the known baseline. `reserve` must enforce that state atomically.

### F3 — High — the one-send task identity invariant is not enforced across seats

**Requirement:** each dispatch has a distinct task ID identifying one logical prompt and one send attempt; the ID must never be reused to send the same prompt again (`SKILL.md:16`).

**Evidence:** `reserve` accepts any `--task`, creates a new random reservation ID, and does not search existing reservations or completed events for the same task (`ledger.py:134-140`). The shared state already stores the task and seat, so this check is possible. The concurrency test deliberately uses two different task IDs and therefore does not exercise duplicate-task behavior (`test_ledger_reservations.py:78-102`).

**Impact:** two seats can concurrently reserve the same task while capacity remains and both can send it. That defeats the send-once recovery rule, double-charges quota, and can produce duplicate or conflicting GitHub writes.

**Action:** make task ID a ledger-wide idempotency key across active reservations and completed/uncertain outcomes. A repeat should return the existing record without authorizing another send. Add same-task/same-seat and same-task/two-seat race tests.

### F4 — Medium — the authoritative GitHub task card is not required to be pinned to its own immutable commit

**Requirement:** dispatch instructions must be committed to GitHub before the conversation, and GitHub is the authoritative handoff (`direction document:78-82`; `SKILL.md:14`).

**Evidence:** the skill requires a base SHA and task-card path but does not require a separate full SHA/blob identity for the task card itself. This trial needs two different immutable identities: task definition `3c65372c4842ed01291f1caefb205334f4d0e1b6` and reviewed candidate `29f7f434c7d24893edc25861f3c8de7b5f4d7193`. A single generic “base SHA” cannot unambiguously pin both.

**Impact:** a starter that names only a mutable branch plus path can lead the web seat to read changed instructions after dispatch while still reviewing the intended candidate SHA. The supposedly authoritative GitHub handoff then lacks immutable instruction identity.

**Action:** require two explicit fields: `task_card_ref` (full commit SHA plus path, optionally blob SHA) and `review_or_construction_base_sha`. The pre-send check must verify both exact objects. Add a static task-card/starter validation example.

### F5 — Medium — the reservation lifecycle cannot durably represent sent, uncertain, or recoverable work

**Requirement:** after one click, an uncertain submission must not be resent; it must be preserved, inspected read-only, marked `uncertain_submission`, and returned safely (`SKILL.md:26`). Conversation links remain local (`direction document:89-92`).

**Evidence:** the CLI exposes only reserve, release, and complete for a reservation (`ledger.py:107-127`). A reservation stores only tier, task, seat, and reservation time (`ledger.py:138-140`). The conversation URL and outcome are written only by `complete`, after a reply (`ledger.py:146-160`). `status` reports aggregate counts, not reservation IDs, task lookup, conversation locator, or lifecycle state (`ledger.py:132-133`). There is no operation for `sent`, `uncertain_submission`, `stalled`, or `needs_login`.

**Impact:** a crash, lost tab, or tool failure after Send but before completion leaves an unsent reservation and a sent prompt indistinguishable in durable state. The operator cannot safely decide between release, recovery, and completion from the supported interface; capacity can remain stranded, and the missing recovery record increases resend risk.

**Action:** use an explicit local state machine such as `reserved -> sent/uncertain -> completed/released`, record a local conversation locator before or immediately after the one allowed send, expose lookup by task ID, and prohibit release after a sent/uncertain transition without a documented reconciliation action.

### F6 — Medium — `reset-tier` can reopen a tier with impossible chronology

**Requirement:** reset requires an actually observed reset timestamp and reason (`SKILL.md:32`), and a switched/exhausted tier stays paused until reset.

**Evidence:** `parse_time` checks only for a timezone offset (`ledger.py:29-33`). `reset-tier` accepts that value, installs it as the reset boundary, removes the block, and records the reason without checking that the reset is not in the future or that it follows the block (`ledger.py:161-165`). `active_events` then discards every event before that boundary (`ledger.py:71-73`).

**Impact:** a future timestamp copied from a displayed “resets at” notice, or a simple date typo, immediately clears the block and makes prior usage disappear from availability calculations before the reset has happened.

**Action:** distinguish `reset_due_at` from `reset_observed_at`. Only an observed reset at or before `now`, and not earlier than the block/exhaustion event, may clear a block and advance the accounting boundary. Add future, stale, and valid reset tests.

### F7 — Low — the lock file grows by one byte on every command

**Evidence:** the lock is opened with append mode, then the handle is explicitly moved to offset zero and tested with `tell() == 0`; that condition is therefore true on every invocation, while append-mode writes still go to the end (`ledger.py:43-49`). The code writes one byte before taking the byte-range lock.

**Impact:** the lock file grows indefinitely and every read-only status call performs an unnecessary unlocked append. This does not by itself defeat the one-byte lock, but it is an avoidable long-run state defect.

**Action:** initialize based on actual file size before seeking, or create once with `w+b` and subsequently open `r+b`; do not write on every acquisition. Add a repeated-status lock-size test.

### F8 — Medium — CI passes the existing five cases but omits the safety invariants above

**Evidence:** the workflow runs Windows `unittest` discovery (`.github/workflows/web-chatgpt-subagent.yml:18-28`). The exact-SHA log proves these five cases executed: GPT-6 disabled, the 121st Sol Pro message blocked, a flagged switch blocks the requested tier, release restores capacity, and two distinct seats cannot both take one remaining slot. The committed tests are limited to those behaviors (`test_ledger.py:26-46`; `test_ledger_reservations.py:50-102`).

No test covers contradictory actual-tier/switch flags, actual-tier accounting, a limit without a reply tier, unknown reconciliation, duplicate task IDs, sent/uncertain recovery, reset chronology, or lock-file growth.

**Impact:** the green validation step is consistent with all findings F1-F7; it does not exercise the dispatch and accounting failure modes most likely to violate send-once or fail-closed behavior.

**Action:** add focused Windows tests for every ledger defect above. Browser model-label reliability, GitHub connector permission isolation, and end-to-end fallback should remain explicit trial checks rather than being mislabeled as unit coverage.

## Controls confirmed by this review

- Routine use remains disabled pending Owner enablement, and GPT-6 Pro is separately disabled.
- The skill requires a fresh non-Project conversation, a pre-send model/tier check, GitHub-only result retrieval, reviewed-SHA reporting, and post-write branch/path inspection.
- It explicitly preserves existing high-risk audit slots, formal gates, external final review, Owner merge authority, and the distinction between an instruction boundary and a proven connector permission sandbox.
- Ledger state, account-use details, reset timestamps, and conversation links are directed to `%LOCALAPPDATA%` and prohibited from the public repository.
- The Windows locking test demonstrates that two distinct seats do not both take the final capacity slot in the tested case.

These controls do not cancel the findings or turn this trial review into acceptance.

## Unverified trial questions

1. **Actual browser model identity:** no immutable GitHub artifact independently proves the composer tier selected for this dispatch or the model actually serving every reply. The requested GPT-5.6 Sol Pro context and the web seat's own report are trial evidence, not independent attestation.
2. **Browser behavior:** login lifetime, account/foreground identity handling, reply latency, the five-minute stall rule, the approximately twenty-minute observation rule, and recovery after a lost tab remain unverified end to end.
3. **Fallback behavior:** the reviewed tree does not independently prove the complete `deadline_expired`, browser failure, limit-only, mismatch, and Codex-subagent handoff paths with durable local records.
4. **Provider rules and limits:** the automated-web terms question, unpublished tier limits, and actual reset-window semantics remain open exactly as listed in direction §9.
5. **Permission isolation:** one compliant result write can demonstrate assigned-branch access, but it cannot prove that the GitHub connector is technically unable to write another branch or path. The skill correctly labels these restrictions as instructions rather than a permission sandbox.
6. **Final CI/acceptance:** exact-SHA raw logs show five passing tests, but the overall run/job conclusion is `cancelled`, and the tests do not cover F1-F7. A non-cancelled run on the eventual candidate, nonzero executed tests, log review, focused regression coverage, and the project's separate acceptance path are still required.

## Disposition

Actionable implementation and handoff defects were found. No acceptance verdict is issued. The reviewed commit remains a draft trial candidate; formal enablement, routing adoption, high-risk acceptance, merge, and release remain outside WCS-05.