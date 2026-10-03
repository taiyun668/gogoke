# Real installed Win11 M1 checks

This development package attaches to the actual installed candidate's WebView2.
It is not bundled with the product. `e2e` 0.16.0 and `@e2e-dev/web` 0.11.2 are
locked development dependencies; browser downloads and installation scripts are
disabled. Set `E2E_TELEMETRY_DISABLED=1` for every invocation.

The Controller runs the scripts in the existing **Interactive/Limited** ordinary
Win11 task. No Owner configuration edits, request-ID copying, e2e subscription,
login or `agent.act` are needed. M3 navigation that needs an agent will use the
Owner's subscription after one explicit Owner login; agent judgments never
replace hard assertions and native facts.

## Connection

`trial-connect.mjs PRIVATE_CONFIG` tries the public `web({ connect })` API against
the original candidate page, with hard Home/Tauri assertions and host instance
status. The real 0.1.19 trial passed, including normal caption close and unchanged
formal protection snapshots. An earlier driver readiness-namespace mistake did
not reach the library and is retained as a measurement failure.

`product-cdp.mjs` verifies candidate HKCU registration, installed file hashes,
bootstrap version/resource identity, PID and loopback listener ancestry before
attaching. The raw CDP bootstrap proves which page exists; e2e then retains that
same page for operations. Each restart creates a new owned connection. It does
not attach to or operate authentication pages, a Vite mock, a new browser context
or a substitute CLI. Raw CDP is a documented fallback only when the e2e trial is
unsuitable; record the actual reason before setting `testerArmy: false`.

## M1 flow

Run `m1-win11.mjs PRIVATE_CONFIG` with signed Node. The Controller generates the
private config from the current frozen manifest and existing native bindings.
All outputs remain outside the repository.

Required config fields are `installed`, `version`, `sourceCommit`,
`installedSha256` (shell, native host and resource index), `registryKey`, `pwsh`,
`evidenceDirectory`, `result`, `domainId`, `seatId`, `instanceId`, `repositoryId`,
`worktreeId`, `worktreeRoot`, `cliVersion`, `cliSha256`, and `observers`.

The mandatory readonly observers are `formal`, `memory`, and `ledger`. Each has
`name`, signed `runtime`, argument array `args` with an `{output}` placeholder,
and explicit `equalFields`. The existing formal snapshot compares **formal,
registeredFormal, formalData, formalRegistry, shortcuts**. The existing memory
reader compares DB/WAL bytes and row counts; a pre-existing DB is not evidence
that memories are enabled. `m1-readback.py STATE_ROOT OUTPUT` supplies the actual
initial ledger epoch/cursor while the product is closed. This is a measurement
bootstrap: the current product API does not expose an initial epoch lookup.

The script reads `codexTestM1` login status, derives the next generation from the
actual seat, opens S1, asks/answers a real native question, steers its same turn,
checks a new JSON file and marker, compacts/renews, normally closes/restarts the
product, resumes the same logical thread and ledger subscription, and completes
a distinct S2. It then stops/releases and repeats the protection snapshots.
Source requests and receipts are saved before/after each write. Mutations are
not implicitly repeated. An explicit generation-change UNKNOWN with no original
failure reason is reconciled using its **exact original bytes and request ID**;
the native RPC journal fences a second vendor write. Transport loss or an actual
failure reason preserves the case for Controller repair.

On Windows, the driver launches the candidate detached from Node's own
kill-on-close Job and directs stderr to a unique private file. On FAIL it keeps
the original child handle and task alive until Controller settles and normally
closes that product; it does not destroy its pipes, terminate it, or invent a
stop fact. Exit code, signal and stderr tail are retained. This does not claim
that any outer Task Scheduler Job is absent.

After normal close, run `m1-readback.py STATE_ROOT OUTPUT E2E_JOURNAL`. This reads
only the case's existing RPC/raw/normalized ledger and stop facts, checks every
observed turn (including the resumed turn), and retains unknown raw methods
explicitly. Ledger stream ordinals and `_meta.rawSourceCursor` are different
facts. The runner's completed flow alone is not M1 completion or Owner acceptance.
Steer consumption is asserted from the original completed agentMessage for the
same thread and turn; optional live deltas and User delivery receipts are not
substitutes for that vendor evidence.
The closed-product observer rejects a nonempty WAL and uses SQLite immutable
read mode so it cannot create WAL/SHM sidecars; all three files are compared.

## M2 flow

[m2-win11.md](m2-win11.md) describes the main native model takeover, child
dispatch, real tool write, stop/release and merge path. Its optional
[m2-sidechat.md](m2-sidechat.md) case uses separate source and side sessions.
Both attach to the actual installed candidate, retain exact request bytes and
require immutable readback after normal close. The new M2 drivers have not yet
been run on the Owner's Win11; the three other-provider logins and recordings
remain NOT_RUN, and Antigravity awaits the Owner scope decision.

M2 recordings use `cli-protocol-golden.mjs import --session-id` to keep each
original provider session and its actual F/H binary identity separate. Missing
directions or normalized events remain incomplete recordings.

## Real CLI recordings

Monitor a running case through its append-only stdout log. Read the JSON
journal with `m1-progress.mjs` only after its task settles. Windows can refuse
the atomic replacement of an open target, including readers that share DELETE.
Snapshot inventories stay in their original files; the journal records hashes
and references, and comparisons read those exact original files. Repeated
assertions still check every value while recording each assertion name once.

Use [cli-protocol-golden.md](cli-protocol-golden.md) to import the readonly
readback and compare versions. Original private bytes remain local; sanitized
observations retain per-direction order, scoped RPC associations, field shape
and normalized events. A failed or incomplete capture is not a success baseline.
M2's other providers use this same real-capture approach after their historical
login flows and official protocols are checked.

## References and differences

- The existing private installed-product launcher, endpoint-custody check,
  checked User batches, output drain, caption close, formal snapshot and memory
  reader supply the actual proven launch/measurement path. The new scripts make
  their steps repeatable; they do not add a product probe or fake provider.
- GOGO PARTY `packages/room/src/accounts.ts` supplies per-instance homes and
  status without reading credentials; seat runtime and Windows process
  supervisor history supply native input and custody semantics.
- Repository `docs/research/reuse-blueprint.md` and source teardown recommend
  real protocol recordings; synthetic conformance fixtures are a separate fact.
- The installed upstream e2e `docs/browser.mdx` and public engine contract
  describe CDP persistent contexts. The trial uses its original-context connect,
  public live surface and hard locators. No agent fixture/model provider is used.
- Native tests/mutations for this change are limited to exact file identity,
  DACL/permissions and the existing security gates. The non-equivalent Rust
  parent/helper protocol fixture was removed; real CLI behavior is tested here.
