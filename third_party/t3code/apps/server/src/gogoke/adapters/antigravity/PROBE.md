# Antigravity CLI adapter evidence

## Pin

- Product: official `agy` headless CLI, not the Antigravity IDE or third-party ACP bridge.
- Fixed version for this adapter: `1.2.11`, official tag `1.2.11`, release commit `6dadd62`.
- The official GitHub repository contains release metadata and a changelog; the CLI implementation is distributed as a binary and is not in the public source tree.
- The repository research note `docs/research/2026-09-26-antigravity-cli-facts.md` records a real 1.2.11 Windows CLI investigation. It reports stream-json input/output, persistent multi-turn context, explicit `--conversation` resume, and that a second message is queued until the current turn ends. This adapter did not rerun that CLI or import an output transcript as a golden fixture.

## Capability state

| Capability           | State                                       | Boundary                                                                                                                                                                                                                                                                                                                 |
| -------------------- | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Headless NDJSON      | Research indicates supported                | `--input-format stream-json` and `--output-format stream-json`; adapter forwards raw events. Adapter E2E remains `NOT_RUN`.                                                                                                                                                                                              |
| Conversation resume  | Research indicates supported by explicit ID | Never use `--continue` or a workspace cache fallback. Resume after forced mid-turn stop remains `NOT_RUN`.                                                                                                                                                                                                               |
| In-turn steer        | `UNSUPPORTED`                               | The researched 1.2.11 behavior queues messages until the active turn ends. There is no verified headless control message.                                                                                                                                                                                                |
| Interrupt and resume | Host-mediated, real path `NOT_RUN`          | H must stop the child under native custody, record confirmed stop, retain the same admission and relaunch the same conversation ID. `result.status=CANCELED` is not process-stop proof.                                                                                                                                  |
| Native question card | `NOT_OBSERVED_UNSUPPORTED`                  | Raw tool/permission events may be passed through; this adapter does not translate them into a supported native question card.                                                                                                                                                                                            |
| Vendor memory off    | `UNSUPPORTED`                               | No verified 1.2.11 switch disables Antigravity conversation history or account-side memory. A private instance home only scopes local CLI state.                                                                                                                                                                         |
| Instruction files    | May be loaded                               | Official Antigravity rules docs list workspace `AGENTS.md`, `GEMINI.md`, `.agents/rules/`, plus global files under `~/.gemini`; rules can be cumulative. The home redirect isolates global paths but does not suppress workspace instructions. Exact fixed-1.2.11 instruction loading remains `NOT_RUN` by this adapter. |
| Account separation   | `UNKNOWN`                                   | Home redirection is not evidence of Windows Credential Manager isolation. No per-instance keyring override was found in the reviewed official material.                                                                                                                                                                  |

## Not run

- Adapter-to-real-CLI golden protocol sample: `NOT_RUN`; no golden event fixture is fabricated.
- Adapter end-to-end against official `agy` 1.2.11: `NOT_RUN`.
- Forced-stop resume after an interrupted active turn: `NOT_RUN`.
- In-turn steer: not claimed supported.
- Login, credential reads, CLI installation, model sessions, and account state probes: `NOT_RUN` by instruction.
- LPAC process launch, trusted Windows stop proof, and installed-product behavior belong to H/F integration and remain `NOT_RUN` here.

## Version drift recorded, not adopted

The official release list now contains versions newer than the pinned 1.2.11 baseline. Release 1.2.14 adds a `queuedMessages=send-immediately` setting; the official release note does not establish its behavior in headless streaming mode. The current latest release is 1.2.16. Neither change is used to upgrade this adapter or claim in-turn steer.

## References

- [Official Antigravity CLI 1.2.11 release](https://github.com/google-antigravity/antigravity-cli/releases/tag/1.2.11)
- [Official 1.2.11 changelog](https://github.com/google-antigravity/antigravity-cli/blob/1.2.11/CHANGELOG.md)
- [Official headless CLI documentation](https://antigravity.google/docs/cli/headless/)
- [Official Antigravity rules documentation](https://antigravity.google/docs/rules/)
- [Official installation and authentication documentation](https://antigravity.google/docs/cli/install/)
