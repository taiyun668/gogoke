# CLI protocol capture comparison

This utility imports already recorded native RPC frames and the matching product-ledger event batch into a sanitized, private bundle. It never launches the CLI, performs login, reads credentials, or changes the source capture. Input files and generated bundles must remain outside the repository. The output stores input hashes and protocol observations, not source paths or copied raw payloads.

## Import a capture

Run with the signed Node runtime used by the repository. Pass absolute paths to private local evidence and choose a fresh output path outside the repository:

```powershell
& $SignedNode tools/e2e/cli-protocol-golden.mjs import `
  --frames $PrivateFrames `
  --normalized $PrivateLedgerBatch `
  --failure-evidence $PrivateToolError `
  --direction in `
  --outcome failed `
  --capture-id cli160-m1-failed-01 `
  --cli-version 0.160.0 `
  --binary-sha256 <official-cli-binary-sha256> `
  --helper-sha256 <official-code-mode-host-sha256> `
  --source-sha <official-fixed-source-commit-or-tree-sha> `
  --out $PrivateOutputBundle
```

For a new capture, omit `--direction` only when each source frame includes its own `direction` (`in` or `out`). With legacy direct-frame records that lack that field, provide the recorded direction explicitly. Do not infer direction from a method name. Optional `commands` are treated as outbound only when their `phase` is `WRITTEN` or `OBSERVED`; other command phases are counted as excluded, unproven records. Inbound `frames` and outbound `commands` retain their own order; the importer does not invent cross-direction interleaving. `--failure-evidence` is required for `--outcome failed`; it may be omitted for `success` or `unknown`. The failure outcome remains a property of the capture and never establishes an accepted baseline.

Inputs accept the earlier `gogoke.37.private-direct-frames.v1` plus `gogoke.37.actual-product-api-batch.v1`, and the direct-readback schema `gogoke.37.private-e2e-ledger.v1`. Both require `credentialReads: false` and `databaseWrites: false`; `count` is checked when supplied. Each `originalFrame` is parsed as the actual recorded JSON object. Direct readback normalized updates are read from `sessions[].normalized[]`; the raw-frame key comes from `update._meta.rawSourceCursor`, scoped by session and source epoch. The row's `cursor` and `sourceCursor` are preserved as separate ledger ordinals and are never used as the raw-frame cursor. This distinction follows `v37_output.rs`, which writes `rawSourceCursor` from the captured frame and records the normalized ledger `source_cursor` from `ordinal`. Older events with only a raw cursor link only when it identifies exactly one recorded frame; ambiguous and unmatched links are reported.

## Compare captures

The current readback joins normalized rows to the unique original
`raw.resolved_event_id = index.source_event_id` relation, carrying operation,
generation and process custody. Raw ordinals can repeat after compact or renew
inside one host epoch; they are disambiguated by the recorded physical scope.
RPC pairing uses session, operation, generation, process ticket and custodian
nonce when available. `WRITTEN` answers legitimately have no observed source
epoch/cursor, so those nullable observation fields are not custody identifiers.
Legacy captures without custody retain the stricter recorded-epoch association;
missing or ambiguous associations remain explicit. These are measurement fixes
on existing bytes, without replaying a product or CLI request.

```powershell
& $SignedNode tools/e2e/cli-protocol-golden.mjs compare `
  --baseline $PrivateBaselineBundle `
  --candidate $PrivateCandidateBundle `
  --out $PrivateDiffReport
```

The report compares direction and per-direction frame order, method and item-type counts, field additions/removals and type changes, normalized event order, selected semantic scalar changes (such as `status` and `sessionUpdate`), request/response associations, and direction/link coverage. Request/response pairs require an actual RPC id, opposite directions, and the same session, operation, generation, and source epoch. Missing scope, ambiguous pairs, and unmatched requests remain explicit. It reports changes; it does not decide compatibility. Every report has `acceptance: NOT_ASSESSED`, including an identical self-comparison. The Controller reviews the direct evidence and differences before choosing a regression capture; milestone acceptance remains with the Owner.

## Privacy and baseline status

Dynamic session, thread, turn, item, request, cursor, and related identifiers receive stable category aliases within an imported bundle. Free-form strings are replaced with `[REDACTED]`; semantic protocol strings such as method, type, status, and session-update names are retained. URL-like values and machine paths are also redacted. The manifest lists redacted field pointers and stores hashes of the private source files, never their paths. Keep generated bundles private because structural observations can still reveal protocol shape.

The importer records missing inbound or outbound directions, unmatched JSON-RPC requests, and raw-frame-to-ledger link coverage. A failed capture is always marked `NOT_READY_FAILED_CAPTURE`; an incomplete direction set is `NOT_READY_MISSING_DIRECTION`; a complete non-failed capture is only `REVIEW_REQUIRED`. These are readiness descriptions, not pass/fail compatibility verdicts.

The controller-provided CLI 0.160.0 sample used for the first tool check contains three inbound notifications and no outbound commands. Its associated model/tool result failed while starting `code-mode-host`; it is therefore a diagnostic failure capture, **not a golden success baseline**. This sample is intentionally not checked into the repository. A future success baseline needs a real successful capture with both directions, reviewed source/binary hashes, adequate raw-to-ledger correlation, and the Controller's evidence review outside this script. This adds no Owner step for test recordings and does not accept the product.

## Reuse references

- `tools/protocol-conformance/README.md` documents shared protocol observations, input provenance, explicit gaps, and the rule against silently normalizing outcomes.
- `tools/protocol-conformance/harness.py` has a deterministic cross-driver relay with explicit per-axis gaps; its synthetic conformance inputs are not substitutes for direct CLI frames.
- `docs/research/adapter-spike/02-mvp-adapter-decision.md` records protocol pinning, native identifier mapping, privacy filtering before observations, and terminal/warning semantics.
- `docs/research/grok-app-reuse-audit.md` recommends reusing fixtures with negative cases rather than relying only on a happy path.
- NaveHQ's `scripts/navehq_parse_codex_run.js` demonstrates deriving a structured report from an existing run; this utility deliberately does not retain source paths.
- LoomOS's `03_specs_施工图/应答地基施工图-v0.2.md` separates ordinary golden behavior cases from high-risk expectations; this utility likewise keeps the failed diagnostic separate from any accepted baseline.
