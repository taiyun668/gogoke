# Installed M2 cross-project memory entry

`m2-memory-win11.mjs` runs the existing `m2-history-boundaries.mjs` against a
fresh, already logged-in Codex and/or Claude instance in the **installed**
candidate. It is an independent entry: it does not run the M2 main/child case,
V12, login, native builds, or a vendor CLI outside the product. The existing
module opens two original H sessions on one instance before either project
marker turn. Its immutable reader binds each original H input, native response,
A source, F registration and physical history object to the real candidate.
The module also performs its existing side/formal sequence because its reader
requires that baseline and refusal chain. These are fresh sessions for the
cross-project condition; earlier M2 results cannot be relabelled as this run.

This entry reports **partial evidence only**. Codex's original `config/read`
response must show `features.memories`, `memories.generate_memories` and
`memories.use_memories` all false. Both projects must have separate original
native session IDs, actual F bindings and original H inputs without the other
project's marker. Claude's launch requests the documented
`CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`, but the pinned CLI's effective auto-memory
state and its actual instruction-file load set are not reported by the current
original stream. Neither the flag, separate homes, no-write snapshot nor model
prose closes V04b. The immutable reader and this driver leave V04b overall
`NOT_RUN`. OpenCode, Grok and Antigravity are `NOT_RUN` in this entry.

Use the signed Node and Python runtimes on Owner Win11. Make a **fresh private**
JSON fixture outside the repository and candidate state root, with a new
`result` filename in a private existing `evidenceDirectory`. Reuse the current
installed-candidate manifest and the three existing read-only `formal`,
`memory` and `ledger` observers from the private M2 fixture. The formal
observer must compare `formal`, `registeredFormal`, `formalData`,
`formalRegistry` and `shortcuts`; each observer needs its actual signed
`runtime`, `args` containing `{output}`, and `equalFields`. The memory observer
must report `memoryDataUnchangedByRead`, `stage1OutputCount=0`, and
`memoryJobCount=0`. No credential or formal source file is an observer target.

Private fixture shape (all angle-bracket values must be replaced locally):

```json
{
  "installed": "<installed candidate directory>",
  "installedSha256": {"gogoke.exe": "<sha256>", "gogoke-native-host.exe": "<sha256>", "resource-index.json": "<sha256>"},
  "version": "<installed version>",
  "sourceCommit": "<40-hex installed source commit>",
  "registryKey": "<installed registry key>",
  "pwsh": "<signed PowerShell executable>",
  "python": "<signed Python executable>",
  "stateRoot": "<candidate state root>",
  "evidenceDirectory": "<existing fresh private evidence directory>",
  "result": "<new private evidence directory/result.json>",
  "domainId": "<project A domain>",
  "repositoryId": "gogokeSeatTestbed",
  "observers": [
    {"name": "formal", "runtime": "<signed observer runtime>", "args": ["<read-only observer>", "{output}"], "equalFields": ["formal", "registeredFormal", "formalData", "formalRegistry", "shortcuts"]},
    {"name": "memory", "runtime": "<signed observer runtime>", "args": ["<read-only observer>", "{output}"], "equalFields": ["<actual memory fact field>"]},
    {"name": "ledger", "runtime": "<signed observer runtime>", "args": ["<read-only observer>", "{output}"], "equalFields": ["<actual ledger fact field>"]}
  ],
  "historyBoundary": {
    "lifecycleOwnership": "EXCLUSIVE_M2_HISTORY_SEATS",
    "peerRead": false,
    "cases": [{
      "driverId": "codex",
      "instanceId": "<already logged-in fixed instance>",
      "version": "<fixed CLI version>",
      "sha256": "<fixed CLI executable sha256>",
      "projectA": {"domainId": "<project A domain>", "repositoryId": "gogokeSeatTestbed", "seatId": "<exclusive idle seat A>", "worktreeId": "<registered worktree A>"},
      "projectB": {"domainId": "<different project B domain>", "repositoryId": "gogokeSeatTestbed", "seatId": "<exclusive idle seat B>", "worktreeId": "<registered worktree B>"},
      "sideBinding": {"domainId": "<project A domain>", "repositoryId": "gogokeSeatTestbed", "seatId": "<exclusive idle side seat>", "worktreeId": "<registered side worktree>"}
    }]
  }
}
```

An optional second case may use `driverId: "claude"`, with its own **distinct**
three registered seats/worktrees and a different already logged-in instance.
Each case's A/B pair shares one instance. The test must own those test seats
exclusively; it does not register or create them. The script creates no
instruction file. Any later instruction-file challenge must use only
authorized non-secret test worktrees and original H/A/tool facts, and must not
promote a model's omission of a marker to proof of non-loading.

```powershell
& $SignedNode tools/e2e/m2-memory-win11.mjs $FreshPrivateFixture
```

The script uses the existing hard installed-WebView locator and
`tester-army/e2e` with telemetry disabled and zero `agent.act`. Every original
request is saved before dispatch; `UNKNOWN` is never replayed. Success requires
actual H stop facts and admission release, normal caption close, immutable
`mode=ro&immutable=1` readback with empty WAL, and the same installed bytes on
restart. An error retains the request and running product for Controller
inspection; there is no forced stop. The private journal's
`DIRECT_CROSS_PROJECT_FACTS_REVIEW_REQUIRED` state is not acceptance.

Reference: the existing history module/reader, `product-cdp.mjs`, the fixed
Claude adapter's launch references, GOGO PARTY seat runtime, NaveHQ and LoomOS
instruction-file examples, and the [Claude Code memory documentation](https://code.claude.com/docs/en/memory).
The older projects have no qualified installed-product V04b readback to copy;
the original history reader is reused without changing its evidence standard.
