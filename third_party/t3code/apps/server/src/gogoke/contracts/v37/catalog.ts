/** Design 37 L0 operation names and durable owners. This list is closed. */
export const V37_OPERATIONS = {
  "K-SESSION": ["open", "resume", "stop", "send", "output-stream", "capability-probe", "model-list-read", "append-without-turn", "exit-and-stop-receipt", "reconnect", "compact", "renew-session", "admission-reserve", "admission-commit", "admission-release"],
  "K-LEDGER": ["record", "scoped-query", "subscribe", "resume-subscription", "end-subscription"],
  "K-INBOX": ["enqueue", "edit", "cancel", "steer", "deliver", "check-unknown", "requeue"],
  "K-QCARD": ["raise", "answer", "expire", "recover"],
  "K-SIDE": ["create", "resume", "archive", "restore", "delete", "pending-delta", "read-thread"],
  "K-SEAT": ["create-from-template", "tune", "bind-instance", "change-instance", "reclaim", "short-to-long", "state-card", "takeover-answers"],
  "K-POLICY": ["call-permission-table", "gate-submit", "gate-decide", "stage-transition", "escalate", "trigger-register", "trigger-recover", "trigger-cancel"],
  "K-INSTANCE": ["register", "install-state", "login-state", "version-and-new-version", "repin-after-manual-upgrade", "concurrency-input", "home-lifecycle"],
  "K-WORKTREE": ["create", "register", "classify-single-or-mixed", "graph-query", "merge", "cleanup"],
  "K-UI": ["read-models", "actions"],
} as const;

export type V37Family = keyof typeof V37_OPERATIONS;
export type V37Operation = (typeof V37_OPERATIONS)[V37Family][number];

export const V37_DURABLE_OWNER = Object.freeze({
  "K-SESSION": "H", "K-LEDGER": "A", "K-INBOX": "C", "K-QCARD": "C",
  "K-SIDE": "D", "K-SEAT": "E", "K-POLICY": "E", "K-INSTANCE": "F",
  "K-WORKTREE": "F", "K-UI": "INTEGRATOR",
} as const);

export const V37_READ_OPERATIONS: Readonly<Record<V37Family, readonly string[]>> = Object.freeze({
  "K-SESSION": Object.freeze(["output-stream", "capability-probe", "model-list-read"]),
  "K-LEDGER": Object.freeze(["scoped-query"]),
  "K-INBOX": Object.freeze(["check-unknown"]),
  "K-QCARD": Object.freeze([]),
  "K-SIDE": Object.freeze(["pending-delta", "read-thread"]),
  "K-SEAT": Object.freeze(["state-card"]),
  "K-POLICY": Object.freeze(["call-permission-table"]),
  "K-INSTANCE": Object.freeze(["install-state", "login-state", "version-and-new-version", "concurrency-input"]),
  "K-WORKTREE": Object.freeze(["classify-single-or-mixed", "graph-query"]),
  "K-UI": Object.freeze(["read-models"]),
});

export function isV37Operation(family: string, operation: string): family is V37Family {
  return Object.hasOwn(V37_OPERATIONS, family) &&
    (V37_OPERATIONS[family as V37Family] as readonly string[]).includes(operation);
}
