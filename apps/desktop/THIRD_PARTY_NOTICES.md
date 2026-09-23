# Third-party source notice

Parts of the initial desktop implementation were derived from CodexMonitor,
copyright Thomas Ricouard, under the MIT License reproduced in `LICENSE`.

Source used for the one-time import:

- Repository: https://github.com/Dimillian/CodexMonitor
- Base commit: `dd61b9abd37de5ded86e82b9fe8a83fd49d46fa5`
- Imported commit: `8a2dd1f87c42a0b331f5a243b9752cc80adb763a`
- Import date: 2026-09-15

This notice records license provenance only. gogoke has its own package
identity, configuration, release channel, telemetry boundary, update trust
root, CI, and product roadmap. It does not fetch or merge changes from that
repository at runtime or during release builds.

The desktop source also includes T3 Code from `pingdotgg/t3code@d6f291303ddc0c9a14f570266a4d9eff6d431593` under MIT (`../../third_party/t3code/LICENSE`) and the pinned SQLite 3.53.2 amalgamation in the public domain (`native-host/vendor/sqlite-3.53.2/SOURCE_IDENTITY.json`). The repository-level `../../THIRD_PARTY_NOTICES.md` inventories other retained donor source and resources.

## Native-host locked Rust dependency inventory

The following external crates are pinned by `native-host/Cargo.lock`. License expressions come from their crates.io version metadata; checksums are the lockfile's package checksums. This file is included in the desktop bundle and portable artifact. The inventory records source identity and declared terms, not a completed legal-compliance review.

| Crate and role | Declared license | Locked checksum |
| --- | --- | --- |
| [`ryu-js` 1.0.3](https://crates.io/crates/ryu-js/1.0.3), runtime dependency | Apache-2.0 OR BSL-1.0 | `04d056b875a9d2e6cb9a61d127afee9ac5999b9f87bcb32079d1318e505be714` |
| [`cc` 1.4.2](https://crates.io/crates/cc/1.4.2), build dependency | MIT OR Apache-2.0 | `5d262e149917187838d5b42777c8253bcb64500067342904e7d429499a6f277e` |
| [`find-msvc-tools` 0.1.10](https://crates.io/crates/find-msvc-tools/0.1.10), transitive build dependency of `cc` | MIT OR Apache-2.0 | `26b73573e6edcd2af0cdf47bd6cb58f0b3839491263c314eaad1ccf24430e1de` |
| [`shlex` 2.0.1](https://crates.io/crates/shlex/2.0.1), transitive build dependency of `cc` | MIT OR Apache-2.0 | `f8fadd59c855ef2080decdef8ff161eb6661b86933c9d82e5ba29dc602a55aba` |

Whether the distributed installer includes every required license text and attribution remains a separate release review item; this inventory alone does not authorize signing or publication.
