# Claude adapter references

## Existing implementations checked first

- `gogo-party/packages/room/src/accounts.ts`: `PROVIDER_LOGIN.claude` uses `claude auth login --claudeai` and `claude auth status`; the host starts login only on an explicit action, keeps the child process and progress, and never opens the credential file. Its isolated environment removes inherited Claude harness variables and secrets. Reused as login-command and environment guidance.
- `gogo-party/packages/seat-runtime/src/claude-seat.ts`: long-lived `-p --input-format stream-json --output-format stream-json` process, `system/init` session identity, newline JSON user inputs and `result` turn terminal. Reused as a shape reference only. Its `--permission-prompt-tool stdio` AskUserQuestion handler and mid-turn delivery claims were not accepted as evidence because the current official CLI documentation does not document that wire callback or a correlated delivery receipt.
- `gogo-party/packages/room/public/index.html`: inspected for the existing instance/login UI flow; no UI changes are in B2.1.
- NaveHQ history: checked the Claude isolation and worker runtime notes. They recommend per-worker HOME/config-root isolation and treat gateways as routing rather than isolation; no runnable Claude adapter was reused. LoomOS history: checked the Claude Code agent-pattern notes; no runnable adapter source was found.

## Repository research and decomposed projects

- `docs/research/2026-09-26-effect-decomposition-and-coverage.md`: the relevant effects are role/session separation, exact-turn message delivery, recoverable session lifecycle, and explicit capability evidence.
- `docs/research/2026-09-26-execution-layer-capability-table.md`: records the archived seat-runtime as a prior implementation and distinguishes it from current gogoke code. Its Paseo/Agent SDK capability claims are research inputs, not CLI conformance evidence.
- `docs/research/reuse-blueprint.md` and `docs/research/source-audit/06-runtime-protocol-and-adapter-sources.md`: use adapter-local vendor protocol types and normalize them at the adapter boundary. Claudexor and Omnigent are semantic references; their session managers and provider-specific code are coupled to their own runtimes and were not copied.
- `docs/research/adapter-spike/01-capability-evidence.md` and `02-mvp-adapter-decision.md`: preserve `UNKNOWN` until a public binary is observed; do not infer steering or conformance from source presence.

## Official Claude Code sources

- CLI reference: https://code.claude.com/docs/en/cli-reference
- Headless / Agent SDK CLI: https://code.claude.com/docs/en/headless
- Session continuation: https://code.claude.com/docs/en/headless#continue-conversations
- Authentication and per-instance `CLAUDE_CONFIG_DIR`: https://code.claude.com/docs/en/authentication
- Memory controls: https://code.claude.com/docs/en/memory#enable-or-disable-auto-memory
- Settings and `CLAUDE_CONFIG_DIR`: https://code.claude.com/docs/en/settings
- SDK permissions and input callbacks: https://code.claude.com/docs/en/agent-sdk/permissions

The adapter pins the actually installed 2.1.196, as checked against its manifest and PE version. The first source package selected 2.1.286 from the headless guide's corrected `--bare` behavior; B2 does not use `--bare`, so that reference does not require an upgrade or identify the tested binary. No CLI was modified. Auto memory is requested off with the documented `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` environment variable, and the host must supply an isolated instance home/config directory; effective behavior remains unverified until the real fixed CLI run.

Official CLI documentation establishes stream-json input/output, `--resume`, stream result events, automatic context management, and the isolated auth/config directory. The historical `auth login`/`auth status` subcommands are taken from `PROVIDER_LOGIN`; the current official auth guide documents browser login, `/login`, and `/status`, so actual command behavior must still be verified on the pinned executable. The docs do not provide a CLI callback wire contract for native question cards or a host-correlated manual compaction receipt. Those capabilities remain unavailable/unverified here and use no guessed wire messages.
