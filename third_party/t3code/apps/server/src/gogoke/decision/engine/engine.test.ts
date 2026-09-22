import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import {
  CLOSED_GENERATIVE_BACKEND,
  CLOSED_JEV_BACKEND,
  DecisionBackendClosedError,
} from "../backends/closed.ts";
import { DECISION_SCENARIOS } from "../families/registry.ts";
import {
  DecisionEngine,
  DecisionEngineError,
  type DecisionBackend,
  type DecisionBackendResult,
  type DecisionCommitPort,
  type DecisionEligibilityPort,
  type DecisionRequest,
  type EligibilityCandidate,
  type EligibilitySnapshot,
} from "./engine.ts";

const request = (scenarioId = "DF02", candidateIds = ["candidate-one"]): DecisionRequest => ({
  operationId: "operation-one",
  scenarioId,
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
  candidateRefs: candidateIds,
});

const candidate = (
  candidateId: string,
  changes: Partial<EligibilityCandidate> = {},
): EligibilityCandidate => ({
  candidateId,
  authorization: "ALLOWED",
  capability: "QUALIFIED",
  isolation: "QUALIFIED",
  capacity: { required: 1, available: 2 },
  priorityClass: 1,
  waitingMs: 0,
  estimatedCost: null,
  recipeRef: `recipe-${candidateId}`,
  resourceReservationRef: `reservation-${candidateId}`,
  actionIntentRef: `opr_${"1".repeat(32)}`,
  ...changes,
});

const eligibility = (
  forRequest: DecisionRequest,
  candidates: ReadonlyArray<EligibilityCandidate>,
): EligibilitySnapshot => ({
  taskRevision: forRequest.taskRevision,
  policyRevision: forRequest.policyRevision,
  capabilityRevision: forRequest.capabilityRevision,
  bindingGeneration: forRequest.bindingGeneration,
  candidates,
});

const ranked = (candidateIds: ReadonlyArray<string>): DecisionBackendResult => ({
  kind: "RANKED",
  modelResolved: "fixture-v1",
  ranks: candidateIds.map((candidateId, index) => ({ candidateId, semanticRank: index })),
});

function errorCode(code: DecisionEngineError["code"]) {
  return (error: unknown): error is DecisionEngineError =>
    error instanceof DecisionEngineError && error.code === code;
}

function commitPort(calls: string[] = []): DecisionCommitPort {
  return {
    async commitDecisionReservation(command) {
      calls.push(command.action.actionIntentRef);
      return {
        kind: "committed",
        operationId: command.record.operationId,
        decisionReceiptId: "receipt-one",
      };
    },
  };
}

function engineFor(
  forRequest: DecisionRequest,
  options: {
    candidates?: ReadonlyArray<EligibilityCandidate>;
    backend?: DecisionBackend;
    commits?: string[];
    eligibilityPort?: DecisionEligibilityPort;
  } = {},
): DecisionEngine {
  const candidates = options.candidates ?? forRequest.candidateRefs.map((id) => candidate(id));
  const eligibilityPort = options.eligibilityPort ?? {
    async resolveEligibility() {
      return eligibility(forRequest, candidates);
    },
  };
  const backend = options.backend ?? {
    kind: "FAKE" as const,
    async evaluate(input) {
      return ranked(input.candidates.map((row) => row.candidateId));
    },
  };
  return new DecisionEngine(eligibilityPort, backend, commitPort(options.commits));
}

describe("DecisionEngine", () => {
  it("decodes requests as passive exact data without invoking accessors", async () => {
    const touches: string[] = [];
    const badRequest = { ...request() };
    Object.defineProperty(badRequest, "scenarioId", {
      enumerable: true,
      get() {
        touches.push("scenarioId");
        throw new Error("request getter invoked");
      },
    });
    const extraField = { ...request(), debug: true };

    await Assert.rejects(
      engineFor(request()).decide(badRequest as unknown as DecisionRequest),
      errorCode("INVALID_INPUT"),
    );
    await Assert.rejects(
      engineFor(request()).decide(extraField as unknown as DecisionRequest),
      errorCode("INVALID_INPUT"),
    );
    Assert.deepEqual(touches, []);
  });

  it("decodes eligibility candidates as passive exact data", async () => {
    const forRequest = request();
    const touches: string[] = [];
    const badCandidate = { ...candidate("candidate-one") };
    Object.defineProperty(badCandidate, "authorization", {
      enumerable: true,
      get() {
        touches.push("authorization");
        throw new Error("eligibility getter invoked");
      },
    });
    const commits: string[] = [];
    const engine = engineFor(forRequest, {
      eligibilityPort: {
        async resolveEligibility() {
          return eligibility(forRequest, [badCandidate as unknown as EligibilityCandidate]);
        },
      },
      commits,
    });

    await Assert.rejects(engine.decide(forRequest), errorCode("BACKEND_PROTOCOL"));
    Assert.deepEqual(touches, []);
    Assert.deepEqual(commits, []);
  });

  it("rejects an enumerable extra eligibility candidate field before backend or commit", async () => {
    const forRequest = request();
    const candidateWithDebug = { ...candidate("candidate-one"), debug: true };
    let backendCalls = 0;
    const commits: string[] = [];
    const engine = engineFor(forRequest, {
      eligibilityPort: {
        async resolveEligibility() {
          return eligibility(forRequest, [candidateWithDebug as unknown as EligibilityCandidate]);
        },
      },
      backend: {
        kind: "FAKE",
        async evaluate(input) {
          backendCalls++;
          return ranked(input.candidates.map((row) => row.candidateId));
        },
      },
      commits,
    });

    await Assert.rejects(engine.decide(forRequest), errorCode("BACKEND_PROTOCOL"));
    Assert.equal(backendCalls, 0);
    Assert.deepEqual(commits, []);
  });

  it("decodes backend results as passive exact data", async () => {
    const forRequest = request();
    const touches: string[] = [];
    const badResult = {
      kind: "RANKED",
      modelResolved: "fixture-v1",
    } as Record<string, unknown>;
    Object.defineProperty(badResult, "ranks", {
      enumerable: true,
      get() {
        touches.push("ranks");
        throw new Error("backend result getter invoked");
      },
    });
    const commits: string[] = [];
    const engine = engineFor(forRequest, {
      backend: {
        kind: "FAKE",
        async evaluate() {
          return badResult as unknown as DecisionBackendResult;
        },
      },
      commits,
    });

    await Assert.rejects(engine.decide(forRequest), errorCode("BACKEND_PROTOCOL"));
    Assert.deepEqual(touches, []);
    Assert.deepEqual(commits, []);
  });

  it("rejects enumerable extra fields on RANKED results and rank rows", async () => {
    const forRequest = request();
    const valid = ranked(["candidate-one"]);
    if (valid.kind !== "RANKED") throw new Error("ranked result required by this test");
    const invalidResults: ReadonlyArray<readonly [string, DecisionBackendResult]> = [
      ["RANKED outer extra field", { ...valid, debug: true } as unknown as DecisionBackendResult],
      [
        "RANKED row extra field",
        {
          ...valid,
          ranks: [{ ...valid.ranks[0]!, debug: true }],
        } as unknown as DecisionBackendResult,
      ],
    ];

    for (const [label, invalidResult] of invalidResults) {
      const commits: string[] = [];
      const engine = engineFor(forRequest, {
        backend: {
          kind: "FAKE",
          async evaluate() {
            return invalidResult;
          },
        },
        commits,
      });

      await Assert.rejects(engine.decide(forRequest), errorCode("BACKEND_PROTOCOL"), label);
      Assert.deepEqual(commits, [], label);
    }
  });

  it("removes every denied, unknown, unqualified, or under-capacity candidate before backend evaluation", async () => {
    const rows = [
      candidate("eligible"),
      candidate("denied", { authorization: "DENIED" }),
      candidate("authorization-unknown", { authorization: "UNKNOWN" }),
      candidate("capability-unknown", { capability: "UNKNOWN" }),
      candidate("isolation-unknown", { isolation: "UNKNOWN" }),
      candidate("capability-unqualified", { capability: "UNQUALIFIED" }),
      candidate("isolation-unqualified", { isolation: "UNQUALIFIED" }),
      candidate("capacity-unknown", { capacity: { required: 1, available: null } }),
      candidate("capacity-insufficient", { capacity: { required: 3, available: 2 } }),
    ];
    const forRequest = request(
      "DF02",
      rows.map((row) => row.candidateId),
    );
    const visible: string[][] = [];
    const commits: string[] = [];
    const engine = engineFor(forRequest, {
      candidates: rows,
      backend: {
        kind: "FAKE",
        async evaluate(input) {
          visible.push(input.candidates.map((row) => row.candidateId));
          return ranked(input.candidates.map((row) => row.candidateId));
        },
      },
      commits,
    });

    const result = await engine.decide(forRequest);
    Assert.deepEqual(visible, [["eligible"]]);
    Assert.deepEqual(commits, [`opr_${"1".repeat(32)}`]);
    Assert.equal(result.kind, "COMMITTED");
    if (result.kind === "COMMITTED") Assert.equal(result.record.choice, "eligible");
  });

  it("does not invoke a backend when only hard-ineligible candidates remain", async () => {
    const denied = candidate("denied", { authorization: "DENIED" });
    const forRequest = request("DF02", [denied.candidateId]);
    let backendCalls = 0;
    const commits: string[] = [];
    const engine = engineFor(forRequest, {
      candidates: [denied],
      backend: {
        kind: "FAKE",
        async evaluate() {
          backendCalls++;
          return { kind: "NONE", modelResolved: null };
        },
      },
      commits,
    });

    const result = await engine.decide(forRequest);
    Assert.equal(result.kind, "NONE");
    Assert.equal(result.record.state, "ABSTAINED");
    Assert.equal(backendCalls, 0);
    Assert.deepEqual(commits, []);
  });

  it("rejects RANKED omissions, duplicates, expanded candidates, and non-finite ranks", async () => {
    const ids = ["candidate-one", "candidate-two"];
    const forRequest = request("DF02", ids);
    const invalidResults: ReadonlyArray<readonly [string, DecisionBackendResult]> = [
      [
        "omitted candidate",
        { kind: "RANKED", modelResolved: null, ranks: [{ candidateId: ids[0]!, semanticRank: 1 }] },
      ],
      [
        "duplicate candidate",
        {
          kind: "RANKED",
          modelResolved: null,
          ranks: [
            { candidateId: ids[0]!, semanticRank: 1 },
            { candidateId: ids[0]!, semanticRank: 2 },
          ],
        },
      ],
      [
        "expanded candidate",
        {
          kind: "RANKED",
          modelResolved: null,
          ranks: [
            { candidateId: ids[0]!, semanticRank: 1 },
            { candidateId: "outside-request", semanticRank: 2 },
          ],
        },
      ],
      ...[Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY].map(
        (value) =>
          [
            `non-finite rank ${String(value)}`,
            {
              kind: "RANKED",
              modelResolved: null,
              ranks: [
                { candidateId: ids[0]!, semanticRank: value },
                { candidateId: ids[1]!, semanticRank: 2 },
              ],
            },
          ] as const,
      ),
    ];

    for (const [label, result] of invalidResults) {
      const commits: string[] = [];
      const engine = engineFor(forRequest, {
        backend: {
          kind: "FAKE",
          async evaluate() {
            return result;
          },
        },
        commits,
      });
      await Assert.rejects(engine.decide(forRequest), errorCode("BACKEND_PROTOCOL"), label);
      Assert.deepEqual(commits, [], label);
    }
  });

  it("records NONE as an abstention and never commits", async () => {
    const forRequest = request();
    const commits: string[] = [];
    const engine = engineFor(forRequest, {
      backend: {
        kind: "FAKE",
        async evaluate() {
          return { kind: "NONE", modelResolved: null };
        },
      },
      commits,
    });

    const result = await engine.decide(forRequest);
    Assert.equal(result.kind, "NONE");
    Assert.equal(result.record.state, "ABSTAINED");
    Assert.equal(result.record.choice, null);
    Assert.deepEqual(commits, []);
  });

  it("keeps closed and NOT_QUALIFIED backends on the safe abstain path", async () => {
    const forRequest = request();
    let eligibilityCalls = 0;
    let backendCalls = 0;
    const commits: string[] = [];
    const eligibilityPort: DecisionEligibilityPort = {
      async resolveEligibility() {
        eligibilityCalls++;
        return eligibility(forRequest, [candidate("candidate-one")]);
      },
    };
    const neverCommit = commitPort(commits);

    for (const backend of [CLOSED_JEV_BACKEND, CLOSED_GENERATIVE_BACKEND]) {
      const result = await new DecisionEngine(eligibilityPort, backend, neverCommit).decide(
        forRequest,
      );
      Assert.equal(result.kind, "NEEDS_EVIDENCE");
      Assert.equal(result.record.state, "ABSTAINED");
    }
    const notQualified = await new DecisionEngine(
      eligibilityPort,
      {
        kind: "FAKE",
        async evaluate() {
          backendCalls++;
          throw new DecisionBackendClosedError();
        },
      },
      neverCommit,
    ).decide(forRequest);

    Assert.equal(notQualified.kind, "NEEDS_EVIDENCE");
    Assert.equal(notQualified.record.state, "ABSTAINED");
    Assert.equal(eligibilityCalls, 1);
    Assert.equal(backendCalls, 1);
    Assert.deepEqual(commits, []);
  });

  it("blocks commit for every proposal_or_replay scenario", async () => {
    const scenarios = DECISION_SCENARIOS.filter((entry) => entry.s1Mode === "proposal_or_replay");
    Assert.ok(scenarios.length > 0);

    for (const scenario of scenarios) {
      const forRequest = request(scenario.id);
      const commits: string[] = [];
      const result = await engineFor(forRequest, { commits }).decide(forRequest);
      Assert.equal(result.kind, "NEEDS_REASONING", scenario.id);
      Assert.equal(result.record.state, "ABSTAINED", scenario.id);
      Assert.equal(result.record.choice, null, scenario.id);
      Assert.deepEqual(commits, [], scenario.id);
    }
  });
});
