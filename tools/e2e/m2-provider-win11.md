# Standalone installed M2 provider E2E

`m2-provider-win11.mjs` captures one real turn each from the already logged-in,
fixed Claude, OpenCode and Grok instances through the installed gogoke product.
It reuses `ActualProduct`, its hard installed-WebView locator, and
`tester-army/e2e` with telemetry disabled and `agent.act` fixed at zero. It does
not run the main M2 Codex child/merge flow, create or register worktrees, invoke
provider CLIs itself, log in, inspect credentials, or modify a file in a
provider worktree. This is protocol evidence, not M2 acceptance.

Run the signed Node and Python runtimes on the Owner's Win11 machine, with the
existing private fixture path supplied as the sole argument:

```powershell
& $SignedNode tools/e2e/m2-provider-win11.mjs $PrivateProviderFixture
```

The fixture supplies the exact installed product path and SHA-256 map, product
version and source commit, registry key, PowerShell/Python runtime paths,
candidate `stateRoot`, private `evidenceDirectory` and fresh `result` path,
actual domain, `repositoryId: "gogokeSeatTestbed"`, and the existing formal,
memory and ledger observers. The formal observer must compare all five
`formal`, `registeredFormal`, `formalData`, `formalRegistry`, and `shortcuts`
facts. Every configured observer runs before launch and after normal close;
its configured fields must remain identical. The memory observer also keeps
the existing no-write facts: `memoryDataUnchangedByRead`, zero
`stage1OutputCount`, and zero `memoryJobCount`.

`cases` contains exactly three distinct seat/worktree bindings, one each for
`claude`, `opencode`, and `grok`. Each row has `instanceId`, `seatId`,
`worktreeId`, fixed CLI `version`, CLI `sha256`, and the expected `model` and
`effort`. The model and effort used in the run are read from that original
seat's state card and must equal the fixture. OpenCode must name an xAI/Grok
model; a GPT setting fails before its turn. The actual instance must already
be `LOGGED_IN`, the seat `IDLE`, and F graph registration must bind the same
domain, repository, seat, instance and worktree. Missing login, capability, or
setting mismatch fails without login, substitution, or fallback.

For each provider, the runner reserves and commits the existing seat, opens its
registered F worktree, checks H's capability version and executable digest,
and sends exactly one private non-secret marker prompt. It then observes only
the original output stream and request receipt. An `UNKNOWN` request is never
sent again; a stated unknown reason fails immediately. A timeout or any
failure preserves the original app/session handle and request for Controller
follow-up. The normal path requires the original H `APPLIED` input receipt,
acquires the real stop fact, releases the admission, and closes the installed
product through its normal window close.

Model provenance is reported by source: Claude's original H capability carries
the requested seat model and effort, and a read-only Windows process observer
captures only the model/effort argv values from the exact SHA-pinned CLI process
inside this installed product's process tree. The selected PID's exact
100-nanosecond creation time comes from `GetProcessTimes`, then readback binds
it to the same stopped H custody row. The process observer does not store the
full command line. Grok uses the same pinned-child
argv observation with its `--model` and `--reasoning-effort` values. OpenCode
requires the original H-recorded ACP `session/set_config_option` model and
effort commands plus their typed-ID-matched A acknowledgements, with the exact
current and offered values in order. These facts establish the requested/acknowledged
settings; they do not claim provider-side model behavior beyond the captured
protocol evidence.

Only after normal product close does
`m2-provider-capture-readback.py` open the actual candidate database as
`mode=ro&immutable=1`. It refuses a nonempty WAL and verifies unchanged DB
bytes. The outer journal distinguishes read-only observers from the expected
candidate database writes caused by real H admissions, provider input, stop,
and release. The five formal protection facts and configured observers are
captured and compared before readback or golden import, so their evidence
remains if later capture work fails. The readback itself independently checks the original H request/receipt,
physical stop and release facts, configured E settings and H binding, fixed
binary custody, and registered F identity by opening the path without following
a reparse point and matching Win32 `FileIdInfo` to the native opaque identity.
Raw pending frames retain their original state with an explicit classification;
a pending row alone does not fail the case. Completion still requires the H
receipt, matching provider end-turn and marker, and the fixed identity/stop
facts above. It exports the
original A frames, confirmed H commands, and normalized ledger rows. Claude
requires its exact original User echo, same vendor session, marker-bearing
assistant output and successful original `type: result`; OpenCode and Grok
require the original `session/prompt` write, its typed-ID matched `end_turn`
response and marker-bearing `agent_message_chunk`. Unexpected provider tool or
permission activity fails the capture.

Reference: path spelling and containment follow the existing M2 readback;
physical F identity follows the native `RootIdentity::opaque()` encoding and
Win32 `FILE_ID_INFO` layout; process creation identity uses the native Win32
`GetProcessTimes` 100-nanosecond value. The independent reader keeps these
checks beside its original H/A/F evidence because they must bind to each of
the three provider sessions in this standalone journal.

The existing `cli-protocol-golden.mjs import` consumes that raw readback
directly once per provider. It writes private bundles marked
`REVIEW_REQUIRED` / `NOT_ASSESSED`; they are diagnostic captures, not accepted
golden baselines. The driver ends with `state: "REVIEW_REQUIRED"` and
`acceptance: false`. A missing case, failed readback, failed observer, or
incomplete direction/output cannot become a pass.
