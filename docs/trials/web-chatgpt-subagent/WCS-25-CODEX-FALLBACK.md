# WCS-25 Codex fallback: public-source scanner audit

- Review commit: `8b04a707b2d9f3b4163193aaa796258cb17bb946`.
- Web status: the Controller clicked the visible Send control once, but the new-chat page retained the entire composer and no new user or assistant turn appeared. The ledger recorded `uncertain_submission`; no retry, ChatGPT reply, GitHub result branch, or connector write was observed. This report is **Codex Astra fallback work**, not GPT-6 Pro output.
- Scope: the scanner, its tests, and `.github/workflows/gogoke-source-hygiene.yml` at the review commit. The fallback agent used read-only fixed-SHA source inspection and in-memory probes; the Controller independently reran the five classification probes against the same scanner bytes. No repository file was edited by the fallback agent.

## Reproduced findings

| Severity | Finding and fixed-SHA location | In-memory probe result |
| --- | --- | --- |
| P1 | UTF-16 text is treated as binary and reduced to printable ASCII runs (`tools/check-public-source.py:177–181`); interleaved NUL bytes hide an otherwise matching token. | A synthetic token encoded as UTF-16 returned no findings. A regression test should decode supported text encodings before token matching. |
| P1 | A complete Ed25519 PKCS#8 key can be 48 DER bytes, while the scanner requires 64 bytes (`tools/check-public-source.py:99–107`). | A synthetic 48-byte valid-shape PEM was classified only as `EXPLAINED marker-without-validated-block`. Test this key shape as a leak. |
| P1 | Any occurrence of `example`, `fake`, `dummy`, `test`, or `abcdef` inside a token causes a blanket synthetic classification (`tools/check-public-source.py:92–96`). | A synthetic token containing `TeSt` in its interior was classified `EXPLAINED synthetic-token`. Test a token with that substring that is otherwise credential-shaped. |
| P2 | Test-file path exemption runs before the Unix home-path check (`tools/check-public-source.py:57–71`). | A synthetic private `/home/...` path in `src/tests/config.py` was classified `EXPLAINED synthetic-test-path`. Test Unix user paths in test fixtures explicitly. |
| P2 | `rust_test_region` becomes true at `#[cfg(test)]` and never resets (`tools/check-public-source.py:82–90`). | A synthetic absolute path after a closed test module was classified `EXPLAINED synthetic-rust-test-path`. Test a post-test production item. |

These probes show classification gaps, **not** confirmed secrets in the repository. No GitHub Actions run or real credential-validity check was part of this fallback audit. The scanner fixes belong to a separate implementation scope; this web-channel PR records the findings without editing that scanner.
