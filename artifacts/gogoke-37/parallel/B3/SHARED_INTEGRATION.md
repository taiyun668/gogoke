# B3 shared integration request

Status: adapter package only. No shared file or host wiring has been changed here.

## Requested integrator changes

1. Register `driverId=opencode` for this adapter's version in the existing adapter registry and source ledger. Keep the OpenCode executable version separate from the gogoke adapter version.
2. Extend the native instance catalog to pin OpenCode `1.18.32` and measure the executable identity from native observation. Do not accept a path or digest supplied by this adapter.
3. In H's existing managed-process path, launch the pinned executable with `acp`, using the registered isolated instance home and `opencodeInstanceEnvironment(home)` merged into the explicit child environment. Set cwd from the host's registered workspace, omit inherited `PWD`, and preserve the current process identity, retained custody, Job, stop-fact and admission rules.
4. Supply ACP NDJSON framing (one JSON object per line) and JSON-RPC request-ID correlation as the host transport. Route adapter actions through the current K-SESSION session/generation binding; do not add a parallel session store or let the adapter start a process.
5. Route login from the OpenCode instance page through the existing provider-login host flow and the official `opencode auth login` command. The ACP initialization advertises this only when `clientCapabilities._meta["terminal-auth"]` is true. Keep the progress/result session host-owned, expose no credential contents, and do not mark logged in from file presence alone. This package did not start login.
6. Tag every ACP event with the H-owned session identity before A normalization. ACP `session/update` is raw vendor data, not a durable seat-ledger event or task receipt.
7. On K-SESSION resume, require confirmed old-generation STOPPED, held admission, current OpenCode pin/capability and custody, then create the new process generation and call `session/resume` with the native OpenCode session ID. Reconcile unknown outcomes by original host request ID; never create a substitute session automatically.
8. On interrupt, keep the inbox delivery pending until the active `session/prompt` returns `stopReason=cancelled`. That response confirms the model turn stopped; H still waits for native process stop proof before releasing custody/admission or overlapping a generation.
9. Before allowing an OpenCode workspace, ensure the managed cwd and its ancestor instruction lookup are the intended project tree. `docs/research/2026-09-26-one-instance-two-projects.md` withdraws the earlier temp-directory observation as an environment artifact while retaining the general rule that upward instruction discovery needs a host-controlled workspace and explicit child environment.

## Shared files for the integrator

These paths are listed in the authorized plan's shared-file map and remain integrator-owned:

- `third_party/t3code/apps/server/src/gogoke/adapters/native/registry.ts`
- `third_party/t3code/apps/server/src/gogoke/index.ts`
- `third_party/t3code/apps/server/src/gogoke/contracts/model.ts`
- `third_party/t3code/apps/server/src/gogoke/contracts/codec.ts`
- `third_party/t3code/apps/server/src/gogoke/contracts/index.ts`
- `third_party/t3code/apps/server/src/gogoke/bootstrap/index.ts`
- `apps/desktop/native-host/src/process/windows.rs`
- `apps/desktop/native-host/src/store/authority/*` and native session service files

The H/session and instance changes are not part of B3's own write scope. The integrator should amend only the minimal authorized extension points and perform the common K-SESSION conformance integration.

## Validation still required

- A real protocol transcript captured from the exact official 1.18.32 executable and bound to its SHA-256. Current protocol sample status: `NOT_RUN`; no fixture is declared golden.
- A real process start/new-session/prompt/resume/interrupt sequence through the native H path with stop receipts, no duplicate message and unchanged native session identity.
- Ordinary Win11/SAC verification that the pinned executable starts when the final signed package uses it.
- Effective config and instruction-source observation across two sessions; current environment builder does not establish runtime isolation by itself.
- The common K-SESSION fake conformance contract, the integrated real H path, and the later exact-candidate M3 repeat.

The B3 package code and local scripted tests establish adapter state handling only. They do not close M2, M1, or M3 and do not establish login, process custody, native isolation or installed-product behavior.
