# Third-party source notices

The repository license covers gogoke-owned source. Included third-party source and resources keep their original notices and terms:

| Component | Source and terms | Preserved notice |
| --- | --- | --- |
| CodexMonitor desktop foundation | Dimillian/CodexMonitor, imported at `8a2dd1f87c42a0b331f5a243b9752cc80adb763a`, MIT, Thomas Ricouard | `apps/desktop/LICENSE`, `apps/desktop/THIRD_PARTY_NOTICES.md` |
| T3 Code donor | pingdotgg/t3code at `d6f291303ddc0c9a14f570266a4d9eff6d431593`, MIT, T3 Tools Inc. | `third_party/t3code/LICENSE` |
| SQLite amalgamation | SQLite 3.53.2, public domain; pinned source and hashes are recorded with the vendored copy | `apps/desktop/native-host/vendor/sqlite-3.53.2/SOURCE_IDENTITY.json` |
| Alchemy Effect donor subtree | Apache-2.0, Functionless Corp.; its own third-party attribution includes MIT components | `third_party/t3code/.repos/alchemy-effect/LICENSE`, `NOTICE`, `THIRD_PARTY_LICENSES.md` |
| Effect Smol donor subtree and packages | MIT, Effectful Technologies Inc.; package-specific notices are preserved | `third_party/t3code/.repos/effect-smol/LICENSE` and each `packages/**/LICENSE` |
| Ghostty terminal source | MIT, Mitchell Hashimoto and Ghostty contributors | `third_party/t3code/native/libghostty-vt/LICENSE` |
| Ghostty terminal font resource | MIT, Ryan L McIntyre | `third_party/t3code/apps/web/src/terminal/ghostty/fonts/LICENSE` |
| T3 mobile composer editor | MIT, 650 Industries Inc. / Expo | `third_party/t3code/apps/mobile/modules/t3-composer-editor/LICENSE` |
| T3 mobile Markdown text | MIT, Bluesky PBC | `third_party/t3code/apps/mobile/modules/t3-markdown-text/LICENSE` |

All donor `LICENSE`, `NOTICE`, and `THIRD_PARTY_LICENSES.md` files present in the source tree remain in their original locations. The export omits donor `node_modules` fixtures, nested agent skills, a symlink to agent skills, and test private-key fixtures; those omissions are recorded in the local migration report and do not change the license of retained source.
