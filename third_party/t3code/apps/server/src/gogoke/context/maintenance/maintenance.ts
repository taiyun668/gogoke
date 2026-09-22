import type { ContextState } from "../../contracts/model.ts";
import type { AssemblyIdentity, ContextVersionRef } from "../assembly/model.ts";
import { captureMethods, canonical, count, digest, fail, hashData, identifier,
  ownArray, ownRecord, u64 } from "../assembly/passive.ts";

export interface MaintenanceRequest extends AssemblyIdentity {
  readonly operationId: string;
  /** Opaque native invalidation event/operation reference; not a caller-supplied graph. */
  readonly triggerId: string;
  readonly maxItems: number;
}
export interface MaintenanceReplayRequest extends AssemblyIdentity {
  readonly operationId: string;
}
export type InvalidationReason = "SOURCE_STATE_CHANGED" | "ACCESS_POLICY_CHANGED" | "READ_PERMISSION_REVOKED";
export interface ReadProjectionInvalidation extends ContextVersionRef {
  readonly currentState: ContextState;
  readonly stateRevision: string;
  readonly accessPolicyRevision: string;
  readonly projectionRevision: string;
  readonly reason: InvalidationReason;
}
export interface MaintenanceBatch extends AssemblyIdentity {
  readonly triggerId: string;
  readonly readRef: string;
  readonly batchId: string;
  readonly batchRevision: string;
  readonly graphRevision: string;
  readonly policyRevision: string;
  readonly authRevision: string;
  readonly revocationHead: string;
  readonly maxItems: number;
  readonly complete: true;
  readonly cause: {
    readonly sourceDomainId: string;
    readonly sourceOperationId: string;
    readonly fingerprint: string;
  };
  readonly invalidations: ReadonlyArray<ReadProjectionInvalidation>;
}
export interface MaintenanceCommand {
  readonly operation: "InvalidateContextReadProjections";
  readonly operationId: string;
  readonly semanticDigest: string;
  readonly basis: MaintenanceBatch;
  readonly invalidations: ReadonlyArray<ReadProjectionInvalidation>;
}
export interface MaintenanceReceipt {
  readonly operationId: string;
  readonly receiptId: string;
  readonly semanticDigest: string;
  readonly invalidatedViews: number;
}
export type MaintenanceCommitResult =
  | ({ readonly kind: "committed" | "replayed" } & MaintenanceReceipt)
  | { readonly kind: "denied" | "stale" | "conflict" };

/**
 * Internal adapter to the SAME Product Authority, not a new status/graph store.
 * Native commit_context_version already owns descendant invalidation. This port
 * resolves that committed cause into a complete, current, per-target-domain batch.
 * It rechecks principal/seat/binding/policy/revocation on every read and commit.
 *
 * commitReadProjectionInvalidation must compare the batch/graph and every state,
 * access and projection revision transactionally, evict derived read projections
 * and mark affected manifests for recheck. It MUST NOT modify/delete immutable
 * Context contents, rewrite source lifecycle state or replay a native action.
 * readCurrentMaintenanceReceipt reauthorizes before disclosing even a prior result.
 * A fake port proves protocol choreography only; native wiring remains required.
 */
export interface ContextMaintenanceAuthorityPort {
  openMaintenance(request: MaintenanceRequest): Promise<MaintenanceBatch | null>;
  commitReadProjectionInvalidation(command: MaintenanceCommand): Promise<MaintenanceCommitResult>;
  readCurrentMaintenanceReceipt(request: MaintenanceReplayRequest): Promise<MaintenanceReceipt | null>;
}

const IDENTITY = Object.freeze(["principalId", "seatId", "taskId", "sessionId", "domainId",
  "bindingId", "bindingGeneration", "sourceEpoch", "runtimeInstanceId"] as const);
const RECEIPT = Object.freeze(["operationId", "receiptId", "semanticDigest", "invalidatedViews"]);
const STATES: readonly ContextState[] = Object.freeze(["ACTIVE", "SUPERSEDED", "CONFLICTED", "STALE", "REVOKED", "ARCHIVED"]);

function snapshotIdentity(raw: Readonly<Record<string, unknown>>, input = false): AssemblyIdentity {
  const code = input ? "INVALID_INPUT" : "AUTHORITY_PROTOCOL_ERROR";
  return Object.freeze({ principalId: identifier(raw.principalId, code), seatId: identifier(raw.seatId, code),
    taskId: identifier(raw.taskId, code), sessionId: identifier(raw.sessionId, code), domainId: identifier(raw.domainId, code),
    bindingId: identifier(raw.bindingId, code), bindingGeneration: u64(raw.bindingGeneration, code),
    sourceEpoch: u64(raw.sourceEpoch, code), runtimeInstanceId: identifier(raw.runtimeInstanceId, code) });
}
function requestSnapshot(input: unknown): MaintenanceRequest {
  const raw = ownRecord(input, [...IDENTITY, "operationId", "triggerId", "maxItems"], [], "INVALID_INPUT");
  return Object.freeze({ ...snapshotIdentity(raw, true), operationId: identifier(raw.operationId, "INVALID_INPUT"),
    triggerId: identifier(raw.triggerId, "INVALID_INPUT"), maxItems: count(raw.maxItems, "INVALID_INPUT") });
}
function batchSnapshot(input: unknown, request: MaintenanceRequest): MaintenanceBatch {
  const raw = ownRecord(input, [...IDENTITY, "triggerId", "readRef", "batchId", "batchRevision", "graphRevision",
    "policyRevision", "authRevision", "revocationHead", "maxItems", "complete", "cause", "invalidations"]);
  const identity = snapshotIdentity(raw);
  if (IDENTITY.some(key => identity[key] !== request[key]) || raw.triggerId !== request.triggerId) return fail("ACCESS_DENIED");
  if (raw.complete !== true) return fail("NEEDS_EVIDENCE");
  const cause = ownRecord(raw.cause, ["sourceDomainId", "sourceOperationId", "fingerprint"]);
  const invalidations = ownArray(raw.invalidations).map(value => {
    const row = ownRecord(value, ["sourceDomainId", "contextId", "version", "currentState", "stateRevision",
      "accessPolicyRevision", "projectionRevision", "reason"]);
    if (typeof row.currentState !== "string" || !STATES.includes(row.currentState as ContextState)) return fail("AUTHORITY_PROTOCOL_ERROR");
    if (row.reason !== "SOURCE_STATE_CHANGED" && row.reason !== "ACCESS_POLICY_CHANGED" && row.reason !== "READ_PERMISSION_REVOKED") {
      return fail("AUTHORITY_PROTOCOL_ERROR");
    }
    // Revoking a reader can invalidate a view while the underlying Context remains ACTIVE.
    if (row.reason === "SOURCE_STATE_CHANGED" && row.currentState === "ACTIVE") return fail("AUTHORITY_PROTOCOL_ERROR");
    const sourceDomainId = identifier(row.sourceDomainId);
    if (sourceDomainId !== identity.domainId) return fail("ACCESS_DENIED");
    return Object.freeze({ sourceDomainId, contextId: identifier(row.contextId), version: u64(row.version),
      currentState: row.currentState as ContextState, stateRevision: u64(row.stateRevision),
      accessPolicyRevision: u64(row.accessPolicyRevision), projectionRevision: u64(row.projectionRevision), reason: row.reason });
  });
  const key = (row: ReadProjectionInvalidation): string => canonical([row.sourceDomainId, row.contextId, row.version]);
  if (new Set(invalidations.map(key)).size !== invalidations.length) return fail("AUTHORITY_PROTOCOL_ERROR");
  invalidations.sort((left, right) => key(left) < key(right) ? -1 : key(left) > key(right) ? 1 : 0);
  const maxItems = count(raw.maxItems);
  if (invalidations.length > Math.min(request.maxItems, maxItems)) return fail("NEEDS_BUDGET");
  return Object.freeze({ ...identity, triggerId: identifier(raw.triggerId), readRef: identifier(raw.readRef),
    batchId: identifier(raw.batchId), batchRevision: u64(raw.batchRevision), graphRevision: u64(raw.graphRevision),
    policyRevision: u64(raw.policyRevision), authRevision: u64(raw.authRevision), revocationHead: u64(raw.revocationHead),
    maxItems, complete: true, cause: Object.freeze({ sourceDomainId: identifier(cause.sourceDomainId),
      sourceOperationId: identifier(cause.sourceOperationId), fingerprint: digest(cause.fingerprint) }),
    invalidations: Object.freeze(invalidations) });
}
function receiptSnapshot(input: unknown, operationId: string): MaintenanceReceipt {
  const raw = ownRecord(input, RECEIPT);
  if (raw.operationId !== operationId) return fail("AUTHORITY_PROTOCOL_ERROR");
  return Object.freeze({ operationId, receiptId: identifier(raw.receiptId), semanticDigest: digest(raw.semanticDigest),
    invalidatedViews: count(raw.invalidatedViews) });
}

/** No local cache or fallback store; shared ContextAssemblyError codes are used. */
export class ContextViewMaintenance {
  readonly #authority: ContextMaintenanceAuthorityPort;
  constructor(authority: ContextMaintenanceAuthorityPort) {
    this.#authority = captureMethods(authority, ["openMaintenance", "commitReadProjectionInvalidation", "readCurrentMaintenanceReceipt"]);
  }
  async #read<T>(operation: () => Promise<T>): Promise<T> {
    try { return await operation(); } catch { return fail("AUTHORITY_UNAVAILABLE"); }
  }
  async replay(input: MaintenanceReplayRequest): Promise<MaintenanceReceipt> {
    const raw = ownRecord(input, [...IDENTITY, "operationId"], [], "INVALID_INPUT");
    const request = Object.freeze({ ...snapshotIdentity(raw, true), operationId: identifier(raw.operationId, "INVALID_INPUT") });
    const result = await this.#read(() => this.#authority.readCurrentMaintenanceReceipt(request));
    if (result === null) return fail("ACCESS_DENIED");
    return receiptSnapshot(result, request.operationId);
  }
  async maintain(input: MaintenanceRequest): Promise<MaintenanceReceipt> {
    const request = requestSnapshot(input);
    const opened = await this.#read(() => this.#authority.openMaintenance(request));
    if (opened === null) return fail("ACCESS_DENIED");
    const basis = batchSnapshot(opened, request);
    const { readRef: _readRef, ...stableBasis } = basis;
    const semanticDigest = hashData({ schema: "gogoke.context-maintenance.v1", request, basis: stableBasis });
    let result: Readonly<Record<string, unknown>>;
    let receipt: MaintenanceReceipt | null = null;
    try {
      const value = await this.#authority.commitReadProjectionInvalidation(Object.freeze({
        operation: "InvalidateContextReadProjections", operationId: request.operationId, semanticDigest,
        basis, invalidations: basis.invalidations,
      }));
      result = ownRecord(value, ["kind"], RECEIPT);
      if (result.kind === "committed" || result.kind === "replayed") {
        ownRecord(result, ["kind", ...RECEIPT]);
        receipt = receiptSnapshot({ operationId: result.operationId, receiptId: result.receiptId,
          semanticDigest: result.semanticDigest, invalidatedViews: result.invalidatedViews }, request.operationId);
        if (receipt.semanticDigest !== semanticDigest || receipt.invalidatedViews !== basis.invalidations.length) {
          return fail("COMMIT_OUTCOME_UNKNOWN");
        }
      } else if (result.kind === "denied" || result.kind === "stale" || result.kind === "conflict") {
        ownRecord(result, ["kind"]);
      } else return fail("COMMIT_OUTCOME_UNKNOWN");
    } catch { return fail("COMMIT_OUTCOME_UNKNOWN"); }
    if (result.kind === "denied") return fail("ACCESS_DENIED");
    if (result.kind === "stale") return fail("STALE_ASSEMBLY");
    if (result.kind === "conflict") return fail("OPERATION_CONFLICT");
    if (receipt === null) return fail("COMMIT_OUTCOME_UNKNOWN");
    const replayIdentity: MaintenanceReplayRequest = Object.freeze({
      principalId: request.principalId, seatId: request.seatId, taskId: request.taskId, sessionId: request.sessionId,
      domainId: request.domainId, bindingId: request.bindingId, bindingGeneration: request.bindingGeneration,
      sourceEpoch: request.sourceEpoch, runtimeInstanceId: request.runtimeInstanceId, operationId: request.operationId,
    });
    const current = await this.replay(replayIdentity);
    if (canonical(current) !== canonical(receipt)) return fail("COMMIT_OUTCOME_UNKNOWN");
    return current;
  }
}
