# gogoke Windows runtime verification — 2026-09-15

Candidate: `8d022076c3c46222b115d13d5cbc264c3a4454bd`.
This evidence records a local-only executable check, not release acceptance.

## Artifact integrity

- GitHub run `35001435470` reports the candidate HEAD above.
- Artifact `10409654923`: `gogoke-windows-local-only-unsigned-executable`.
- Download directory: `%USERPROFILE%\Downloads\gogoke-run-35001435470-8d02207`.
- `SHA256SUMS.local-only` and the downloaded and installed executables agree:
  `34fd88b2d0a4788c94aec8019bd7de38559fa380f9100d30056711ef3ef6c872`.
- Authenticode status: `NotSigned`.
- Previous installed executable retained as `gogoke.previous-3efd8a5.exe` in the download directory.

## Effective configuration and runtime

`node apps/desktop/scripts/check-product-identity.mjs` exits 0 on this candidate.
This checks the effective configuration; it does not itself prove runtime behavior.

The installed executable was replaced while gogoke was stopped, then launched.
Process 24708 reported title `gogoke` and executable path
`%USERPROFILE%\AppData\Local\Programs\gogoke\gogoke.exe`.
Read-only `DwmGetWindowAttribute` attribute 38 returned value 3, HRESULT 0.
The original Codex Monitor process 26424 also returned value 3, HRESULT 0.

Both desktop shortcuts retain their original targets. The original Codex Monitor
shortcut targets the OpenAI.Codex package LocalCache executable. Its process API
reported `%USERPROFILE%\AppData\Local\Codex Monitor\codex-monitor.exe`, while the
Computer Use window identity reported the package LocalCache path. These differing
path representations were observed; this report does not infer their mapping.
No original installation or shortcut was edited.

## Visual observation

Computer Use captured the actual gogoke window and the original Codex Monitor
window. Both visibly show blurred colored background through their content area.
The former opaque-white gogoke failure is not present in these observations.
Screenshots are retained locally in the exact-run download directory as
`gogoke-runtime.png` and `codex-monitor-reference.png`; they are excluded from Git
because the UI includes local usage information.

These are separate window captures at different sizes, not a single simultaneous
side-by-side screenshot. Glass restoration is observed; exact visual parity across
all layouts is not claimed.

Owner subsequently confirmed: "已经恢复玻璃透视了". The glass-restoration
visual acceptance is therefore PASS; this does not extend to updater acceptance.

## Pending CI and next boundary

At 2026-09-15 17:55 UTC, runs `34994928895` (3efd8a5) and `35001435470`
(8d02207) both remain in progress at full update/readiness/rollback smoke.
Both browser jobs and local-only uploads succeeded. Neither run was cancelled or
rerun. Full updater/release acceptance remains pending.

Read-only inspection identifies unbounded installer/uninstaller waits, but no
live smoke subphase or failure log establishes the current delay's cause.
Do not repair a guessed cause or treat an in-progress smoke as a pass.
Generated-icon defaults and portable update semantics remain deferred until the
current CI is resolved, following the handoff's execution order.

Main checkout remains at `179ad7f` with its two modified design logs and two
untracked paths unchanged. All task work is in the independent Codex worktree.
