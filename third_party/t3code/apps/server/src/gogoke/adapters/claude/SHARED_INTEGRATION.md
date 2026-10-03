# Shared integration request: Claude B2.1

Controller-owned shared wiring remains required before this adapter can run in gogoke.

- H must launch the pinned 2.1.286 executable in LPAC only, with a current native admission receipt proving the requested permission tier. If it cannot prove LPAC or the tier, it must refuse; no ordinary-user model-process fallback is allowed. The adapter's `ClaudeHostBoundary` is an input contract, not proof by itself.
- F/H must provide the measured pinned executable/version and the instance-owned `HOME`, `USERPROFILE`, `CLAUDE_CONFIG_DIR`, temp and workspace paths. Do not read, copy or synthesize CLI credentials. The model-process environment must not inherit host Claude harness variables or secret environment entries.
- The adapter requests `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`; the effective runtime behavior has not been observed. The host must report this as configured, not verified, until a real pinned CLI run confirms it.
- C owns the K-QCARD fallback. B2.1 does not claim native question-card support from an undocumented CLI control message.
- H may expose the historical login command `claude auth login --claudeai` and status command `claude auth status` only behind an explicit user login action, bound to this instance's home/config directory. The host starts, tracks, cancels and reports the fixed CLI operation, routes the official URL to the existing host browser handoff once, retains the raw CLI failure, and determines login state from a CLI status result. The status subcommand must be checked against the pinned executable because the current official auth guide documents browser login, `/login` and `/status` instead. The host never reads credentials and never modifies the CLI installation or system settings. Login is a separate identity operation; it does not run a model request or weaken the model process's LPAC requirement.
- K-SESSION must preserve `UNKNOWN` after a stdin write without a correlated native receipt; a process exit is not a turn terminal receipt. `--resume` must use only the opaque session ID stored for the same seat/instance/generation.
- The first `startTurn` writes the input before waiting for `system/init`, following the existing gogo-party seat runtime. Native session identity remains unknown until the real init frame; do not synthesize it. Result `session_id`, `subtype` and boolean `is_error` are required, so missing fields cannot be accepted as a successful completion.
- No CI/workflow, package export, dependency, registry or product entry is changed in this task. Controller owns those shared changes.

## Verification still required

Real CLI golden protocol samples and end-to-end Host/K-SESSION/K-INBOX/K-QCARD wiring were not available to this task and are `NOT_RUN`; do not use generated fixtures as golden evidence. The exact pin must be checked against the actual installed binary before dispatch. Login, LPAC admission, effective memory-off behavior, native question cards, and compaction receipts remain runtime evidence items.
