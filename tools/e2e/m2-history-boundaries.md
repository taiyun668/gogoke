# M2 original history boundary cases

`runHistoryBoundaryCases(product, config, journal)` extends the installed M2
driver with real K-SESSION and K-SIDE operations. It uses `ActualProduct` from
`product-cdp.mjs`, its hard CDP Home locator, original User ingress, custody,
fixed CLI pin, append-only raw H/A records and normal caption close. Importing
the module performs no operation. There is no new launcher or test framework.
The module does not use `agent.act`, login, change a CLI pin, change permissions,
inspect a credential or read a vendor history file.

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
  cases: [{
    driverId: 'codex', // or claude, opencode, grok; only already admitted pins
    instanceId: 'originalTestInstance',
    version: 'originalFixedVersion',
    sha256: 'originalFixedBinarySha256', // 64 hexadecimal characters
    projectA: {
      repositoryId: 'gogokeSeatTestbed',
      seatId: 'originalExclusiveSeatA', worktreeId: 'originalHostCreatedTreeA'
    },
    projectB: {
      repositoryId: 'originalSecondTestRepository',
      seatId: 'originalExclusiveSeatB', worktreeId: 'originalHostCreatedTreeB'
    }
  }],
  normalCloseReadbackRestart: async phase => { /* existing runner integration */ }
}
```

The configuration and its records remain in the existing private evidence
directory. `sha256`, version, IDs and test objects must come from original F/E
creation/registration evidence. Both repositories are candidate test sources;
no formal source, installation, data or user global HOME is an allowed target.
F must already bind each named worktree to its named seat and the same instance
in the candidate domain. Seats/worktrees are disjoint across cases and owned
exclusively for the whole flow; instance/project caps must already permit two
live sessions. The module never changes caps, login state or permission tiers.
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

Each configured provider opens two genuine WORK sessions on separate registered
test repositories, on the identical fixed instance, before sending a different
random non-secret marker to each. Both original H inputs are sent before either
process stops. Neither request contains the other project's marker. The
assistant A event streams must actually contain each original marker so an
empty history cannot satisfy the control. Model prose about what it remembers
is never the isolation assertion.

The second seat is stopped/released, then opened through original
`gogoke.37.owner-side-open.v1` as SIDE_CHAT with the first original WORK session
as its source. A second unique marker is delivered through
`gogoke.37.owner-side-question.v1`, including D's assembled reference and exact
explicit question. After physical stop/release, the same second seat opens a
new FORMAL_REVIEW session and sends one fresh marker. The reader requires four
distinct actual vendor/native session IDs, different generations on the same
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
and registrations, F worktrees/seats and D registry/sync/pending rows byte for
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
bound `thread/start` reply contains `result.thread.path`; it does not open it
or infer a filename from HOME. Controller's production readback/ACL integration
must supply the real object binding and an authorized exact-object native
worker negative case before this scope fact can pass. Null/missing path,
static paths, empty fields, guessed credentials or test-created fake history
cannot close it. Antigravity remains NOT_RUN and is never launched.

## References and execution record

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

Validation for this construction package: signed Node `--check` and signed
Python `ast.parse` only. Reader execution, databases, native binaries, CLI,
models, application launch, tasks, ACL behavior and Owner Windows 11 execution
are **NOT_RUN**. Cloud hygiene must be checked for the pushed commit separately.
Shared runner/CI wiring is a Controller integration dependency. This worktree
is retained for that integration and independent review; no candidate data or
runtime artifacts were produced here.
