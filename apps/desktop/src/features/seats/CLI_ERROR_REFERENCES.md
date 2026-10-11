# G.0 runtime CLI error display

The instance page accepts optional `runtimeIssues`. Each entry carries the
current affected `seatId`, `sessionId`, `generation`, original `reason`, and
`sourceEpoch`/`sourceCursor`. Missing and empty arrays produce the same page.
The host supplies these facts only from the current instance/seat/generation
and H/A's captured typed CLI failure. This view passes them through; it does
not parse model prose, infer a login failure, change the instance state, or
start an upgrade, instance switch, or Owner popup. A malformed present field
fails decoding instead of rendering a fabricated error.

The existing instance row uses `settings-help` and its error color token. The
seat and full original reason remain visible with natural wrapping; native
`details` holds the session/generation and source locator for inspection.
`role="status"` announces the asynchronously observed error without the
intrusion of a modal or repeated polling alert. The same login and catalog
version components remain in their original places. The browser-only K-UI
fake has an explicit two-seat CLI error toggle; it is no native observation.

References read: historical gogo-party `packages/room/src/server.ts` retains
seat startup `lastError` in the seat event around line 4282, and
`packages/room/src/cli-input-page.test.ts` checks that an error refreshes its
feedback. This is a feedback pattern, not an H/A provenance source. The
repository's G.0 plan and current `Design37InstanceSection`/Settings styles
own the page structure; no new design system or CSS is introduced. The
targeted `ui-ux-pro-max` UX search `error feedback accessible` returned
near-problem feedback and accessible announcement guidance, applied with the
existing row and `role="status"`. Root's native list producer owns exact
current-instance H/A filtering and original typed-error evidence. UI syntax,
HTTP serving and fake previews cannot establish installed-product behavior.
