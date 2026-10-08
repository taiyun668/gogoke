# PR77 native conversation consumer integration

## Changed

- Read back Owner merge `bdef08ba0b86dd291c5d3d3b66d46d954063ba1c` before editing the four added paths. Trusted main verifier recomputes `b2d36f6d55865d1c40168457dbd04989669aaec9836c1a065bcc58071457fabd`; receipt blob is `040be1d40ee317bd6c5960cbf83df7989dfc00b5`. Committed MANIFEST bytes and the three signer bindings agree.
- Integrated main and the native visible lifecycle/notification branch. New actions require the latest workspace selection in the original immediate transaction; existing request recovery keeps its historical action/selection/H/A records.
- The three authorized hooks identify the actual native attachment, omit legacy runtime/payload overrides, and hydrate complete history with read-only `thread/read`. A saved choice without an attachment refuses explicitly. Native empty history replaces stale local items; original failures reach the conversation.
- PR54 finding 1 remains fixed by `e53351a7`: no unconditional Now placeholder without a host Now fact. This deletion remains in installed 0.1.40; that installed evidence covered initial Home, not every conversation body.
- Prepared 0.1.42 from these real product changes and current authorization. Retained 0.1.41 source/artifacts; the old source predates this receipt's introducing merge and will not be signed under the new authorization.

## Result and next step

Signed Node typecheck and 159 existing affected JS tests pass. The first JS run exposed a missing new export in legacy mocks (20 failures); hooks now query native transport only in the actual Tauri runtime, preserving browser-preview behavior without changing test paths. The initial receipt-reading instrument used the wrong subprocess text encoding; raw bytes decoded as UTF-8 establish the readback. These instrument failures are not product passes.

Fresh Sol focused static review found saved choice being mistaken for an attachment; corrected and re-reviewed with no remaining finding in that bounded scope. Native build/test and installed native conversation execution remain NOT_RUN for this source. Old-source cloud jobs remain exact component evidence only. Next: current-source cloud integration checks, then the stable candidate chain and real USER conversation/managed CLI end-to-end using the retained authenticated test instances.

## Reference

Used main AGENTS/routing, the PR77 exact consumer extensions, existing hooks/hydration, native USER choices/select and full history, original effect transactions and H/A receipts, and the previously reviewed M3 notification/StopFact implementation. Reused these actual sources rather than creating a second renderer or model process. The independent review withdrew a proposed legacy fallback after checking the real fail-closed connect preflight. No G edits, credential reads, new login/reboot, formal-data changes, public release or milestone acceptance.

## Cloud failure and complete consumer review batch

The first integration cut `f35bdfff` passed cloud Browser (154 files, 1117 tests), but native library run `37838663856` failed before test execution: original artifact `11577136207/lib-tests-list.log` reports `E0308` at the new assertion, which passed a raw database pointer rather than the required verified connection reference. Corrected the argument without relaxing the assertion. The first run remains FAILED, not a native behavior pass.

Fresh Astra reviewed every consumer risk axis against that exact cut and returned one complete batch. In the authorized hooks, the integration now omits native list sortKey rejected by the host, caches only successfully reconciled exact attachments instead of reading incomplete history before every steer, separates explicit resume from read-only refresh, and lets original ordered native notifications own processing/turn state. Interrupt and UNKNOWN no longer clear state or claim physical stop; delayed ACKs do not revive a completed turn. Full history applies only if the attachment, latest read and synchronous projection revision are unchanged, including event writes still queued for React. Reset invalidates old reads/cache; empty history alone does not prove H idle.

Fresh Sol focused static re-review found no remaining known finding or direct regression. Signed Node typecheck and 159 existing affected JS tests pass after the batch. These are not native/Win11 or acceptance evidence; the corrected source must rerun cloud checks. Reference for the revision guard is the existing read invalidation pattern in the integrated panels plus the actual thread dispatch/read/event sources; it changes no event/type file or authority contract.

Latest source review at `5eb14fe8` found one further list integration defect: the old global-index consumer queried one workspace but replaced lists for all targets. Actual native history is workspace scoped. Native and unresolved targets now read singly; proven LEGACY/REMOTE targets retain the original combined query and shared-path tie-break. Failed reads do not clear other lists. Fresh Sol checked both native isolation and legacy shared-root compatibility; typecheck and the same 159 existing tests pass. The native tree and native CI inputs are unchanged by this final TS-only adjustment, so the exact `5eb14fe8` native library/LPAC run is retained for component qualification, never relabelled as the new source's entire run.

Ordinary Win11 closed-product baseline is PASS: formal five groups and original instance/custody identities unchanged, no credential bytes read. The one completed baseline task was matched to its original action, script hash, Interactive/Limited principal and successful result, exported, removed and reread absent; evidence files remain. New source desktop/bytes/sign/install and actual native conversation execution still require their own results.
