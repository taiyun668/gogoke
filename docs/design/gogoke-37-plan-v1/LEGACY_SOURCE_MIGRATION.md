# Codex M1 legacy SOURCE migration proposal

Status: `PROPOSED_NOT_AUTHORIZED`. This document defines a narrow authorization contract for review. Owner permission covers preparing and sending the proposal for independent Claude review; it does not delegate merge or execution. This is not product code, does not qualify the symbolic case, and does not permit a database write before independent Claude review, a separately authorized main merge, and exact receipt/scope readback.

## Goal and scope

The only target is the single existing, non-tombstoned, disabled Codex M1 legacy instance whose current SOURCE row is absent. This public proposal names that target `LEGACY_CODEX_M1`; the private instance identifier, HOME/FileID and account observations remain outside the repository. Native selection must independently prove that the complete set of non-tombstoned Codex instances with an absent SOURCE row contains exactly this one disabled legacy candidate. The approved target's original registration and HOME anchors must exact-match the new request and journal; native resolves and validates those anchors from the original database, while caller values are only lookup and comparison keys. Current cardinality alone never identifies the approved target. Zero, two or more candidates, an enabled candidate, changed original anchors, or a caller-selected substitute refuses without selecting another candidate.

The migration may add provenance for the already registered same-pin instance. It does not install, upgrade, roll back, enable, log in, launch or stop an instance. It does not promise that a later version action will pass. After a successful migration, the ordinary native managed-CLI guards still decide every begin, stage, probe, switch and rollback.

## Classification before migration

Before this route can be considered, native code must evaluate the existing exact registration replay preconditions read-only against the original schema, raw bytes, APPLIED operation/native receipt, HOME and pin chain. The current implementation has no independent SOURCE-bind journal: an APPLIED registration may continue to a separate SOURCE insert, so a missing journal cannot distinguish legacy from replay.

1. `ELIGIBLE`: the original request satisfies the actual existing replay and SOURCE-bind preconditions. Only that existing exact original-request route may be considered; this migration refuses.
2. `PROVEN_INELIGIBLE`: complete original records positively prove which actual replay precondition cannot apply, independently of SOURCE absence or missing evidence. Only with separate positive proof of the complete legacy registration and every migration condition may this route proceed.
3. `UNKNOWN`: missing or ambiguous original schema/raw/receipt, an unproved predicate, query failure or inability to distinguish the histories refuses. A failed trial replay, old name/version, APPLIED label, absent SOURCE or missing record is not proof of ineligibility. Do not dispatch registration or write SOURCE to test classification.

Finding original records that satisfy the real replay predicates falsifies `PROVEN_INELIGIBLE` and selects the existing route. Finding any current SOURCE row, a second candidate or an original registration/HOME/pin mismatch falsifies the proposed case. The supplied historical summary does not expose the full original registration raw/receipt, so the actual M1 replay classification remains `UNKNOWN`; this proposal does not qualify it.

## Required direct evidence

All evidence is read from the original verified ProductDatabase/RootLock and trusted native producers. A report, operator string or caller assertion is not evidence.

- The original registration request ID and exact raw fingerprint, its APPLIED operation row, native receipt ID and receipt bytes, target instance row, and unchanged original outcome.
- The canonical root and database identities plus the instance's exact registered HOME identity, current physical HOME FileID/marker/SID, and unique original registration-to-HOME association.
- The current compiled official Codex catalog entry and the current managed stage's archive, image, executable, version and source-chain identities. They must match the legacy instance's existing program digest and version exactly. A same name, PATH lookup, latest notice or caller path is insufficient.
- A complete affected-resource snapshot: the instance/profile/SOURCE-absence rows and revisions; original registration operations, raw/native receipts and actual replay inputs; current managed-copy row and revision; every claim, owner binding, session/generation, H/C custody record, alias, temporary HOME, pending instance operation and managed version action that can affect this instance or the shared current-stage selection. Do not invent a historical SOURCE-bind journal or infer a negative fact from its absence.
- Every affected execution is settled by its original StopFact/StopProof or by the already authorized fresh typed native fact over the complete original execution identity set. Exact PID plus full creation FILETIME or a directly established kernel-restart cutoff may prove the old objects gone. Timeout, age, grace period, window close, empty caller list, PID alone or ACL-only history cannot.
- Root/database/HOME/stage identities, source absence, candidate cardinality and all row revisions are re-read immediately before the write. Any drift, live or unqualified object, query denial, uncertain identity, incomplete family or original-error loss refuses.

Unrelated historical UNKNOWN and original operation rows remain present. Qualification of the affected resource set does not rewrite them and does not turn their outcome into STOPPED, FAILED or APPLIED.

## Independent Owner intent and journal

Migration requires one new explicit authenticated Owner-layer request with a new request ID and exact raw bytes. Within the existing authenticated USER managed-cli schema and the already listed `v37_managed_cli.rs` bridge, this proposal permits one strict migration command arm with exact fields binding that request and the approved original registration/HOME anchors to F's independent journal. The old four-field migrate frame is insufficient and must not acquire this effect implicitly. No new top-level transport, UI, shared path, writer or authority is permitted. The Controller may mechanically submit the already approved request; this adds no login or manual installation touchpoint. The request is independent of the original registration and of any later version action.

F/K-INSTANCE is the only persistent writer. Its durable migration journal minimally binds:

- the new migration request ID and raw fingerprint;
- the one native-selected `LEGACY_CODEX_M1` instance and driver;
- the original registration request/raw/receipt identities without changing them;
- the root/database/HOME identity tuple and pre-write SOURCE-absence fingerprint;
- the exact current official stage/version/program digest and full package-source-chain digest;
- the complete quiescence/native-fact digest and every row revision used for qualification;
- phase, exact result, receipt bytes, and the post-write SOURCE-row fingerprint/revision.

The journal uses a strict one-request state machine. The exact same request and bytes may only reconcile or read the original result. Changed bytes or target conflict. A different request while the original is unresolved, after an APPLIED migration, or against an existing SOURCE row refuses. A commit-unknown result is reconciled only from the same journal plus exact SOURCE row; it is never retried under a new ID.

## Only permitted write

The effect transaction may write only:

1. the independent legacy-source migration journal and its receipt; and
2. one previously absent row in `gogoke_v37_instance_program_sources` for `LEGACY_CODEX_M1`.

The SOURCE row retains the existing shape: `source=MANAGED`, exact current official `stage_name`, same-pin `program_digest` and `version`, exact original `home_identity`, the original APPLIED `registration_request_id`, and initial revision. The independent migration journal is the authoritative proof that this row was created later by a new Owner migration request. Readers and evidence must not state or infer that the historical registration originally created a MANAGED source.

The SOURCE insert and APPLIED migration receipt are atomic under the original verified authority and revision checks. The insert is absent-to-present only: no update, replacement or delete of an existing SOURCE row is allowed. The transaction does not modify the instance, profile, enabled/tombstone/login state, managed-copy state, version, program digest, HOME, credential, alias, H/C custody, claim, original operation, original receipt or old UNKNOWN.

## Existing code is not acceptance

The controlled construction source currently contains a broad `migrate_quiescent_legacy_instances` path in `apps/desktop/native-host/src/store/instance/program_source.rs` and a caller in `apps/desktop/native-host/src/store/product_database/v37_managed_cli.rs`. Its current shape is evidence for planning only. It handles a candidate list and directly inserts SOURCE rows but does not implement the independent Owner request/journal and one-disabled-candidate contract above. It must not be invoked for `LEGACY_CODEX_M1` merely because this proposal is merged. Construction may add only the strict request arm described above and narrow the F effect to this contract; a new top-level command/transport, writer, unlisted shared path or authority requires return to the Controller.

## Verification and stop points

Planning validation is limited to JSON parsing, LF and manifest hashes, the existing `verify_plan.py` structural/self tests, recomputed scope digest, fixed receipt-blob equality and exact diff scope. It does not run native code, a CLI, a model or the product.

Later construction must use cloud-native tests and actual installed-candidate evidence. Positive coverage requires the exact approved single disabled candidate, a positively proved `PROVEN_INELIGIBLE` classification and complete original APPLIED legacy registration, same-pin official source chain, complete quiescence, one atomic SOURCE/journal write, exact replay readback, and unchanged old records. Negative coverage includes replay `ELIGIBLE` or `UNKNOWN`, missing original raw/receipt, absent historical bind journal, trial-replay errors, changed approved anchors, no/multiple candidates, enabled/tombstoned/wrong-driver targets, existing SOURCE, changed raw or receipt, HOME/FileID/marker/SID mismatch, stage/package/pin mismatch, live/reused/unqueryable process identities, incomplete descendant set, timeout/grace substitutes, row/CAS drift, concurrent managed version action, changed replay bytes, new-ID retry and commit-unknown reconciliation.

Stop and return to the Controller before construction if any of these is true: the original replay-vs-legacy classification cannot be proved; the public contract cannot enforce the one-candidate scope without private data; a second writer or unlisted shared file is required; a schema/API/security change exceeds the listed F/H authority; proof requires a new evidence family; or a later version action would need any existing SOURCE/CAS, COLD/Busy, credential or UNKNOWN guard weakened.

## Non-claims and handoff

Merging this proposal authorizes only later implementation of this exact contract. It does not qualify `LEGACY_CODEX_M1`, execute a migration, close a HOME, clear Busy, prepare a replacement, start a version action, pass F.2, accept a candidate or publish a release. Review, merge and exact readback are gates; all real SOURCE and version actions remain refused until the later implementation and every direct evidence check pass.
