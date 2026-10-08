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
