# Provider login recipe references

This file and `provider_login.rs` define data-only login intent for the fixed
M2 CLI pins. H owns the ordinary same-user login process, its complete raw
stdout/stderr, cancellation and custody. Model sessions remain under the
existing LPAC admission boundary. These recipes neither read credential data
nor start a process, open a browser, change a CLI installation, or modify user
environment configuration.

## Fixed recipes and limits

| CLI pin | Recipe | Environment intent | Browser and completion |
| --- | --- | --- | --- |
| Claude Code `2.1.196` | `claude auth login`. Anthropic's official `v2.1.41` release introduced `auth login` and `auth status`, before the fixed `2.1.196` pin. The prior Room `--claudeai` flag is omitted because the fixed-version source did not verify it. | `HOME`, `USERPROFILE`, and `CLAUDE_CONFIG_DIR` use the registered instance home, matching H's model launch selector. `APPDATA` and `LOCALAPPDATA` retain the Owner user's original values for the same-user browser context. | Official docs say login may open the default browser. No account-login browser-suppression switch is evidenced; `mcp login --no-browser` is for MCP OAuth and does not apply. H must not auto-open an output URL. `claude auth status` is the documented read; use only its documented exit contract (0 logged in, 1 not logged in; other codes unknown). Do not parse later-added `configDirectory` output fields on `2.1.196`. |
| OpenCode `1.18.32` | `opencode auth login`, as recorded by B3's integration contract and the official CLI/ACP docs. | `HOME` and `USERPROFILE` use the registered instance home. Set `XDG_CONFIG_HOME=.config`, `XDG_DATA_HOME=.local/share`, `XDG_CACHE_HOME=.cache`, `XDG_STATE_HOME=.local/state`, `OPENCODE_CONFIG_DIR=.opencode`, and `OPENCODE_CONFIG=.opencode/opencode.json`, all beneath that home, matching H's model launch selectors. Preserve original `APPDATA` and `LOCALAPPDATA` for the browser context. | No exact-version URL or browser suppression behavior is recorded. H must not auto-open output URLs. Exit status alone is not login proof; no fixed-version status parser is included. |
| Grok Build `1.0.41` | **Unsupported.** The historical Room recipe and mutable official guide are not exact-version evidence for the installed executable. No `--oauth`, device-auth, or other login flag is emitted. | `HOME`, `USERPROFILE`, and `GROK_HOME` use the registered instance home, matching H's model launch selector; preserve original AppData for browser context. | **Unsupported** until the exact pinned executable's login command, browser behavior, and completion signal are evidenced. |
| Antigravity CLI `1.2.11` | The exact official tag says first launch of `agy` authenticates through system keyring and opens the browser if needed. **Unsupported as an instance login**: it exposes no documented login-only command, and the system keyring is shared across this Windows user. | Local home bindings cannot isolate the system keyring identity. | The CLI owns first-launch browser behavior; H must not open a duplicate URL. Account status stays `UNKNOWN`; do not turn a shared keyring session into per-instance readiness. |
| Codex `0.160` | Out of scope; existing Codex login path stays unchanged. | This module makes no Codex launch or environment change. | Existing `v37_login` owns its Codex browser URL and callback flow. |

`APPDATA` and `LOCALAPPDATA` have `PreserveUserValue` intent so browser profile
resolution remains in the same Windows user context. The host still constructs
the final explicit environment from its approved source; these descriptors do
not authorize inheriting arbitrary parent variables. The login process uses
the logged-in user token, while every model process continues to require its
existing LPAC route.

## Home-selector reconciliation

The H launch environment audited at commit `cc48ef5f` is the direct reference:
`apps/desktop/native-host/src/store/session_transport/launch.rs` sets Claude's
`CLAUDE_CONFIG_DIR` and Grok's `GROK_HOME` to the registered instance root. For
OpenCode it sets all four XDG directories below that root and sets
`OPENCODE_CONFIG_DIR` / `OPENCODE_CONFIG` beneath `.opencode`. The earlier login
intent removed `CLAUDE_CONFIG_DIR` and OpenCode selectors and pointed Grok at
`<instance>/.grok`; those differences could make a successful login write to a
different store than the model process reads. The recipes now carry the same
selector values for both login and any status command. This change is limited
to child-environment intent; it does not alter H or read credentials.

For login processes, `APPDATA` and `LOCALAPPDATA` still preserve the original
user values so a CLI-owned browser uses the user's existing browser context.
The provider-specific home selectors above control credential/config locations.

`https_url_candidate_for_manual_owner_display` extracts one syntactically
bounded `https://` candidate from already captured text. It does not prove the
candidate is an authorization endpoint. It must stay in volatile host state,
must not enter durable evidence or logs, and must never trigger an automatic
browser launch. The Owner may use it only as a manual fallback after the host
has not already caused the CLI's browser flow. The host owns the foreground
browser handoff and must avoid opening the same request twice.

No recipe treats a zero exit code, an output substring, or file presence as
login completion. `status_argv` remains absent until a fixed-version status
command and an unambiguous response contract are supported by evidence. Raw
stdout/stderr stays with H and must retain the original failure reason.

## Sources read

- Historical Room implementation: `gogo-party/packages/room/src/accounts.ts`
  (`PROVIDER_LOGIN`, `loginEnv`, `AccountStore.startLogin/probe`). It is a
  precedent only; its Claude command spelling, Grok `--oauth`, and status
  heuristics are not treated as proof for these fixed executables.
- Project cross-repository research: `docs/research/2026-09-25-own-history-rework.md`,
  `docs/research/2026-09-25-parts-source-teardown.md`,
  `docs/research/2026-09-26-kernel-parts-harvest.md`,
  `docs/research/reuse-blueprint.md`, and
  `docs/research/upstream-reference-map.md`. NaveHQ and LoomOS login/isolation
  history was read as design context; neither supplies the current native host
  process, browser, keyring, or completion contract.
- M2 adapter evidence and handoffs: `artifacts/gogoke-37/parallel/B2/REFERENCES.md`,
  `B3/SHARED_INTEGRATION.md`, `B4/SHARED_INTEGRATION.md`,
  `B5/SHARED_INTEGRATION.md`, and `F2/SHARED_INTEGRATION.md`.
- Claude Code official sources: [v2.1.41 release](https://github.com/anthropics/claude-code/releases/tag/v2.1.41),
  [CLI reference](https://code.claude.com/docs/en/cli-reference), and
  [Authentication](https://code.claude.com/docs/en/authentication). The exact
  official release record introduces `auth login` and `auth status`; the
  fixed `2.1.196` pin is later. The current CLI reference documents `auth
  status` exit codes 0/1 and JSON output. Its `configDirectory` field requires
  `2.1.268`, so this recipe does not depend on that field. The mutable current
  docs describe browser login and config-directory scoping; they do not show a
  browser suppression switch for account login.
- OpenCode official docs: [CLI](https://dev.opencode.ai/docs/cli/) and
  [ACP authentication](https://opencode.ai/docs/cli/acp/). They document
  `opencode auth login` and terminal-auth metadata. They do not establish the
  fixed binary's output transcript or a browser-open suppression switch.
- Grok official source snapshot:
  [authentication guide at source-audit commit](https://github.com/xai-org/grok-build/blob/b13fa526f5112c0b20dad5f1f2300d3d3b127895/crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md).
  B4 records the installed `1.0.41` version observation but states the source
  snapshot and installed executable are not byte-equivalent. The current main
  guide is mutable, so this does not qualify a login recipe for the fixed CLI.
- Google official sources: [Antigravity CLI `1.2.11` README](https://github.com/google-antigravity/antigravity-cli/blob/1.2.11/README.md)
  and [current installation/authentication docs](https://www.antigravity.google/docs/cli/install/).
  The exact tag documents first-launch keyring sign-in, local automatic browser
  opening, remote URL display, and `/logout`; it does not document a login-only
  subcommand. B5 records that no Windows per-instance keyring override is
  evidenced.
- Existing Codex host/browser flow: `apps/desktop/native-host/src/store/product_database/v37_login.rs`.
  It is unchanged and remains the reference for host-owned URL presentation,
  ordinary-user process identity, and callback custody.

## Build and validation boundary

The native module is included by the integrator in the authorized Rust module
tree. Per `docs/governance/gogoke-build-and-release.md`, native compilation and
tests run in cloud CI only. This package added no mirror unit tests and did not
run login, a model request, a browser, a local native build, or a credential
probe. Until the module is exported and the cloud job runs against its exact
commit, native validation is `NOT_RUN`.
