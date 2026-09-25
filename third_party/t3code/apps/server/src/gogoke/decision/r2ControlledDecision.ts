import { createHash } from "node:crypto";

import type { NativeStoreSession } from "../bootstrap/nativeStoreService.ts";
import type {
  NativeR2ActionDecisionBasis,
  NativeR2TestDelegationReceipt,
} from "../persistence/base/nativeHostClient.ts";
import type { DecisionCommitResult, DecisionRecord } from "./engine/engine.ts";
import { createNativeDecisionCommitPort } from "./nativeAuthority.ts";

const OPERATION = "decision-r2-02-test";
const ACTION = "opr_22222222222222222222222222222222";
const CANDIDATE = "candidate-r2-02-fixture";
const candidateHash = `sha256:${createHash("sha256")
  .update("gogoke.s1-r4.r2-02.fixed-candidate:deterministic-fixture")
  .digest("hex")}`;

/** Fixed, public test Decision through the existing native capacity/commit port. */
export async function commitR2ControlledDecision(input: {
  readonly store: NativeStoreSession;
  readonly basis: NativeR2ActionDecisionBasis;
  readonly grant: NativeR2TestDelegationReceipt;
  readonly recordedAt: string;
  readonly slot?: "novel";
}): Promise<DecisionCommitResult> {
  const { store, basis, grant, recordedAt } = input;
  const novel = input.slot === "novel";
  const operation = novel ? "decision-r2-03-test" : OPERATION;
  const action = novel ? "opr_33333333333333333333333333333333" : ACTION;
  const candidate = novel ? "candidate-r2-03-fixture" : CANDIDATE;
  const selectedHash = novel ? `sha256:${createHash("sha256")
    .update("gogoke.s1-r4.r2-03.novel-candidate:deterministic-fixture")
    .digest("hex")}` : candidateHash;
  const deadlineEpochMs = Number(grant.expiresAtEpochMs);
  if (basis.state !== "TEST_ONLY_DECISION_BASIS_NOT_ACTION" ||
      basis.bindingId !== (novel ? "binding-r2-03-worker" : "binding-r2-02-worker") || basis.bindingGeneration !== "1" ||
      grant.state !== "TEST_ONLY_GRANT_PREPARED_NOT_ACTION" ||
      !Number.isSafeInteger(deadlineEpochMs) || deadlineEpochMs <= Date.now() ||
      !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/u.test(recordedAt)) {
    throw new Error("INVALID_R2_TEST_DECISION_BASIS");
  }
  const record: DecisionRecord = Object.freeze({
    operationId: operation,
    scenarioId: "DF10",
    family: "CONTEXT_SELECTION",
    state: "COMMITTED",
    stateViewHash: basis.stateViewHash,
    candidateHash: selectedHash,
    questionVersion: "1",
    rubricVersion: "1",
    modelRequested: null,
    modelResolved: "deterministic-fixture",
    taskRevision: basis.taskRevision,
    policyRevision: basis.policyRevision,
    capabilityRevision: "1",
    bindingGeneration: basis.bindingGeneration,
    backendKind: "FAKE",
    choice: candidate,
    reason: "QUALIFIED_BOUNDED_SELECTION",
    budgetUnits: 1,
    deadlineEpochMs,
  });
  const port = createNativeDecisionCommitPort(store, {
    operationId: operation,
    domainId: "domain-r2-02-test",
    decisionId: operation,
    eventId: novel ? "r2-03-decision-event" : "r2-02-decision-event",
    receiptId: novel ? "r2-03-decision-receipt" : "r2-02-decision-receipt",
    recordedAt,
    candidates: [{
      candidateId: candidate,
      requiredCapacityUnits: 1,
      snapshot: {
        operationId: operation,
        candidateId: candidate,
        stateViewHash: basis.stateViewHash,
        candidateHash: selectedHash,
        taskRevision: basis.taskRevision,
        policyRevision: basis.policyRevision,
        capabilityRevision: "1",
        bindingId: basis.bindingId,
        bindingGeneration: basis.bindingGeneration,
        authRevision: basis.policyRevision,
        resourceRef: "capacity-r2-02-fixture",
        resourceRevision: "1",
        capacityTotal: 1,
        actionOperationId: action,
        actionDigest: basis.actionDigest,
      },
    }],
  });
  return port.commitDecisionReservation({
    record,
    reservation: { candidateId: candidate, resourceReservationRef: novel ? "capacity-lease-r2-03" : "capacity-lease-r2-02" },
    action: { actionIntentRef: action },
  });
}
