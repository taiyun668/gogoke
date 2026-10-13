# Installed-candidate observation

Use these shared instruments for subsequent cases. Do not generate a new case-specific
SQLite reader, protection comparison, readiness loop or close observer.

| Observation | Shared entry | Case responsibility |
|---|---|---|
| Readiness and normal close | `ActualProduct` delegates to `installed-lifecycle-observation.mjs` | Supply the actual installed candidate and preserve its original journal. |
| Normal close of the retained failed continuation | `retained-installed-close.mjs` via `run-retained-installed-close.ps1` | Pin the original held launch and keep its parent driver alive. The adapter reads that parent's actual child-exit receipt; the same shared close observer still checks original OS handles and both exit codes. |
| StopFact and seat relationship | `installed_native_readback.py` | Supply pinned original requests, the observed binding mode and the original stopped source. |
| Root identity before an operation | `installed_native_readback.py`, `kind: root_identity` | Supply the original stopped proof; call before claim and immediately before the operation. This metadata-only mode does not open SQLite and can observe a live root. |
| Three observers / thirteen fields | `installed-protection-observation.mjs` for actors; `capture-protection.ps1` for pinned standalone capture or retained-snapshot verification | Supply the original observer definitions and actual before/after references. Both use `installed-protection-fields.json`. |
| Instrument qualification | `installed-observation-verify.mjs` via `run-installed-observation-job.ps1` | Bind a private job to the actual runtime, executed instrument and original scene bytes. |

`m2-win11`, `m2-extra-win11`, `m2-provider-win11` and `m2-v06-remaining-win11`
delegate protection capture and comparison. The current stop-worktree and V06
remaining readers delegate their H relationship and StopFact checks. Other historical
readers retain their original bytes and evidence; port their existing case semantics
to these functions before using them for a new case. A resume or Secretary case must
not be forced through the ordinary five-request admission/open/stop/release shape.

The ordinary readback requires the same root identity, original immutable relationship,
five original raw requests/CAS receipts, process episode/custody/claim StopFact and
physical F identity. Native V2 authorization generation, physical H generation and
current E generation are separate facts. Explicit legacy mode never falls back from a
missing native row; a retained `LEGACY_V1` initial relationship is checked separately.
Secretary readback requires STOPPED and no release for the measured generation.

The native CLI opens only the original closed database with immutable/query-only
SQLite, checks bytes and root identity before and after, and never copies the database
or reads credentials. Outputs remain outside the installed state. Caller pins and
reported loaded-source hashes identify the instrument actually used. The root-identity
mode compares canonical path, Windows physical file ID and stat identity to the
original stopped proof. It measures the same root twice and refuses disagreement.

Readiness still uses the original 180-second deadline. A valid receipt published after
an original failed decision remains a failed launch; its measured publication time
does not grant a grace period or trigger a second signal. Normal close uses the
original shell/native handles, creation times, image identities, caption event and
both `GET_EXIT_CODE_PROCESS` results. PID absence is not an exit-code observation.

Qualification uses retained successful and failed scenes, with explicit source paths
provided privately. Node actual-scene tests fail when these inputs are absent. The
runner requires TAP pass counts and zero skipped tests, pins the executed entry and
accepts the expected original-source refusals only with their actual error text.
An instrument qualification result does not make a failed business case pass and
does not replace a new candidate's actual product validation.
