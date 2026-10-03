# Provider login recipe references

`provider_login.rs` defines fixed login intent. `provider_login_preparation.rs`
now turns that intent into native process requests from the same verified
database, RootLock, and OwnerIssuer used by F. H owns the ordinary same-user login process, its complete raw
stdout/stderr, cancellation and custody. Model sessions remain under the
existing LPAC admission boundary. These recipes neither read credential data
nor start a process, open a browser, change a CLI installation, or modify user
environment configuration.

## Native preparation and H handoff

`provider_login::prepare_registered_provider_login(db, root, owner, instance_id)`
returns `LoginPreparation::Ready(PreparedProviderLogin)` or an explicit
`Unsupported` result. It first verifies the Owner product identity, F's
applied registration journal and physical receipt, the registered driver and
home marker/identity, and the exact fixed executable through the native
catalog. Neither the path, driver, version, hash, nor home comes from a login
request. The returned `login` is an actual `PrepareRequest` with an absolute
application path, registered SHA-256 `NativeBinding`, instance revision, and a
complete finite environment. There is no `PATH` or inherited provider secret
environment. The host's original `APPDATA`/`LOCALAPPDATA` stay in the ordinary
user login child; `HOME`/`USERPROFILE` and provider selectors target the
registered instance root. `TEMP`/`TMP` also target that root. The host must
re-resolve F and let ProcessCustodian recheck program/image bytes at actual
dispatch; this preparation is only a snapshot, not process custody.

An exceptional PREPARED/activation/ACTIVE-record failure retains the original
driver, registered home identity, host-owned runtime identity, and native
custody. After the original STOPPED proof is durable, cleanup revalidates that
same provider's F home before removing only `gogoke-login-runtime`. The Codex
path keeps its existing Windows cache-junction handling. A changed home or
runtime identity fails closed and does not broaden deletion.
If that fixed runtime cannot be removed after the STOPPED confirmation, the
original in-memory `PendingAccount` stays unsettled with the raw cleanup error.
A later status action on the same request revalidates the same home/runtime and
retries cleanup without preparing or launching another child. A new request
cannot replace that custody while cleanup remains pending.
The ordinary confirmed STOPPED path retains that same released custody if
runtime cleanup fails: same-request status/cancel only retries the original
directory after checking its identity. Neither path reissues login or status
while cleanup is pending. The original Windows error remains in the private
output; Final is emitted only after cleanup succeeds.

`status` is a separate `StatusObservation`. Claude's fixed `auth status --json`
requires both a documented exit code and matching boolean `loggedIn`; any
mismatch is Unknown. OpenCode's fixed `auth list --pure` lists local credentials.
The observed empty-home output, exit 0 and `0 credentials`, establishes
LOGGED_OUT without a host read of `auth.json`. Nonempty output alone stays
Unknown because its display name does not identify the provider ID. The
original pinned `openai` browser callback instead stores the OAuth entry
before printing Clack's complete LF-terminated `Login successful` line. Only
that original custodied line together with durable STOPPED exit 0, no
cancellation/capture failure, and the same F pin/home records
`CREDENTIAL_PRESENT_NO_VALIDITY_CHECK`; exit zero alone never does. A later
ambiguous inventory leaves that durable observation intact, while the exact
empty `0 credentials` inventory records LOGGED_OUT. Grok has no evidenced
independent status command and remains Unknown. Antigravity remains Unsupported
because its Windows keyring is shared. Codex retains its existing app-server path.
The status CLI is separately custodied. Its final stderr tail is drained and
captured before durable release, then included with the original exit code in
the User-private error when a status result is unclassified due to CLI failure.
Claude's documented exit 1 with `loggedIn:false` remains LOGGED_OUT, even
though stdout may end at EOF without another LF frame. No status stderr or
account fields enter the public instance journal.

The caller must have established a private User-origin action. Preparation
does not open a browser, read credentials, start a process, persist a result,
or perform a model turn. The native User runtime owns custody, progress,
cancellation, status polling, physical revalidation, and durable observation.
Root owns the Tauri browser guard. OpenCode's fixed browser method skips the
CLI menus. All providers' actual Win11 browser/Smart App Control outcomes
remain unverified until the
installed product runs the fixed bytes. These fixed CLI children are new
processes under Smart App Control; the installed main process being allowed
does not establish their launch result.

## Fixed recipes and limits

| CLI pin | Recipe | Environment intent | Browser and completion |
| --- | --- | --- | --- |
| Claude Code `2.1.196` | `claude auth login`. Anthropic's official `v2.1.41` release introduced `auth login` and `auth status`, before the fixed `2.1.196` pin. The prior Room `--claudeai` flag is omitted because the fixed-version source did not verify it. | `HOME`, `USERPROFILE`, and `CLAUDE_CONFIG_DIR` use the registered instance home, matching H's model launch selector. `APPDATA` and `LOCALAPPDATA` retain the Owner user's original values for the same-user browser context. | Official docs say login may open the default browser. No account-login browser-suppression switch is evidenced; `mcp login --no-browser` is for MCP OAuth and does not apply. H must not auto-open an output URL. `claude auth status --json` is the independent read; only matching documented exit 0/`loggedIn:true` or 1/`loggedIn:false` is classified. Do not parse other account fields. |
| OpenCode `1.18.32` | `opencode auth login --pure --provider openai --method "ChatGPT Pro/Plus (browser)"`. Fixed help and tagged source show both selectors skip terminal menus. `--pure` skips external plugins while retaining the built-in OpenAI auth plugin. | `HOME` and `USERPROFILE` use the registered instance home. Set `XDG_CONFIG_HOME=.config`, `XDG_DATA_HOME=.local/share`, `XDG_CACHE_HOME=.cache`, `XDG_STATE_HOME=.local/state`, `OPENCODE_CONFIG_DIR=.opencode`, and `OPENCODE_CONFIG=.opencode/opencode.json`, all beneath that home, matching H's model launch selectors. Preserve original `APPDATA` and `LOCALAPPDATA` for the browser context. | The selected built-in method prints a complete `Go to:` authorization URL, waits on localhost:1455, and does not call a browser opener. The User host may open that exact printed URL once. The fixed callback stores the local OAuth entry before its exact complete success line. That original line plus durable successful stop proves local credential presence; `auth list --pure` independently proves the observed empty inventory. No remote token validity is claimed. |
| Grok Build `1.0.41` | `grok login --oauth`. Exact fixed-binary `--help` evidence confirms this command and OAuth option. This is command-shape evidence only; the runtime flow is not qualified. | `HOME`, `USERPROFILE`, and `GROK_HOME` use the registered instance home, matching H's model launch selector; preserve original AppData for browser context. | Browser-opening behavior and completion remain unknown. The help exposes no `--no-browser` or status flag. H must not auto-open an output URL or infer login success from process exit; status detection is Unsupported. |
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
candidate is an authorization endpoint. It must stay in volatile host state
and must not enter durable evidence or logs. The OpenCode recipe separately
declares `HostOpensPrintedAuthorization`; Root's Tauri guard must match the
complete printed authorization URL and open it once. Claude may open its own
browser, so its output URL remains a manual fallback only. Grok behavior is
Unknown. These cases cannot share a generic output-URL auto-opener.

No recipe treats a zero login exit code, an output substring, or host-observed
file presence as login completion. OpenCode's fixed callback provides a narrow
original-process completion contract; other state comes from a separately
custodied fixed CLI status command. Raw stdout/stderr stays on the User-private path and
must retain the original failure reason. Unfinished stdout fragments may be
displayed but do not become process frames, status evidence, or durable facts.

## Sources read

- Historical Room implementation: `gogo-party/packages/room/src/accounts.ts`
  (`PROVIDER_LOGIN`, `loginEnv`, `AccountStore.startLogin/probe`). It is a
  precedent for keeping a session's progress and failure output available
  after the CLI exits. This native flow instead retains the original durable
  STOPPED custody and runtime identity until cleanup succeeds; Room's
  replacement/cancel behavior, Claude command spelling, Grok `--oauth`, and
  status heuristics are not treated as proof for these fixed executables.
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
  `opencode auth login` and terminal-auth metadata. More direct fixed-tag
  sources are [providers.ts](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/cli/cmd/providers.ts),
  [Auth.Service](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/auth/index.ts),
  [OpenAI browser plugin](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/plugin/openai/codex.ts),
  [plugin loader](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/plugin/index.ts),
  and [CLI flag parser](https://github.com/anomalyco/opencode/blob/v1.18.32/packages/opencode/src/index.ts).
  `providers.ts` skips provider/method prompts for exact selector labels,
  prints `Go to:` and locally lists credential display names and types;
  `codex.ts` uses localhost:1455 callback and contains no browser opener;
  the fixed plugin loader retains the built-in OpenAI plugin under `--pure`.
  Ordinary-view fixed executable SHA-256
  `cf664aa1da32b788f9b2699b84a9bb9be30b7e025693b90f9b85829d5fe4e252`
  independently showed `--provider`, `--method`, and `--pure` in `auth login
  --help`. In a fresh isolated home, `auth list` exited 0 with exactly `0
  credentials` and no host credential read. This proves a negative local
  inventory only; no login, positive OAuth entry, model turn, or browser flow
  was exercised.
  The fixed source's browser callback calls `Auth.set(openai, oauth)` before
  printing `Login successful`. `Auth.Service.set` awaits `writeJson` to the
  CLI's own local data path before returning; the host never reads that file.
  The pinned `@clack/prompts@1.0.0-alpha.1`
  [spinner](https://github.com/bombshell-dev/clack/blob/aece08386ee630a3b5d888460fe0028fc05dfe05/packages/prompts/src/spinner.ts)
  and [symbols](https://github.com/bombshell-dev/clack/blob/aece08386ee630a3b5d888460fe0028fc05dfe05/packages/prompts/src/common.ts)
  identify the exact LF line with `◇` or `o`, optionally green CSI color.
  The host recognizes only that complete line from the original prepared
  process after STOPPED exit 0 and no cancellation/capture failure. It records
  the same local-presence semantics as Codex's
  `CREDENTIAL_PRESENT_NO_VALIDITY_CHECK`, not remote token validity. An
  independent `auth list` positive parser remains Unknown because its display
  name does not identify the provider ID. Actual Owner Windows positive
  output and browser/SAC outcome remain NOT_RUN at the stable point.
- Grok official source snapshot:
  [authentication guide at source-audit commit](https://github.com/xai-org/grok-build/blob/b13fa526f5112c0b20dad5f1f2300d3d3b127895/crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md).
  B4 records the installed `1.0.41` version observation but states the source
  snapshot and installed executable are not byte-equivalent. The exact help
  observation below supersedes this source for command syntax only; the guide
  does not establish runtime browser or completion behavior for the fixed CLI.
- Owner-provided ordinary-view help observation for the fixed Grok binary:
  version `1.0.41`, SHA-256
  `ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25`, private
  original evidence name `grok-login-help-original.json` (not committed); the
  before/after executable hashes matched this pin.
  Its short excerpt is `Usage: grok login [OPTIONS]`; `--oauth Use Grok OAuth
  via auth.x.ai`; `--device-auth` selects device-code authentication. The same
  help output exposed no `--no-browser` or status option. This read-only help
  observation proves only the parser's displayed command/option surface: no
  login, browser authorization, credentials, or model request occurred, and it
  does not prove URL printing, browser launching, or completion semantics.
  The published GitHub source snapshot and the current changelog are not
  proven byte-equivalent to this `1.0.41` executable. Its fixed `login --help`
  exposes `--oauth` but no independent account-status result or exit contract.
  Keep Grok state Unknown until an exact fixed-version CLI-owned status or
  login-completion result is captured from an Owner-authorized isolated login;
  a host read of `auth.json` or a model request is not a substitute.
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

Per `docs/governance/gogoke-build-and-release.md`, native compilation and
tests run in cloud CI only. The User runtime is integrated in the native module
tree; the latest changed SHA still needs its own cloud result. No login, model
request, browser, local native build, signing, or installation is included.
