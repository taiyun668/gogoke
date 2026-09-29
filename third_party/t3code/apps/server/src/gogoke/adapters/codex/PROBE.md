# Codex 0.149.0 app-server process probe

2026-09-29, Windows 11 Owner machine. This probes the installed npm Codex CLI in an empty disposable `CODEX_HOME`, with a child environment that excludes inherited API keys and auth tokens. The CLI was not logged in through this probe. No model turn, authentication request, approval response, or quota-consuming operation was sent.

The npm root package reported `@openai/codex` version `0.149.0`; the Windows platform package reported `0.149.0-win32-x64`. The observed `codex.exe` SHA-256 was `14b7e6b2356e82d1d9275579eaa588757b4e0a501b65dcc19fccdf77bd83dc00`; its `--version` output was `codex-cli 0.149.0`. The app-server `initialize` response contained a user agent identifying version `0.149.0`. This is a local process observation, not proof of H's future pinned launch or installed candidate behavior.

Run from `third_party/t3code/apps/server` with the signed Node 24 runtime:

```text
node src/gogoke/adapters/codex/processProbe.ts
node src/gogoke/adapters/codex/processProbe.ts --thread-probe
```

The launcher uses the explicit overrides `features.memories=false`, `memories.generate_memories=false`, and `memories.use_memories=false`, plus `--strict-config --stdio`. `config/read` returned all three values as `false` in effective configuration. The default probe completed `initialize` and `config/read`; it did not create a thread.

The optional no-turn probe completed `thread/start`, `thread/inject_items` with an empty-object ACK, then `thread/resume` for the same ID. No `turn/started` notification arrived in the one-second observation after append. An immediate `thread/resume` before append was also tried: app-server returned JSON-RPC code `-32600` with “no rollout found for thread id”. Resume succeeded after append. That sequence suggests a thread without persisted rollout cannot yet be resumed; the storage mechanism was not independently inspected.

| Operation | Evidence state |
| --- | --- |
| `initialize`, `config/read`, effective memory-off | EXECUTED / observed success |
| `thread/start`, `thread/inject_items`, `thread/resume` after append | EXECUTED / observed success in disposable home |
| Append does not start a turn | ACK received; no turn observed for one second only |
| Native `item/tool/requestUserInput` | NOT_RUN: requires a real model turn |
| `turn/steer` | NOT_RUN: requires an active turn |
| `thread/compact/start` | NOT_RUN: may invoke a model |
| H native Job custody, persistent installation, recovery, M1 | NOT_RUN |

Focused TS tests exercise the codec and state machine with fixtures. They do not replace this real-process observation, and this observation does not establish the later product or Owner acceptance gates.
