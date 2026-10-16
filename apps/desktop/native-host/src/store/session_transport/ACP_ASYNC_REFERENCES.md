# Native ACP prompt completion references

The H User request remains the original K-SESSION `send` bytes in the existing stdin journal. The ACP command is a separate, correlated original stdin line in the existing RPC journal. A captures the real child stdout frame before either journal consumes its `RawSourceKey`. A prompt response, including a remote JSON-RPC error, is the terminal delivery fact; `session/update`, a successful stdin write, and `session/cancel` are not.

## Direct references

- Historical gogo-party `packages/seat-runtime/src/grok-acp-seat.ts` showed the host-owned ACP stdio request and update flow. It did not supply native H authority or the current A source binding.
- The frozen `third_party/t3code/apps/server/src/gogoke/adapters/opencode/{adapter,protocol}.ts` and `grok/session.ts` establish the pinned initialize, new, advertised resume/load, prompt, and cancel shapes. `provider_evidence/COMMANDS_REFERENCES.md` records their limits. The latter adapters cannot replace this H transaction or grant check.
- Repository research in `docs/research/adapter-spike/` and `docs/research/2026-09-26-execution-layer-capability-table.md` separates protocol capabilities from an installed and observed process.
- Official [ACP prompt turn](https://agentclientprotocol.com/protocol/v1/prompt-turn) defines the prompt response as the end of a turn and session updates as intermediate notifications. The pinned [OpenCode 1.18.32 ACP source](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/acp/service.ts) emits `end_turn`, `cancelled`, `max_tokens`, `refusal`, or JSON-RPC service errors from that response path. Grok's frozen official source and the repository's pinned adapter are shape references; no live CLI or model session was started for this package.

The runtime entry must still recheck `LaunchEvidence::verify_live` and the physical grant immediately before writing. This module checks the same verified database, H claim/seat/instance/home/custody, original User bytes, and A capture; it neither admits a new permission nor interprets vendor text as a K-SESSION StopFact. Cold candidate resume and Windows 11 installed-product behavior remain separate gates.
