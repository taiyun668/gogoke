# Owner notice presenter

The presenter displays the current native `OWNER_HOST_RULE_NOTICES` projection. It renders the native mechanical `body` as text and identifies its source seat. Closing hides that cause for the current presenter lifetime, with no operation, acknowledgement, cancellation, read marker, or C state change. Projection withdrawal removes the dialog immediately. An already presented cause is not presented again in that lifetime, including after withdrawal and restoration. A new UI lifetime can present it again.

## References and reuse

- gogo-party `packages/room/src/server.ts`: `push` persists cards into its timeline, while the state response filters the timeline to the current room. Reuse the distinction between persisted message facts and current UI presentation. This presenter does not borrow Room's data model or invent delivery facts.
- `Design37InstanceSection.tsx`: the existing G.0 instance page and browser preview establish the desktop's current tokens and fake-host iteration path. Owner notices use the same preview entry and preserve the independent instance controls.
- `../design-system/components/modal/ModalShell.tsx` and `../../styles/ds-modal.css`: reuse the existing modal container, backdrop, semantic dialog attributes, surface colors, type classes, and action layout. `ModalShell` supplies no focus behavior; this one-control dialog adds focus containment and restoration locally.
- `docs/research/source-audit/08-observability-dashboard-and-shell-sources.md`: UI/Inbox product patterns do not supply durable authority or delivery semantics. Native C remains authoritative.
- UI/UX Pro Max, `dialog keyboard focus desktop --domain ux`: the verified focus-state result requires visible keyboard focus; use the existing focus token for the close control.
- [W3C APG modal dialog](https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/): initial focus on the title supports reading the mechanical facts, Tab and Shift+Tab stay on the sole close control, Escape closes, background siblings become inert, and focus returns to the prior connected control. No new dialog library or reusable focus framework is introduced.

The shared `base.css` reduced-motion rule remains effective in the preview. Layout limits the body height and wraps long facts while keeping the close action visible.

## Global host and native source

`requestDesign37OwnerNotices(executeUserSourceOperation, requestId)` accepts an injected existing User source-operation bridge. The request uses `gogoke.37.operations.v1`, `K-INBOX/check-unknown`, `global`, literal `OWNER`, revision `0`, and the sole payload `{ "projection": "OWNER_HOST_RULE_NOTICES" }`.

The parser accepts the original K-INBOX receipt only: matching schema, family, operation, target and supplied request ID; native receipt revisions `0`; status `APPLIED`; the exact projection name and string DTO fields with `PENDING` state. A native receipt has no domain field; each notice has its own domain ID. Rejected receipts preserve their original content in the error, and bridge exceptions propagate.

`apps/desktop/src/App.tsx` mounts `Design37OwnerNoticeHost` only after `useWindowLabel("")` resolves to literal `main`. The unknown initial label and `about` never mount the notification host. Its presenter stays mounted across the main app's internal routes.

The actual native source is `apps/desktop/src-tauri/src/public_runtime/product_entry.rs`, function `gogoke_design37_user_operation`. The Host invokes this existing Tauri command with `{ frame: JSON.stringify(originalRequest) }`, parses the original JSON string, and passes the original K-INBOX receipt to the existing DTO parser. JSON parse failures preserve the returned response and parser error. Native K-UI currently appears as a closed operation catalog in `apps/desktop/native-host/src/store/session_transport/wire.rs`; this work adds no K-UI dispatcher, handler, opcode or Tauri command. The User pipe remains the sole authority for this read.

The Host reuses `Design37InstanceSection.tsx`'s immediate read plus 1-second measurement cadence and in-flight exclusion. Its local active flag blocks response updates after unmount, and cleanup removes the timer. Each read has a fresh request ID; there is no acknowledgement, cancellation, resend or mutation request. A failed or unavailable current projection clears the notice array. The exact native `GOGOKE_DESIGN37_USER_HOST_NOT_STARTED` means no retained host is available; it is retained internally without a daily-main-window toast because this is the normal pre-initialization lifecycle. Other failures retain their original text in a visible alert and offer an explicit refresh action through existing Toast primitives. No prior notice is reconstructed from a local cache.

## Browser preview and verification

The browser-only entry is `src/features/seats/preview/index.html`. The fake source is explicit and cannot acknowledge anything. Controls preserve the pure presenter scenes and also mount the actual Host against an injected fake source. Host scenes provide unavailable and raw-error states, recovery, slow responses, unmount/reopen and read statistics. In Host mode the scene controls change only fake source state; they do not issue additional parallel reads. The slow-response fixture can return a captured projection after the Host unmounts.

Actual browser checks verified mechanical text, initial title focus, Tab and Shift+Tab containment, Escape and button closure, focus restoration, no repeated cause after closure, renewed presentation after UI restart, withdrawal-driven closure, no repeated withdrawn cause after restoration, and presentation of a new cause. The preview runs with signed Node and its own port; runtime custody is retained in the Controller handoff. Browser reload was used to confirm the final changed files because automatic hot refresh did not expose the newly added control in this worktree.

Initial presenter source checks and browser evidence remain specific to the initial presenter package. For the global Host, signed Node typecheck passed and the running fake preview served the current Host and main modules. A one-off source DOM check loaded the actual Host, actual fake source and actual frontend Tauri serialization with a fake bridge: original command and read-only request shape, closure, new causes, exact unavailable status, original failure plus refresh, maximum concurrency of one, timer cleanup and ignored late response after unmount, and presentation in a fresh Host lifetime all passed. No new non-security unit test file was added.

The worker's source-DOM checks were followed by Controller browser verification of the actual fake Host preview: automatic projection read displayed the source-bound body, initial title focus moved to Close with Tab, and Escape restored the triggering control. This preview used the worker's served source, before the Controller's normal-unavailable-toast adjustment; it is not proof of that later adjustment or any installed application. The fake preview is **NOT_ACCEPTANCE**, with runtime custody retained in the handoff.

Native host execution, native build on the Owner machine, real CLI/model/DB actions, real application windows, persistent Owner ACK, installed-candidate validation, milestone acceptance and release are **NOT_RUN**. Browser fake results are **NOT_ACCEPTANCE**. C remains PENDING and E remains EINTENT; G presentation supplies no durable fact.
