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

## Controller glue

`requestDesign37OwnerNotices(executeUserSourceOperation, requestId)` accepts an injected existing User source-operation bridge. The request uses `gogoke.37.operations.v1`, `K-INBOX/check-unknown`, `global`, literal `OWNER`, revision `0`, and the sole payload `{ "projection": "OWNER_HOST_RULE_NOTICES" }`.

The parser accepts the original K-INBOX receipt only: matching schema, family, operation, target and supplied request ID; native receipt revisions `0`; status `APPLIED`; the exact projection name and string DTO fields with `PENDING` state. A native receipt has no domain field; each notice has its own domain ID. Rejected receipts preserve their original content in the error, and bridge exceptions propagate.

Controller owns the global mount, existing K-UI to User bridge mapping and refresh lifecycle. Keep `Design37OwnerNoticePresenter` mounted across app routes and pass only the latest successfully read current projection. Replace notices with an empty array when that projection disappears or becomes unavailable; do not retain a stale array on failure. Preserve the original failure for the existing error surface. This package creates no Tauri command, family, opcode, or polling mechanism.

## Browser preview and verification

The browser-only entry is `src/features/seats/preview/index.html`. The fake source is explicit and cannot acknowledge anything. Controls show a cause, withdraw its route, show and withdraw a new cause after three seconds, create a new cause, or restart the presenter lifetime.

Actual browser checks verified mechanical text, initial title focus, Tab and Shift+Tab containment, Escape and button closure, focus restoration, no repeated cause after closure, renewed presentation after UI restart, withdrawal-driven closure, no repeated withdrawn cause after restoration, and presentation of a new cause. The preview runs with signed Node and its own port; runtime custody is retained in the Controller handoff. Browser reload was used to confirm the final changed files because automatic hot refresh did not expose the newly added control in this worktree.

Source checks: typecheck and the 14 existing related instance, fake-host and ModalShell tests passed. Browser source build passed. The already started existing full frontend suite completed with 148 files and 1056 tests passing; no new non-security unit tests were added. Final typecheck covers the subsequent presenter lifetime adjustment, and the final browser reload verifies that adjustment directly.

Native host execution, native build on the Owner machine, real CLI/model/DB actions, real application windows, persistent Owner ACK, installed-candidate validation, milestone acceptance and release are **NOT_RUN**. Browser fake results are **NOT_ACCEPTANCE**. C remains PENDING and E remains EINTENT; G presentation supplies no durable fact.
