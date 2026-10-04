# M2 original history boundary cases

`runHistoryBoundaryCases(product, config, journal)` extends the installed M2
driver with real K-SESSION and K-SIDE operations. It uses `ActualProduct` from
`product-cdp.mjs`, its hard CDP Home locator, original User ingress, custody,
fixed CLI pin, append-only raw H/A records and normal caption close. Importing
the module performs no operation. There is no new launcher or test framework.
The module does not use `agent.act`, login, change a CLI pin, change permissions,
or inspect a credential. Optional peer cases ask the real native command tool
to read one original non-secret test history object.

Build/execution constraints follow
[`gogoke-build-and-release.md`](../../docs/governance/gogoke-build-and-release.md).
The actual candidate and native binaries come from cloud builds; this package
adds only development scripts run with already signed Node/Python. It adds no
product child process. Actual native/CLI/Smart App Control behavior requires the
Owner Windows 11 candidate execution; source parsing cannot establish it.

## Controller integration

The shared `m2-win11.mjs` is outside this package's ownership. Controller must
import `runHistoryBoundaryCases` and call it while the actual installed product
is connected. Private configuration supplies `historyBoundary`:

```js
{
  lifecycleOwnership: 'EXCLUSIVE_M2_HISTORY_SEATS',
  peerRead: true, // optional stable-point real Codex exact-object negative cases
  cases: [{
    driverId: 'codex', // or claude, opencode, grok; only already admitted pins
    instanceId: 'originalTestInstance',
    version: 'originalFixedVersion',
    sha256: 'originalFixedBinarySha256', // 64 hexadecimal characters
    projectA: {
      domainId: 'originalTestDomainA', repositoryId: 'gogokeSeatTestbed',
      seatId: 'originalExclusiveSeatA', worktreeId: 'originalHostCreatedTreeA'
    },
    projectB: {
      domainId: 'originalTestDomainB', repositoryId: 'gogokeSeatTestbed',
      seatId: 'originalExclusiveSeatB', worktreeId: 'originalHostCreatedTreeB'
    },
    sideBinding: {
      domainId: 'originalTestDomainA', repositoryId: 'gogokeSeatTestbed',
      seatId: 'originalExclusiveSideSeat', worktreeId: 'originalHostCreatedSideTree'
    }
  }],
  normalCloseReadbackRestart: async phase => { /* existing runner integration */ }
}
```

The configuration and its records remain in the existing private evidence
directory. `sha256`, version, IDs and test objects must come from original F/E
creation/registration evidence. Both project domains share the already
authorized private `gogoke-seat-testbed` source registered as
`gogokeSeatTestbed`; a second remote repository is not created. All objects are
candidate test sources;
no formal source, installation, data or user global HOME is an allowed target.
F must already bind each named worktree to its named domain/seat and the same
instance. `projectA.domainId !== projectB.domainId` and
`sideBinding.domainId === projectA.domainId` are mandatory. `sideBinding` has a
third independent seat/worktree, reused only by its subsequent formal review.
Seats are identified by domain plus seat ID; worktree IDs are globally distinct.
Seats/worktrees are disjoint across cases and owned
exclusively for the whole flow; instance/project caps must already permit two
live sessions. The Controller precreates the E seats and F worktrees in both
domains and the third side binding; the module does no E/F provisioning. Its
operation wrapper submits each original session's domain through the real User
bridge and preserves raw requests/receipts without changing `ActualProduct`
or its config. The frozen receipt envelope has no domain field; the original
request and independent persisted H/A/F/D rows supply domain evidence.
The module never changes caps, login state or permission tiers.
Do not share these seats with M1, V08, V12 or other active work.

The callback must perform these existing runner actions, in this order:

1. `await product.closeNormally()` after the module stopped/released its H
   sessions. Other test flows must also be settled before candidate close.
2. Spawn the configured signed Python with the new reader, `config.stateRoot`,
   a fresh output in `config.evidenceDirectory`, `config.result`, and
   `before-refusal`. Require exit zero; retain stderr and the private output on
   failure. Never launch a reader against an active candidate or nonempty WAL.
3. Record the file basename and actual SHA256; `await product.launch()` on the
   same candidate/root/domain, then return `{file, sha256}`. The module rechecks
   installed bytes and custody after this normal restart.

After the module returns `FLOW_COMPLETE_DIRECT_READBACK_REQUIRED`, the runner
must normally close the candidate and run the same reader with phase `final`.
Final output must be in the same private evidence directory as the journal and
the original baseline. Require `directFlowEvidence`, `directRefusalEvidence`,
and `measurementPreservedDatabaseBytes` to be true. Record its actual hash in
the shared journal. This does not set M2 or Owner acceptance.

```text
signed Python tools/e2e/m2-history-boundaries-readback.py STATE_ROOT NEW_PRIVATE_OUTPUT ORIGINAL_M2_JOURNAL before-refusal
signed Python tools/e2e/m2-history-boundaries-readback.py STATE_ROOT NEW_PRIVATE_OUTPUT ORIGINAL_M2_JOURNAL final
```

## Actual flow and independent assertions

Each configured provider opens two genuine WORK sessions in different registered
project domains sharing the same test repository and identical fixed instance,
before sending a different
random non-secret marker to each. Both original H inputs are sent before either
process stops. Neither request contains the other project's marker. The
assistant A event streams must actually contain each original marker so an
empty history cannot satisfy the control. Model prose about what it remembers
is never the isolation assertion.

The second domain's seat is stopped/released. The independent third seat in the
first source domain is opened through original
`gogoke.37.owner-side-open.v1` as SIDE_CHAT with the first original WORK session
as its source. Both nested side-open/create requests, source registration and
side registration belong to that source domain. A second unique marker is delivered through
`gogoke.37.owner-side-question.v1`, including D's assembled reference and exact
explicit question. After physical stop/release, the same third side seat opens a
new FORMAL_REVIEW session and sends one fresh marker. The reader requires four
distinct actual vendor/native session IDs, two different project domains,
three independent worktrees, different generations on the same source-domain
side/review seat, a genuine D registry/sync, fresh native `thread/start` or
`session/new`, and no native resume/fork command. Claude identity is bound to
its actual echoed User and success Result. The original formal H commands must
contain none of the previous project or side private markers.

The module then stops/releases all owned processes and obtains a normally
closed independent baseline. After restarting only the candidate, it attempts
`resume`, `reconnect`, `compact`, `renew-session`, and an `open` with the formal
purpose plus `sourceSessionId` (the existing product's inherited/fork-open
negative shape) on each persisted formal session. Each original User ingress
receipt must be DENIED with unchanged revision. No arbitrary standalone fork
verb is substituted for that production branch. Final readback compares the
original scoped H claim/operation/process/generation/stdin/RPC rows, A raw/index
and registrations, F worktrees/seats and D registry/sync/pending rows in each
session's actual domain byte for
byte against the baseline. A denial with a new process, command, source event
or changed test object fails.

Readback uses only `state.sqlite` with `mode=ro&immutable=1` after the exact
candidate's normal-close receipt and empty WAL. It preserves original source
BLOB bytes and typed RPC IDs, matches H command ACKs to A source cursors and
physical custody, checks fixed CLI bytes, and compares database/sidecar hashes
before/after observation. Unexpected input uncertainty, tool/question activity,
incomplete raw capture or provider failure stops the flow with original error;
there is no resend, alternative CLI, forced process stop or synthetic golden.

## Remaining production dependencies and non-claims

For Codex, the reader reuses the already existing production `config/read`
command and fixed response decoder's three effective memory flags. It matches
the actual H command to its original A response by typed RPC ID, source epoch,
cursor and physical custody; `features.memories`, `generate_memories` and
`use_memories` must each be false. It reports that original configuration fact,
without a new probe or operation. Other vendors have no such qualified fact in
this reader. Configuration observation does not prove memory-store activity or
loaded instruction provenance.

V04b overall remains **NOT_RUN** until actual vendor memory-store and loaded
instruction provenance are available. Original prompt isolation and fresh
threads prove the tested H input behavior; launch flags, config intent,
different directories and model answers cannot prove vendor memory was off.
V10 overall also records **NOT_RUN** for effective vendor history provenance
even when the original SideChat/fresh formal H/A/D flow and refusal facts are
complete. Both fields are explicit in the independent reader output.

The worker-to-OwnerLead history product scope case is **NOT_RUN**. Current
`model_call.rs` exposes `gogoke_seat`, `gogoke_policy`, `gogoke_worktree` and
`gogoke_takeover`, without a model ledger-read tool. User
`K-LEDGER/scoped-query` selects `readerSessionId` and is not a worker native
history-read attempt. There is currently no qualified production field that
locates an exact non-secret OwnerLead test history object across vendors.
The reader exports `originalCodexThreadPath` only if the original, physically
bound `thread/start` reply contains `result.thread.path`. With `peerRead: true`,
the existing `before-refusal` normal-close readback opens only that original
test JSONL, after checking the production registry's `home_ref`, physical home
identity, containment inside the candidate registered instance home, and absence
of reparse points. No filename is inferred from HOME. The first original
`session_meta.id`/`cwd` must match the unique original native H/A/F identity;
the actual assistant record must contain its non-secret marker. The reader
exports `verifiedVendorObjects` with the exact path, file identity and SHA256;
it preserves the vendor file bytes and copies no vendor database. Missing path,
unsupported session metadata, home identity mismatch or unavailable original
file records `vendorObjectNotRun` and cannot trigger a model read. Antigravity
remains NOT_RUN and is never launched.

## Optional original peer file reads

Controller first completes the old `final` readback and records its reference
under `journal.readbacks` with `phase: 'history-final'`. This preserves all four
original single-marker/no-tool sessions and the original refusal snapshot.
After that normal close, Controller launches the same candidate and calls
`runHistoryPeerReadCases(product, config, journal)` from this module. Its
returned `PEER_FLOW_COMPLETE_DIRECT_READBACK_REQUIRED` requires another normal
close and the same Python reader with `peer-final`. The shared runner owns this
minimal integration; the peer module adds no lifecycle callback or product API.

For each Codex case with a qualified original project A object, peer flow opens
two new logical and physical sessions on the released original F/E bindings:
project B WORK on the same instance, and fresh FORMAL_REVIEW on the side/review
seat. Each has one independent native turn requesting exactly
`type "ORIGINAL_THREAD_START_PATH"` with `shell="cmd.exe", login=false`.
This uses the same provisioned shell as the actual Codex launch instructions;
PowerShell is not provisioned for those model sessions. Paths with CMD expansion
or control characters are refused before input is sent.
There is no resume, reconnect, inherited source, copied history, fake ACL,
permission change, additional shell script or fallback command. A native
question/approval or source error stops the original run for Controller.

`peer-final` binds each original H stdin/turn/start/typed ACK and physical
custody to its original A tool lifecycle, thread and turn. Both actual tool
command strings must be the ordinary exact-object read. A denial requires the
native `commandExecution` started/completed pair, nonzero original exit code
and original aggregated output consisting of CMD's `Access is denied.` text.
The exact original object must still exist with the same identity and hash;
missing-file text and other error output do not qualify. It invents no numerical
Win32 code. Unsupported command rendering or localized output stays NOT_RUN.
The reader rechecks the
original source SHA256 and identity after the tool attempts.

The result is `directPeerReadEvidence: true` only when both independent scope
cases for every configured completed Codex case have that original denial.
No tool, missing raw code or unsupported native command spelling produces
`NOT_RUN_PEER_READ_DENIAL_UNQUALIFIED`; an actual successful read fails.
Generic exit 1, assistant text, an echoed error, UNKNOWN, intended command,
empty history or a skipped run cannot pass. Controller preserves the original
history flow facts independently when peer evidence is NOT_RUN. This checks
only these test product scopes; it does not close OwnerLead history, vendor
memory or instruction provenance, nor assert any particular production ACL
layout or M2/Owner acceptance.

```text
signed Python tools/e2e/m2-history-boundaries-readback.py STATE_ROOT NEW_PRIVATE_OUTPUT ORIGINAL_M2_JOURNAL peer-final
```

## References and execution record

The registered home identity comparison requires Windows Python 3.12 or later.
CPython's [Windows stat conversion](https://github.com/python/cpython/blob/v3.13.0/Python/fileutils.c)
uses the native volume serial and 128-bit file ID, matching `root::RootIdentity`;
unsupported or unequal observations stay NOT_RUN. The reader preserves the
original extended local path for file access, rejects reparse and additional
links, and compares its DOS spelling with the registered home.

The historical GOGO PARTY context/session governance in
[`06-context-session-governance.md`](../../docs/design/06-context-session-governance.md)
requires source-bound prompt/load evidence and distinguishes fresh context from
native continuation. The own-history research
[`2026-09-25-own-history-rework.md`](../../docs/research/2026-09-25-own-history-rework.md)
and ledger/context source audit
[`07-ledger-workflow-context-sources.md`](../../docs/research/source-audit/07-ledger-workflow-context-sources.md)
inform the original-source and project-owned history rule. Existing
`m2-provider-cases.mjs`, `m2-provider-readback.py`, `m2-rules.mjs`,
`m2-rules-readback.py`, `m2-sidechat.mjs`, and `m1-readback.py` supply the actual
User composition, generation, stop and post-close measurement pattern.
Production `v37_runtime.rs`, `v37_runtime_tests.rs`, `v37_side.rs`,
`session_transport/journal.rs`, `rpc_journal.rs`, and `ledger/mod.rs` determine
the actual field names and denial branches. This package extends their real
flows; it does not redefine contracts or HOME layout.

The frozen design 37 §2b2 states that repositories can be shared across projects
and project isolation is independent of repository identity. Production
`ledger::query` filters domain, H binds claims and seats by domain, F records
domain separately from repository, and D checks both source and side registration
inside its own domain. The earlier same-domain/two-repository fixture premise
was incorrect. This package now requires two real domains and three domain-bound
F/E test bindings; changing only repository names cannot satisfy it.

Validation for this construction package: signed Node `--check` and signed
Python `ast.parse` only. Reader execution, databases, native binaries, CLI,
models, application launch, tasks, ACL behavior and Owner Windows 11 execution
are **NOT_RUN**. Cloud hygiene must be checked for the pushed commit separately.
Shared runner/CI wiring is a Controller integration dependency. This worktree
is retained for that integration and independent review; no candidate data or
runtime artifacts were produced here.
