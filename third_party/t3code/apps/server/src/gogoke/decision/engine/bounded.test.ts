import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import { BoundedDecisionExecutor, type DecisionTiming } from "./bounded.ts";
import {
  DecisionEngineError,
  type DecisionBackend,
  type DecisionBackendResult,
  type DecisionCommitPort,
  type DecisionEligibilityPort,
  type DecisionRequest,
  type EligibilityCandidate,
  type EligibilitySnapshot,
} from "./engine.ts";

const request: DecisionRequest = {
  operationId: "operation-one",
  scenarioId: "DF02",
  stateViewHash: "view-one",
  candidateHash: "candidates-one",
  questionVersion: "question-one",
  rubricVersion: "rubric-one",
  modelRequested: null,
  taskRevision: "1",
  policyRevision: "2",
  capabilityRevision: "3",
  bindingGeneration: "4",
  budgetUnits: 5,
  deadlineEpochMs: 10_000,
  candidateRefs: ["candidate-one"],
};

const candidate: EligibilityCandidate = {
  candidateId: "candidate-one",
  authorization: "ALLOWED",
  capability: "QUALIFIED",
  isolation: "QUALIFIED",
  capacity: { required: 1, available: 2 },
  priorityClass: 1,
  waitingMs: 0,
  estimatedCost: null,
  recipeRef: "recipe-one",
  resourceReservationRef: "reservation-one",
  actionIntentRef: `opr_${"1".repeat(32)}`,
};

const snapshot: EligibilitySnapshot = {
  taskRevision: request.taskRevision,
  policyRevision: request.policyRevision,
  capabilityRevision: request.capabilityRevision,
  bindingGeneration: request.bindingGeneration,
  candidates: [candidate],
};

const backendResult: DecisionBackendResult = {
  kind: "RANKED",
  modelResolved: "fixture-v1",
  ranks: [{ candidateId: candidate.candidateId, semanticRank: 1 }],
};

function errorCode(code: DecisionEngineError["code"]) {
  return (error: unknown): error is DecisionEngineError =>
    error instanceof DecisionEngineError && error.code === code;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

async function drainMicrotasks(): Promise<void> {
  for (let turn = 0; turn < 12; turn++) await Promise.resolve();
}

class ManualTiming implements DecisionTiming {
  epoch = 1_000;
  monotonic = 100;
  monotonicReads = 0;
  closeOnRead: number | null = null;
  cancelledTimers = 0;
  #deadline: (() => void) | undefined;

  epochMs(): number {
    return this.epoch;
  }

  monotonicMs(): number {
    this.monotonicReads++;
    return this.monotonicReads === this.closeOnRead ? 2_100 : this.monotonic;
  }

  schedule(_delayMs: number, callback: () => void): () => void {
    this.#deadline = callback;
    return () => {
      this.cancelledTimers++;
    };
  }

  expire(): void {
    this.#deadline?.();
  }
}

function eligibilityPort(
  resolve: () => Promise<EligibilitySnapshot | null> | EligibilitySnapshot | null,
): DecisionEligibilityPort {
  return {
    async resolveEligibility() {
      return resolve();
    },
  };
}

function backendPort(
  evaluate: (
    input: Parameters<DecisionBackend["evaluate"]>[0],
  ) => Promise<DecisionBackendResult> | DecisionBackendResult,
): DecisionBackend {
  return {
    kind: "FAKE",
    async evaluate(input) {
      return evaluate(input);
    },
  };
}

function commitPort(
  commit: (
    command: Parameters<DecisionCommitPort["commitDecisionReservation"]>[0],
  ) => ReturnType<DecisionCommitPort["commitDecisionReservation"]>,
): DecisionCommitPort {
  return {
    async commitDecisionReservation(command) {
      return commit(command);
    },
  };
}

function committed(command: Parameters<DecisionCommitPort["commitDecisionReservation"]>[0]) {
  return {
    kind: "committed" as const,
    operationId: command.record.operationId,
    decisionReceiptId: "receipt-one",
  };
}

describe("BoundedDecisionExecutor", () => {
  it("does not enter eligibility when the first await gate sees a closed deadline", async () => {
    const timing = new ManualTiming();
    timing.closeOnRead = 2;
    let eligibilityCalls = 0;
    let backendCalls = 0;
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => {
        eligibilityCalls++;
        return snapshot;
      }),
      backendPort(() => {
        backendCalls++;
        return backendResult;
      }),
      commitPort((command) => {
        commitCalls++;
        return Promise.resolve(committed(command));
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    Assert.deepEqual(await run.result, {
      kind: "DEADLINE",
      operationId: request.operationId,
      commitStarted: false,
    });
    Assert.equal(eligibilityCalls, 0);
    Assert.equal(backendCalls, 0);
    Assert.equal(commitCalls, 0);
  });

  it("does not enter backend evaluation when its pre-await deadline gate closes", async () => {
    const timing = new ManualTiming();
    timing.closeOnRead = 5;
    let eligibilityCalls = 0;
    let backendCalls = 0;
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => {
        eligibilityCalls++;
        return snapshot;
      }),
      backendPort(() => {
        backendCalls++;
        return backendResult;
      }),
      commitPort((command) => {
        commitCalls++;
        return Promise.resolve(committed(command));
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    Assert.deepEqual(await run.result, {
      kind: "DEADLINE",
      operationId: request.operationId,
      commitStarted: false,
    });
    Assert.equal(eligibilityCalls, 1);
    Assert.equal(backendCalls, 0);
    Assert.equal(commitCalls, 0);
  });

  it("does not enter native commit when its pre-await deadline gate closes", async () => {
    const timing = new ManualTiming();
    timing.closeOnRead = 7;
    let eligibilityCalls = 0;
    let backendCalls = 0;
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => {
        eligibilityCalls++;
        return snapshot;
      }),
      backendPort(() => {
        backendCalls++;
        return backendResult;
      }),
      commitPort((command) => {
        commitCalls++;
        return Promise.resolve(committed(command));
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    Assert.deepEqual(await run.result, {
      kind: "DEADLINE",
      operationId: request.operationId,
      commitStarted: false,
    });
    Assert.equal(eligibilityCalls, 1);
    Assert.equal(backendCalls, 1);
    Assert.equal(commitCalls, 0);
  });

  it("stops after a pending eligibility await when cancelled, even if eligibility later resolves", async () => {
    const timing = new ManualTiming();
    const pending = deferred<EligibilitySnapshot | null>();
    const entered = deferred<void>();
    let backendCalls = 0;
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => {
        entered.resolve();
        return pending.promise;
      }),
      backendPort(() => {
        backendCalls++;
        return backendResult;
      }),
      commitPort((command) => {
        commitCalls++;
        return Promise.resolve(committed(command));
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    await entered.promise;
    run.cancel();
    Assert.deepEqual(await run.result, {
      kind: "CANCELLED",
      operationId: request.operationId,
      commitStarted: false,
    });
    pending.resolve(snapshot);
    await drainMicrotasks();
    Assert.equal(backendCalls, 0);
    Assert.equal(commitCalls, 0);
  });

  it("stops after the eligibility await when its deadline has elapsed", async () => {
    const timing = new ManualTiming();
    const pending = deferred<EligibilitySnapshot | null>();
    const entered = deferred<void>();
    let backendCalls = 0;
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => {
        entered.resolve();
        return pending.promise;
      }),
      backendPort(() => {
        backendCalls++;
        return backendResult;
      }),
      commitPort((command) => {
        commitCalls++;
        return Promise.resolve(committed(command));
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    await entered.promise;
    timing.expire();
    pending.resolve(snapshot);
    Assert.deepEqual(await run.result, {
      kind: "DEADLINE",
      operationId: request.operationId,
      commitStarted: false,
    });
    await drainMicrotasks();
    Assert.equal(backendCalls, 0);
    Assert.equal(commitCalls, 0);
  });

  it("does not enter commit after a pending backend await is cancelled", async () => {
    const timing = new ManualTiming();
    const pending = deferred<DecisionBackendResult>();
    const entered = deferred<void>();
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => snapshot),
      backendPort(() => {
        entered.resolve();
        return pending.promise;
      }),
      commitPort((command) => {
        commitCalls++;
        return Promise.resolve(committed(command));
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    await entered.promise;
    run.cancel();
    Assert.deepEqual(await run.result, {
      kind: "CANCELLED",
      operationId: request.operationId,
      commitStarted: false,
    });
    pending.resolve(backendResult);
    await drainMicrotasks();
    Assert.equal(commitCalls, 0);
  });

  it("does not enter commit after a pending backend await reaches its deadline", async () => {
    const timing = new ManualTiming();
    const pending = deferred<DecisionBackendResult>();
    const entered = deferred<void>();
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => snapshot),
      backendPort(() => {
        entered.resolve();
        return pending.promise;
      }),
      commitPort((command) => {
        commitCalls++;
        return Promise.resolve(committed(command));
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    await entered.promise;
    timing.expire();
    pending.resolve(backendResult);
    Assert.deepEqual(await run.result, {
      kind: "DEADLINE",
      operationId: request.operationId,
      commitStarted: false,
    });
    await drainMicrotasks();
    Assert.equal(commitCalls, 0);
  });

  it("reports COMMIT_UNKNOWN after cancellation while commit is pending, including late success", async () => {
    const timing = new ManualTiming();
    const pending =
      deferred<Awaited<ReturnType<DecisionCommitPort["commitDecisionReservation"]>>>();
    const entered = deferred<void>();
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => snapshot),
      backendPort(() => backendResult),
      commitPort((command) => {
        commitCalls++;
        entered.resolve();
        return pending.promise;
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    await entered.promise;
    run.cancel();
    await Assert.rejects(run.result, errorCode("COMMIT_UNKNOWN"));
    pending.resolve({
      kind: "committed",
      operationId: request.operationId,
      decisionReceiptId: "late-receipt",
    });
    await drainMicrotasks();
    Assert.equal(commitCalls, 1);
  });

  it("reports COMMIT_UNKNOWN after cancellation while commit is pending, including late rejection", async () => {
    const timing = new ManualTiming();
    const pending =
      deferred<Awaited<ReturnType<DecisionCommitPort["commitDecisionReservation"]>>>();
    const entered = deferred<void>();
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => snapshot),
      backendPort(() => backendResult),
      commitPort(() => {
        commitCalls++;
        entered.resolve();
        return pending.promise;
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    await entered.promise;
    run.cancel();
    await Assert.rejects(run.result, errorCode("COMMIT_UNKNOWN"));
    pending.reject(new Error("late transport rejection"));
    await drainMicrotasks();
    Assert.equal(commitCalls, 1);
  });

  it("reports COMMIT_UNKNOWN when the deadline closes during commit", async () => {
    const timing = new ManualTiming();
    const pending =
      deferred<Awaited<ReturnType<DecisionCommitPort["commitDecisionReservation"]>>>();
    const entered = deferred<void>();
    let commitCalls = 0;
    const executor = new BoundedDecisionExecutor(
      eligibilityPort(() => snapshot),
      backendPort(() => backendResult),
      commitPort((command) => {
        commitCalls++;
        entered.resolve();
        return pending.promise;
      }),
      "INTERACTIVE",
      timing,
    );

    const run = executor.start(request);
    await entered.promise;
    timing.expire();
    await Assert.rejects(run.result, errorCode("COMMIT_UNKNOWN"));
    pending.resolve({
      kind: "committed",
      operationId: request.operationId,
      decisionReceiptId: "late-receipt",
    });
    await drainMicrotasks();
    Assert.equal(commitCalls, 1);
  });
});
