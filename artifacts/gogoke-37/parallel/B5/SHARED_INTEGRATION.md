# B5.1 shared integration request

This package owns only the Antigravity adapter directory. The following work is required in Controller-owned shared paths before any real seat is admitted.

## H / F launch and identity

- Resolve only the F-registered absolute executable whose observed binary identity is pinned to official `agy` 1.2.11 (`google-antigravity/antigravity-cli`, tag `1.2.11`, release commit `6dadd62`). Do not resolve an arbitrary `agy` through inherited `PATH`.
- Pass H's measured pinned version to `AntigravityStreamAdapter`; it rejects a version mismatch. Native H still owns binary identity, LPAC, process creation, stdout/stderr, and process-tree custody. LPAC is a hard precondition for every model process.
- Use the adapter's `antigravityInstanceEnvironment` with the F instance home and H-approved base environment. Keep the canonical, F-verified worktree as child `cwd`; do not inherit the parent process's working directory.
- Do not interpret local home isolation as account isolation. The current official authentication guide says Windows CLI authentication uses Windows Credential Manager when a saved session exists. The reviewed official materials expose no per-instance keyring override. Keep login/account state `UNKNOWN` until a separately authorized host observation supports a stronger claim; do not read keyring material or credentials.

## K-SESSION / A protocol wiring

- H launches `agy --input-format stream-json --output-format stream-json` and writes each user event to stdin. For a continuation, pass the exact persisted conversation ID through `antigravityHeadlessArgs(id)`. The adapter intentionally offers no `--continue` or new-conversation fallback.
- Feed stdout bytes through `AntigravityStreamAdapter.acceptStdout`; preserve each raw vendor event and tag it with the trusted K-SESSION seat, generation and request before A normalizes it. Persist the first `init.conversation_id` only through the existing durable H/K-SESSION owner.
- A successful stdin write is `sent-not-settled`; only the correlated raw `result` event can inform H's delivery settlement. A `result` event or adapter EOF does not prove that the process tree stopped.
- While a turn is active, `submitPrompt` refuses another message with `TURN_ACTIVE`; no in-turn steer is advertised. To interrupt, H must seal new admission, stop the exact child using the native custody chain and record the trusted stop receipt. Only then may it resume the same conversation ID with the same driver, instance, pinned binary and retained admission, and send the pending message. Unknown stop/resume outcomes stay `UNKNOWN`; never start a new conversation or resend under a fresh request ID.
- The 1.2.14 `queuedMessages=send-immediately` feature is outside the 1.2.11 pin and does not prove headless in-turn interruption. Do not wire it as a fallback.

## Memory, rules, and login

- The helper redirects HOME/USERPROFILE and AppData to the F instance home. This isolates local Antigravity CLI state only. K-SESSION must map one conversation ID to its existing project/seat binding and must never infer cross-project isolation from HOME.
- The CLI has no verified memory-off option in this evidence set. Keep `memoryOff=UNSUPPORTED`; vendor/account-side history isolation is `UNKNOWN`.
- Global rules may be found below the isolated `~/.gemini` home. Workspace `AGENTS.md`, `GEMINI.md`, and `.agents/rules/` may still be loaded cumulatively. Do not copy, synthesize, or suppress rules in this adapter; F/H own workspace and instance-home policy.
- A future login entry must be an explicit Owner action on the host's instance page. The host owns the process, visible progress, original failure reason and automatic login-state reconciliation. Vendor authorization must occur in the Owner's visible browser page; the adapter neither launches login nor handles credentials. No login was run for this package.

## Evidence still required

- Real protocol golden output and end-to-end run against the exact registered 1.2.11 binary, with no login or model turn during this adapter-only phase.
- H's production LPAC launch, raw stderr settlement, confirmed stop receipt and same-ID interrupted-session recovery.
- Exact fixed-version rule-file behavior and an authorized account-isolation decision/measurement before any claim beyond local-home scoping.
