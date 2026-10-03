# B2.1 references

Existing implementation: `gogo-party/packages/room/src/accounts.ts` (`PROVIDER_LOGIN`, `loginEnv`, `AccountStore.startLogin/probe`), `packages/seat-runtime/src/claude-seat.ts`, and `packages/room/public/index.html`. NaveHQ Claude isolation notes and LoomOS Claude Code pattern notes were read as history only; neither provided runnable adapter code.

Repository research: `docs/research/2026-09-26-effect-decomposition-and-coverage.md`, `2026-09-26-execution-layer-capability-table.md`, `reuse-blueprint.md`, `source-audit/06-runtime-protocol-and-adapter-sources.md`, and `adapter-spike/01-capability-evidence.md` / `02-mvp-adapter-decision.md`. Paseo, Claudexor and Omnigent code was not copied because the documented behaviors do not supply real Claude CLI wire evidence and their session types are runtime-coupled.

Official sources: [CLI reference](https://code.claude.com/docs/en/cli-reference), [headless/Agent SDK CLI](https://code.claude.com/docs/en/headless), [authentication](https://code.claude.com/docs/en/authentication), [memory](https://code.claude.com/docs/en/memory), [settings](https://code.claude.com/docs/en/settings), and [SDK permissions](https://code.claude.com/docs/en/agent-sdk/permissions). The CLI pin is 2.1.286, an exact release identified by the official headless guide. Official auth docs describe browser login, `/login`, `/status`, and config-directory separation; `auth login/status` command spellings come from the prior `PROVIDER_LOGIN` implementation and remain unverified on the pinned CLI.

Implementation details and integration limits are recorded in the adapter's `REFERENCES.md` and `SHARED_INTEGRATION.md`.
