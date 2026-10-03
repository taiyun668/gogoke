# M2 provider command encoding references

This unconnected Rust file encodes vendor stdin data only. The native H owner must supply the observed pinned executable, process and session generation, current grant, original RPC ID and journal, framing write, and later origin bound stdout capture. An encoded line or successful stdin write is no vendor receipt. `session/cancel` is a notification; a correlated prompt response with `stopReason=cancelled` is required even to claim the model turn was cancelled, and H separately proves process stop.

## Direct shape sources

| Pin | Frozen source used | Encoded here | Deliberate limit |
| --- | --- | --- | --- |
| Claude Code 2.1.196 | `third_party/t3code/apps/server/src/gogoke/adapters/claude/{protocol,adapter}.ts` | `type:user` / `message.role:user` / text content JSONL; `--print --input-format stream-json --output-format stream-json --verbose`, optional explicit `--resume` | No in-turn append receipt, CLI permission callback wire contract, control response or steer. The historical `packages/seat-runtime/src/claude-seat.ts` callback was specifically rejected by the frozen adapter's `REFERENCES.md`. |
| OpenCode 1.18.32 | `third_party/t3code/apps/server/src/gogoke/adapters/opencode/{adapter,protocol}.ts` | ACP v1 initialize, new, resume, prompt, cancel; `acp` launch arg | Resume needs the caller's current capability evidence. Pinned success may omit `sessionId`. No `session/load` or steer in the frozen adapter. |
| Grok Build 1.0.41 | `third_party/t3code/apps/server/src/gogoke/adapters/grok/session.ts` | ACP v1 initialize, new, advertised load, prompt, cancel; minimal `agent stdio` launch args | Load is encoded only when H passes an observed advertised capability. No separate `session/resume` request or in-turn steer. The former gogo-party seat's model/sandbox flags are not a default authority template. |
| Antigravity CLI 1.2.11 | `third_party/t3code/apps/server/src/gogoke/adapters/antigravity/protocol.ts` | `event:user` / `message.content` JSONL; headless stream-json args, optional explicit `--conversation` | No guessed `role` field, `--continue`, native cancel, permission response, or in-turn steer. |

The ACP JSON-RPC ID is preserved as its native string or safe integer type via `provider_evidence/acp.rs::RpcId`; `atomic::JsonString` and `Json::canonical()` escape fields before the terminal LF. `PermissionResponse` returns `Unsupported` because the frozen ACP adapters do not encode a reply to `session/request_permission`. Only H can bind a server permission request to a current grant and an original request ID; this file cannot authorize it.

## Other references checked

- Archived gogo-party `packages/seat-runtime/src/{claude-seat,grok-acp-seat}.ts` provided prior CLI patterns. The frozen adapter sources above win where the historical runtime and current evidence differ.
- `docs/research/adapter-spike/{01-capability-evidence,02-mvp-adapter-decision}.md`, `docs/research/2026-09-26-execution-layer-capability-table.md`, and `docs/research/2026-09-26-antigravity-cli-facts.md` distinguish source, researched and live observed capability.
- NaveHQ's worker runtime and provider preflight notes and LoomOS's agent pattern notes were inspected read only. Neither provides a more direct fixed version wire encoder than the frozen adapter sources.
- Fixed official source pointers and limits are recorded in the frozen adapters' `REFERENCES.md` and `PROBE.md` files: OpenCode `v1.18.32` ACP service, official ACP guide; Claude Code CLI/headless reference; Antigravity CLI headless reference and 1.2.11 release. No CLI was started or upgraded for this package.

`commands.rs` is not included from `mod.rs` in this package, so cloud compilation and real installed product behavior are **NOT_RUN** here. The Controller owns shared integration and all final authority checks.
