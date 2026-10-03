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
JSON-RPC pairing uses session, operation, generation, process ticket and custodian
nonce when available. `WRITTEN` answers legitimately have no observed source
epoch/cursor, so those nullable observation fields are not custody identifiers.
Legacy captures without custody retain the stricter recorded-epoch association;
missing or ambiguous associations remain explicit. These are measurement fixes
on existing bytes, without replaying a product or CLI request.

Claude associations use separate protocol namespaces: `control_request.request_id`
matches `control_response.response.request_id`, and an outbound host `user.uuid`
matches the inbound original human `user.uuid` echo. IDs retain their original
string/number type and require the same complete physical scope as JSON-RPC.
An inbound `type:user` tool result, synthetic message, or subagent message is
not a human echo. A `result` without a user UUID is never inferred to be a
reply to a host input; user echo association records only an observed echo,
not a completed turn. Missing, duplicate, or ambiguous associations remain
unresolved. The existing JSON-RPC association fields and behavior are retained.

```powershell
& $SignedNode tools/e2e/cli-protocol-golden.mjs compare `
  --baseline $PrivateBaselineBundle `
  --candidate $PrivateCandidateBundle `
  --out $PrivateDiffReport
```

The report compares direction and per-direction frame order, method and item-type counts, field additions/removals and type changes, normalized event order, selected semantic scalar changes (such as `status` and `sessionUpdate`), JSON-RPC and Claude associations, and direction/link coverage. Association pairs require an explicit typed ID, opposite directions, and the same complete physical scope. Missing scope, ambiguous pairs, and unmatched records remain explicit. It reports changes; it does not decide compatibility. Every report has `acceptance: NOT_ASSESSED`, including an identical self-comparison. The Controller reviews the direct evidence and differences before choosing a regression capture; milestone acceptance remains with the Owner.

## Privacy and baseline status

Dynamic session, thread, turn, item, request, cursor, and related identifiers receive stable category aliases within an imported bundle. Free-form strings are replaced with `[REDACTED]`; semantic protocol strings such as method, type, status, and session-update names are retained. URL-like values and machine paths are also redacted. The manifest lists redacted field pointers and stores hashes of the private source files, never their paths. Keep generated bundles private because structural observations can still reveal protocol shape.

The importer records missing inbound or outbound directions, unmatched JSON-RPC requests, and raw-frame-to-ledger link coverage. A failed capture is always marked `NOT_READY_FAILED_CAPTURE`; an incomplete direction set is `NOT_READY_MISSING_DIRECTION`; a complete non-failed capture is only `REVIEW_REQUIRED`. These are readiness descriptions, not pass/fail compatibility verdicts.

M2 imports the actual `gogoke.37.private-m2-readback.v1` output with mandatory
`--session-id`. Frames, confirmed commands and normalized rows are selected only
for that original session; its F instance and H custody driver/version/binary
digest must agree with the requested CLI label. Never combine different
providers under one version or binary hash. Use the same private readback file
for `--frames` and `--normalized`; its original file hash remains recorded.
Codex still requires `--helper-sha256`; providers without that helper leave it
absent. For M2, `--source-sha` is optional and only supplies a known upstream
source identity. An unknown upstream commit remains `NOT_ASSERTED`, separately
from the actual candidate `productSourceCommit`; do not invent a vendor source
hash from the product commit or CLI binary. Zero normalized rows are retained
as `NOT_READY_MISSING_NORMALIZED_OUTPUT`. All imports still require review and
never accept a golden baseline or milestone.

The controller-provided CLI 0.160.0 sample used for the first tool check contains three inbound notifications and no outbound commands. Its associated model/tool result failed while starting `code-mode-host`; it is therefore a diagnostic failure capture, **not a golden success baseline**. This sample is intentionally not checked into the repository. A future success baseline needs a real successful capture with both directions, reviewed source/binary hashes, adequate raw-to-ledger correlation, and the Controller's evidence review outside this script. This adds no Owner step for test recordings and does not accept the product.

## Reuse references

- `tools/protocol-conformance/README.md` documents shared protocol observations, input provenance, explicit gaps, and the rule against silently normalizing outcomes.
- `tools/protocol-conformance/harness.py` has a deterministic cross-driver relay with explicit per-axis gaps; its synthetic conformance inputs are not substitutes for direct CLI frames.
- `docs/research/adapter-spike/02-mvp-adapter-decision.md` records protocol pinning, native identifier mapping, privacy filtering before observations, and terminal/warning semantics.
- `docs/research/grok-app-reuse-audit.md` recommends reusing fixtures with negative cases rather than relying only on a happy path.
- NaveHQ's `scripts/navehq_parse_codex_run.js` demonstrates deriving a structured report from an existing run; this utility deliberately does not retain source paths.
- LoomOS's `03_specs_施工图/应答地基施工图-v0.2.md` separates ordinary golden behavior cases from high-risk expectations; this utility likewise keeps the failed diagnostic separate from any accepted baseline.
- `packages/seat-runtime/src/claude-seat.ts` writes the host `user` envelope and returns control responses under `response.request_id`; frozen `third_party/t3code/apps/server/src/provider/Layers/ClaudeAdapter.ts` distinguishes human turns from tool-result `user` messages and supplies host UUIDs. The importer uses these structural distinctions but does not adopt their runtime or history storage.
- Anthropic's pinned `@anthropic-ai/claude-agent-sdk@0.3.276` `sdk.d.ts` defines `SDKControlRequest.request_id`, `SDKControlResponse.response.request_id`, optional `SDKUserMessage.uuid`, and `SDKResultMessage` separately. The importer reads raw recorded frames under these shapes; a type declaration is not a capture.

RPC alias keys preserve the original JSON ID type. Numeric `7` and string `"7"` remain different IDs, including in the same process custody. Each normalized protocol frame and correlation pair exposes `rpcIdType`, so a CLI upgrade that changes the ID type remains visible after redaction. This is an instrument rule; matching frames still do not accept a baseline or a product.
Claude control and user UUID aliases are likewise typed and isolated from each other and from JSON-RPC IDs. No demonstration frame is described as a real provider golden; actual provider goldens remain `NOT_RUN` until separately recorded and reviewed.
