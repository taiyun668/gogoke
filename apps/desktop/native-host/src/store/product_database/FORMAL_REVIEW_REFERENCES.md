# Formal review native entry

The Owner User ingress may add `purpose: "FORMAL_REVIEW"` to the existing
K-SESSION `open` payload: `seatId`, `generation`, `repositoryId`, `worktreeId`.
The existing admission, fixed CLI, LPAC launch and original A/H receipts still
apply. This starts a new native thread and persists the ledger purpose.
No previous session, fork source, transcript or caller-supplied authority is
accepted. A model-created child still opens an ordinary Work session.

The persisted purpose refuses resume, reconnect, compact and renewal before
continuation effects. A repeated original open may only return its original
H outcome. The existing ledger rules refuse review reads of inherited context.
Normal stop/release remain available. Per-provider real input and memory
evidence, and same-seat side-chat markers, are still required by V10; source
or cloud controls do not settle that check.

## References and differences

Read Design 37 section 6 and PLAN.json V10; the existing ledger registration
and FormalReview read rules; and the native SideChat registration entry.
Historical gogo-party seat-runtime and its research coverage distinguish fresh
review from a reused seat role, but the coverage record does not establish
enforced fresh sessions. This entry therefore reuses the native ledger and
SideChat purpose mechanism rather than adopting role-only independence.
No operation, permission tier or credential boundary is added.
