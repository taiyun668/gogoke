import type { NativeStoreSession } from "../bootstrap/nativeStoreService.ts";
import type {
  NativeDecisionAuthoritySnapshot,
  NativeDecisionRecord,
} from "../persistence/base/nativeHostClient.ts";
import type {
  DecisionCommitCommand,
  DecisionCommitPort,
  DecisionCommitResult,
  DecisionRecord,
} from "./engine/engine.ts";

export interface NativeDecisionCommitCandidate {
  readonly candidateId: string;
  readonly requiredCapacityUnits: number;
  readonly snapshot: NativeDecisionAuthoritySnapshot;
}

export interface NativeDecisionCommitBasis {
  readonly operationId: string;
  readonly domainId: string;
  readonly decisionId: string;
  readonly eventId: string;
  readonly receiptId: string;
  readonly recordedAt: string;
  readonly candidates: ReadonlyArray<NativeDecisionCommitCandidate>;
}

const stale = (): DecisionCommitResult => Object.freeze({ kind: "stale" as const });
const conflict = (): DecisionCommitResult => Object.freeze({ kind: "conflict" as const });
const denied = (): DecisionCommitResult => Object.freeze({ kind: "denied" as const });

const safeCount = (value: number): boolean =>
  Number.isSafeInteger(value) && value >= 0;

function nativeRecord(record: DecisionRecord): NativeDecisionRecord | null {
  if (
    record.state !== "COMMITTED" ||
    record.choice === null ||
    record.reason !== "QUALIFIED_BOUNDED_SELECTION" ||
    !["RULES", "FAKE", "REPLAY"].includes(record.backendKind) ||
    !safeCount(record.budgetUnits) ||
    !safeCount(record.deadlineEpochMs)
  ) {
    return null;
  }
  return Object.freeze({
    operationId: record.operationId,
    scenarioId: record.scenarioId,
    family: record.family,
    state: "COMMITTED",
    stateViewHash: record.stateViewHash,
    candidateHash: record.candidateHash,
    questionVersion: record.questionVersion,
    rubricVersion: record.rubricVersion,
    modelRequested: record.modelRequested,
    modelResolved: record.modelResolved,
    taskRevision: record.taskRevision,
    policyRevision: record.policyRevision,
    capabilityRevision: record.capabilityRevision,
    bindingGeneration: record.bindingGeneration,
    backendKind: record.backendKind as NativeDecisionRecord["backendKind"],
    choice: record.choice,
    reason: "QUALIFIED_BOUNDED_SELECTION",
    budgetUnits: record.budgetUnits,
    deadlineEpochMs: record.deadlineEpochMs,
  });
}

function replayRecord(record: NativeDecisionRecord): DecisionRecord {
  return Object.freeze({ ...record });
}

/**
 * Operation-scoped bridge from the bounded Decision engine to the one native
 * Product Authority. Candidate snapshots are comparison inputs from the same
 * scheduler eligibility pass, not transferable authorization or a local cache.
 *
 * The native side persists/rechecks policy, binding, Action identity, pool
 * revision and capacity before atomically leasing capacity + writing Decision.
 */
export function createNativeDecisionCommitPort(
  store: NativeStoreSession,
  basis: NativeDecisionCommitBasis,
): DecisionCommitPort {
  const candidates = new Map<string, NativeDecisionCommitCandidate>();
  if (basis.operationId.length === 0 || basis.candidates.length === 0) {
    return Object.freeze({ commitDecisionReservation: async () => denied() });
  }
  for (const candidate of basis.candidates) {
    if (
      candidate.candidateId.length === 0 ||
      candidate.candidateId !== candidate.snapshot.candidateId ||
      candidate.snapshot.operationId !== basis.operationId ||
      !safeCount(candidate.requiredCapacityUnits) ||
      candidates.has(candidate.candidateId)
    ) {
      return Object.freeze({ commitDecisionReservation: async () => denied() });
    }
    candidates.set(candidate.candidateId, candidate);
  }

  return Object.freeze({
    async commitDecisionReservation(command: DecisionCommitCommand): Promise<DecisionCommitResult> {
      if (command.record.operationId !== basis.operationId || command.record.choice === null) return conflict();
      const candidate = candidates.get(command.record.choice);
      if (candidate === undefined || command.reservation.candidateId !== candidate.candidateId) return conflict();
      const snapshot = candidate.snapshot;
      if (
        snapshot.stateViewHash !== command.record.stateViewHash ||
        snapshot.candidateHash !== command.record.candidateHash ||
        snapshot.taskRevision !== command.record.taskRevision ||
        snapshot.policyRevision !== command.record.policyRevision ||
        snapshot.capabilityRevision !== command.record.capabilityRevision ||
        snapshot.bindingGeneration !== command.record.bindingGeneration
      ) {
        return stale();
      }
      if (snapshot.actionOperationId !== command.action.actionIntentRef) return conflict();
      const record = nativeRecord(command.record);
      if (record === null) return denied();

      // Publishing is itself an authenticated Product Authority operation. Any
      // uncertain native transport result is allowed to throw so DecisionEngine
      // converts it to COMMIT_UNKNOWN rather than guessing/retrying.
      await store.publishDecisionSnapshot(snapshot);
      const outcome = await store.commitDecision({
        domainId: basis.domainId,
        decisionId: basis.decisionId,
        eventId: basis.eventId,
        receiptId: basis.receiptId,
        recordedAt: basis.recordedAt,
        record,
        resourceReservationRef: command.reservation.resourceReservationRef,
        actionIntentRef: command.action.actionIntentRef,
        requiredCapacityUnits: candidate.requiredCapacityUnits,
      });
      if (outcome.operationId !== basis.operationId) throw new Error("native Decision operation identity mismatch");
      return outcome.kind === "committed"
        ? Object.freeze({ kind: "committed" as const, operationId: outcome.operationId,
            decisionReceiptId: outcome.decisionReceiptId })
        : Object.freeze({ kind: "replayed" as const, operationId: outcome.operationId,
            decisionReceiptId: outcome.decisionReceiptId, record: replayRecord(outcome.record) });
    },
  });
}
