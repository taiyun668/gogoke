# B5 Antigravity credential and process isolation options

**Research snapshot:** 2026-10-06  
**Source base:** `799cb02b9681b422894644450d05dfcbb0674130`  
**Disposition:** research only; B5 remains `UNSUPPORTED` / `NOT_RUN`.

## Decision

No currently documented `agy` OAuth profile selector, auth-store selector, or
CLI data-root override establishes an independent Windows sign-in under the
same user. Redirecting `HOME` or changing a settings/data directory can select
home-relative files, but does not select a different Windows Credential
Manager logon session. The documented OAuth backend is the OS keyring.

The current product path therefore has no evidence that satisfies both existing
requirements: Owner performs an ordinary-user login, while the model process
runs under its existing LPAC identity. Windows documents AppContainer credential
isolation; `agy` documents that it obtains OAuth from the native keyring. No
documented `agy` broker transfers an isolated credential to an LPAC model
process. This is a source-backed unresolved compatibility gap, not a request
to change the process boundary. Keep B5 unsupported and unrun.

The task handoff reports that the fixed local `agy` executable path currently
has no executable. This investigation did not probe that path, run `agy`, read
credential state, install software, or open an account flow. The repository's
login recipe still pins `agy` 1.2.11, while Google's current download page lists
CLI 1.2.14. Neither that pin nor the current release listing proves which bytes
are installed or that their auth behavior matches. Do not silently replace the
pin or infer readiness from the newer release.

## What the plan and repository already require

Plan B5.1 is an Antigravity adapter deliverable for the headless protocol,
conversation resume, interrupt/resume, capability reporting, and memory
behavior at a pinned version. B5 depends on L0. The plan's host boundary keeps
ordinary-user login separate from model sessions under LPAC; changing that
boundary would change scope.

Current source already records the unresolved case: `provider_login.rs` pins
1.2.11, passes no `login` arguments, marks Antigravity `Unsupported`, uses the
common home environment, and explains that first launch owns browser sign-in
through the current Windows user's shared Credential Manager. The companion
reference says home bindings cannot isolate this store, the account state must
stay `UNKNOWN`, and the model remains on the existing LPAC route. The M2 runner
records Antigravity as `NOT_RUN_UNSUPPORTED_INSTANCE_LOGIN`; this is not a B5
pass.

The fixed 1.2.11 research record dated 2026-09-26 contains a real, version-bound
Win11 observation: `--mode plan` together with `--sandbox` still wrote to the
workspace. That observation is useful for rejecting those flags as an isolation
answer, but it is not current-binary identity, a credential-isolation test, or
LPAC evidence.

## Existing design precedents

- **GOGO PARTY `PROVIDER_LOGIN`:** `packages/room/src/accounts.ts` provides a
  separate home and vendor-owned login flow for Claude, Codex, and Grok. Its
  own comments say to select each CLI's actual home variable; Grok needed
  `GROK_HOME` rather than `HOME`. It contains no Antigravity recipe. These
  file/home-backed precedents do not isolate a Windows OS keyring.
- **NaveHQ:** the true-launch boundary requires worker-local `HOME`, XDG and
  OpenCode config roots, and stops if auth appears at a global path. This is a
  useful rule for file-backed CLI state, not evidence about `agy`'s Windows
  Credential Manager access. Its container strategy review also treats native
  provider login and attach behavior as a separate cost, not something path
  mounts solve.
- **LoomOS:** the searched decisions, lessons, and specs contained no
  `agy`/Antigravity, `PROVIDER_LOGIN`, keyring, or credential-profile precedent.
  No transferable Antigravity isolation behavior was found there.

## Candidate mechanisms

| Mechanism | What evidence supports | B5 assessment |
| --- | --- | --- |
| `HOME` or settings directory | Google's CLI docs put settings under `~/.gemini/antigravity-cli/settings.json`; project history/session files are home-relative. | Can select local settings/history. It does not select the Windows Credential Manager set associated with the process token. Insufficient for OAuth isolation. |
| CLI `--profile`, `--data-dir`, `--user-data-dir`, auth-store flag | Google's current CLI flag reference lists prompt, model, conversation, mode/sandbox-related flags; it does not document these auth selectors. Upstream issue #381 is still open and asks for a supported auth-profile/root selector; the issue is user-submitted evidence of a documentation gap, not a vendor implementation contract. | Unsupported/unverified. Do not route a B5 login through a guessed flag or Antigravity IDE launcher option. |
| `--mode=plan` / `--sandbox` | Google's headless documentation says active-workspace file reads/writes are auto-allowed; the fixed 1.2.11 direct research recorded a workspace write even with both flags. | Execution mode/sandbox flags are not an auth boundary and do not prove no-write behavior. |
| Windows Credential Manager | Google documents native-keyring OAuth. Microsoft's `CredRead` contract, if an app uses that API, reads from the set associated with the current token's logon session. | A home/profile path does not choose another logon session. Do not read, copy, rename, inject, or swap credentials to simulate a profile. |
| Separate Windows user/logon | Microsoft's docs define a token/logon-session credential set; AppContainer adds a distinct package identity. A third-party adapter for **Antigravity IDE 2.x** chooses an OS-user boundary and requires elevation on Windows. | Different identity/process boundary, not same-user B5. The IDE adapter is not the `agy` CLI; its documentation is not evidence for the fixed CLI. Out of current scope and requires an Owner decision before any future attempt. |
| Gemini API-key mode | Google's CLI docs require `modelProvider: "gemini"` plus `GEMINI_API_KEY` and say requests go directly to Gemini API without an account session. | Different provider/auth route, requires a secret and separate authorization, and is not Antigravity subscription OAuth. It cannot be counted as B5. |
| Third-party wrappers/proxies | Reviewed projects either switch the shared live keyring state, depend on a different OS/user boundary, or accept/manage token material. Some place a token in the agent environment or add a credential proxy. | Not an eligible substitute for the official pinned `agy` login path; introduces credential handling or changes the trust boundary. No wrapper CLI or proxy was built or run. |

The current official CLI reference lists `/logout`, which purges keyring
credentials, but no login-only command. The installation/auth guide describes
launching `agy` and letting it open the default browser if no valid keyring
profile exists. The headless guide says a prompt run uses cached credentials;
headless is an agent/model run, not a login-only probe. In this investigation,
there is no supported no-model authentication check to qualify.

Microsoft describes LPAC as more isolated than AppContainer and says
AppContainer credential isolation prevents using user credentials to log into
other environments. Microsoft's `CredRead` API contract is
token/logon-session based, not `HOME`-based; that contract does not prove the
fixed `agy` binary calls `CredRead` or uses it as its native-keyring backend.
This research did not measure an LPAC token's `AuthenticationId` or keyring
visibility. **Inference:** launching the same `agy` OAuth consumer under LPAC
cannot be presumed to read the ordinary user's keyring; launching it as an
ordinary process instead would fail the existing model-LPAC boundary unless
Google documents and the product proves a separate broker/model split. Neither
mechanism is present in the reviewed public contract or the fixed product
recipe.

## Options and Owner touchpoints

1. **Keep `B5 = UNSUPPORTED` and `NOT_RUN` (recommended now).** No login, install,
   credential action, or scope change is required. This preserves the accepted
   boundary and records the blocker honestly.
2. **Reconsider only after Google documents a per-profile OAuth store or a
   brokered login/model split.** The Owner would decide whether to authorize a
   fresh version-pinned Win11 qualification and would personally complete any
   required sign-in in the visible official flow. No credential file/token
   inspection is needed or allowed. Prove the actual same-user/LPAC behavior
   and real workspace/history separation on the exact binary; a different
   `HOME`, capability flag, model list, or model self-report is not proof.
3. **A distinct OS-user or Gemini API-key route changes the authority/account
   model.** Either would need a separate Owner direction and authorization for
   the new identity/credential, installation or secret, applicable account
   terms and any spend. They are not fallback implementations under current B5.

No work here changes the plan, support matrix, native provider recipe, CLI pin,
permission profile, or acceptance result. The next useful measurement is a
source-supported selector/broker contract from Google. Repeating probes with
different home folders cannot answer the keyring question.

## References

- Plan: [`PLAN.md` B5.1](../design/gogoke-37-plan-v1/PLAN.md); current M2 status:
  [`m2-win11.md`](../../tools/e2e/m2-win11.md).
- Existing fixed recipe and environment boundary:
  `apps/desktop/native-host/src/store/instance/provider_login.rs` and
  `PROVIDER_LOGIN_REFERENCES.md`.
- Prior pinned-version research:
  [`2026-09-26-antigravity-cli-facts.md`](2026-09-26-antigravity-cli-facts.md).
- [Google Antigravity CLI installation and authentication](https://www.antigravity.google/docs/cli/install/),
  [CLI reference](https://www.antigravity.google/docs/cli/reference/),
  [headless mode](https://www.antigravity.google/docs/cli/headless/), and
  [current download/version page](https://www.antigravity.google/download).
- [Google's upstream auth-profile issue #381](https://github.com/google-antigravity/antigravity-cli/issues/381)
  (open when checked; user request, not implementation evidence).
- [Microsoft AppContainer credential isolation](https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation),
  [LPAC/AppContainer requirements](https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer),
  and [`CredRead`](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credreadw).
- Open-source comparisons reviewed: `burakdede/aisw` README and auth matrix
  (Antigravity OAuth is described as shared keyring; its API-key profile is a
  different Gemini route); `Spielewoy/multi-cli`'s Antigravity adapter notes
  (Antigravity **IDE 2.x**, separate OS-user approach); NaveHQ's
  `navehq_cao_true_launch_abort_cleanup_boundary_v0.1.md`; GOGO PARTY's
  `packages/room/src/accounts.ts`; LoomOS decisions/specs (no relevant match).
