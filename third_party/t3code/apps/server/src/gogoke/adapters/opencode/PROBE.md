# OpenCode 1.18.32 adapter evidence

This adapter targets the immutable OpenCode `v1.18.32` release at source commit `545f51d` and ACP protocol version 1. It accepts only a host-observed version equal to that pin. A source match or scripted transport test does not prove an installed binary or real session.

| Capability | Report | Evidence in this package |
| --- | --- | --- |
| ACP initialize and session create | `SOURCE_PRESENT_RUNTIME_UNVERIFIED` | OpenCode tagged `acp/service.ts` advertises ACP v1, creates a persistent backing session, and returns its native session ID. |
| Hosted login handoff | `SOURCE_PRESENT_RUNTIME_UNVERIFIED` | ACP advertises `opencode-login` and, when the client requests `terminal-auth`, returns the official `opencode auth login` command. This adapter only returns that command descriptor; the host must execute it with the pinned binary and isolated environment. |
| Resume | `SOURCE_PRESENT_RUNTIME_UNVERIFIED` | `resumeSession` loads the exact requested ID and cwd. OpenCode 1.18.32 omits the ID in the success body; this adapter binds to the requested ID and accepts only the expected `configOptions` result. |
| In-turn steer | `UNSUPPORTED` | ACP exposes `session/cancel` and `session/prompt`, not a steer operation. The adapter rejects steer. |
| Interrupt and continue in same native session | `SOURCE_PRESENT_RUNTIME_UNVERIFIED` | Cancel is a one-way notification. The adapter waits for the active prompt response and confirms only `stopReason=cancelled`; then a new prompt may be sent in the same session. This is not proof that the process stopped. |
| Project instruction files | `SOURCE_PRESENT_RUNTIME_UNVERIFIED` | Tagged source searches upward for project `AGENTS.md` (with CLAUDE/CONTEXT fallback) and reads configured local instruction paths. Actual effective source paths have not been observed from a real instance. |
| Instance home isolation | `HOST_CONFIGURATION_REQUIRED` | `opencodeInstanceEnvironment()` assigns USERPROFILE/HOME, XDG config/data/cache/state, OpenCode config directory and an empty config-content override, and disables Claude compatibility. H must apply these values to its explicit child environment and omit inherited `PWD`. |
| Vendor memory off | `NOT_SUPPORTED` | OpenCode's persistent native session/history remains available. Home isolation prevents inherited global configuration paths; it does not disable session history or prove project instruction contents. |
| Real ACP golden sample and end-to-end | `NOT_RUN` | No genuine captured protocol sample is checked in or represented as a golden. No real login or model request was made. |

The source-only evidence is based on OpenCode's pinned [ACP service](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/acp/service.ts), [instruction loader](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/session/instruction.ts), and [config loader](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/config/config.ts). The official [ACP guide](https://opencode.ai/docs/acp/), [CLI auth guide](https://opencode.ai/docs/cli/), and [rules documentation](https://opencode.ai/docs/rules/) describe the public protocol, provider login command and instruction conventions; runtime claims remain pinned to the release source above.

`adapter.test.ts` exercises binding and stop-state safety through a scripted in-memory transport. Its response objects are test inputs, not captured OpenCode frames and not golden evidence.
