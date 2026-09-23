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

## Native-host locked Rust dependency inventory

The following external crates are pinned by `apps/desktop/native-host/Cargo.lock`. License expressions come from the corresponding crates.io version metadata; checksums are the lockfile's package checksums. This records source identity and declared terms, not a completed installer-license or legal-compliance review.

| Crate and role | Declared license | Locked checksum |
| --- | --- | --- |
| [`ryu-js` 1.0.3](https://crates.io/crates/ryu-js/1.0.3), runtime dependency | Apache-2.0 OR BSL-1.0 | `04d056b875a9d2e6cb9a61d127afee9ac5999b9f87bcb32079d1318e505be714` |
| [`cc` 1.4.2](https://crates.io/crates/cc/1.4.2), build dependency | MIT OR Apache-2.0 | `5d262e149917187838d5b42777c8253bcb64500067342904e7d429499a6f277e` |
| [`find-msvc-tools` 0.1.10](https://crates.io/crates/find-msvc-tools/0.1.10), transitive build dependency of `cc` | MIT OR Apache-2.0 | `26b73573e6edcd2af0cdf47bd6cb58f0b3839491263c314eaad1ccf24430e1de` |
| [`shlex` 2.0.1](https://crates.io/crates/shlex/2.0.1), transitive build dependency of `cc` | MIT OR Apache-2.0 | `f8fadd59c855ef2080decdef8ff161eb6661b86933c9d82e5ba29dc602a55aba` |

Whether the distributed installer includes every required license text and attribution remains a separate release review item; this inventory alone does not authorize signing or publication.
