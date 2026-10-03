# Native provider output projection references

`normalize.rs` is a pure M2 handoff from A-captured stdout to `codex_output::NormalizedUpdate` shaped values. It has no caller in this branch: `provider_evidence/mod.rs` is integrator-owned. The Controller must register it, bind the exact H process/session/thread and pending ACP request, and keep the raw frame in A before calling it. No local native build or runtime test was run (`NOT_RUN`); the fixed candidate needs cloud compile and genuine pinned-binary evidence.

## Sources checked

- Historical implementation: `gogo-party/packages/seat-runtime/src/claude-seat.ts` and the archived Grok ACP seat path, as identified in `third_party/t3code/apps/server/src/gogoke/adapters/{claude/grok}` and `docs/research/2026-09-26-execution-layer-capability-table.md`. NaveHQ isolation notes and LoomOS hand/login patterns were reviewed through B2/B4 references; neither gives a trusted output frame for this converter.
- Repository research: `docs/research/2026-09-26-execution-layer-capability-table.md`, `docs/research/source-audit/06-runtime-protocol-and-adapter-sources.md`, `artifacts/gogoke-37/parallel/B2/REFERENCES.md`, B3/B4/B5 `SHARED_INTEGRATION.md`, and the four `third_party/t3code/apps/server/src/gogoke/adapters/{claude,opencode,grok,antigravity}` packages. The TypeScript packages provide protocol and uncertainty boundaries, not a second native session store.
- Fixed official references: [Claude stream-json headless guide](https://code.claude.com/docs/en/headless), [OpenCode v1.18.32 ACP source](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/acp/service.ts), [OpenCode ACP guide](https://opencode.ai/docs/acp/), [Grok Build fixed source snapshot](https://github.com/xai-org/grok-build/tree/b13fa526f5112c0b20dad5f1f2300d3d3b127895), and [ACP protocol](https://agentclientprotocol.com/protocol/overview). Current guides may describe later releases; the pinned version and real H-observed binary are separate evidence.

## Mapping and limits

| Source | Pure result | Boundary |
| --- | --- | --- |
| Claude `system/init` | session data | H must already bind the CLI-observed session before projection. |
| Claude `assistant` text blocks | `agent_message_chunk` | Text only. Stream events and tool blocks remain `Unhandled`, avoiding duplicate partial/final text and invented tool outcomes. |
| Claude `result` | terminal candidate with exact subtype and error flag | Does not prove delivery, process stop, or a successful task. |
| ACP `session/update` text chunk | `agent_message_chunk` or `agent_thought_chunk` | Rebuilt from text with H's provider/session/thread metadata; vendor `_meta` is discarded. Tool, usage, mode and future updates remain `Unhandled` with exact raw frame until their semantics are qualified. |
| ACP matching `session/prompt` response | terminal candidate with source stop reason | `cancelled` settles only the provider turn; it is not StopFact. The full result remains in A's raw frame. |
| ACP `session/request_permission` | data with typed JSON-RPC ID and raw params JSON | Neither permission grant nor response. H owns grant validation and any reply. |
| ACP RPC error | original error object and original frame | No replacement error string or success. |
| Unknown/unsupported | `Unhandled`/`Unsupported` | Pinned `agy` 1.2.11 remains unsupported in this output module. |

The parser in `stream_json.rs`/`acp.rs` is the first gate; this module cannot turn an invalid frame into a success. `NormalizedUpdate` is only a value. H/A must still create event IDs, source labels, cursor, and ledger input from the actual captured frame. Session creation and resume ACKs are data, not a substitute for native process custody or a confirmed old-generation stop. No runtime golden frame, credential, login, LPAC observation, Windows installation, acceptance, or release is claimed here.
