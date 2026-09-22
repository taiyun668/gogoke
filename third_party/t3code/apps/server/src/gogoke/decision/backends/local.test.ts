import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import { decisionScenario } from "../families/registry.ts";
import { DecisionEngineError } from "../engine/passive.ts";
import type {
  DecisionBackendResult,
  DecisionRequest,
  EligibilityCandidate,
} from "../engine/engine.ts";
import type { DecisionScenarioDefinition } from "../families/registry.ts";
import {
  createFixtureBackend,
  createReplayBackend,
  createRulesBackend,
  type DecisionBackendInput,
  type LocalDecisionTranscript,
} from "./local.ts";

function getScenario() {
  const value = decisionScenario("DF02");
  if (value === null) throw new Error("DF02 scenario is required by this test");
  return value;
}
const scenario = getScenario();

function candidate(candidateId: string, priorityClass: number): EligibilityCandidate {
  return {
    candidateId,
    authorization: "ALLOWED",
    capability: "QUALIFIED",
    isolation: "QUALIFIED",
    capacity: { required: 1, available: 4 },
    priorityClass,
    waitingMs: 20,
    estimatedCost: 0.25,
    recipeRef: `recipe-${candidateId}`,
    resourceReservationRef: `reservation-${candidateId}`,
    actionIntentRef: `action-${candidateId}`,
  };
}

function input(operationId = "operation-one"): DecisionBackendInput {
  return {
    scenario,
    request: {
      operationId,
      scenarioId: scenario.id,
      stateViewHash: "state-view-one",
      candidateHash: "candidate-hash-one",
      questionVersion: "question-one",
      rubricVersion: "rubric-one",
      modelRequested: null,
      taskRevision: "1",
      policyRevision: "2",
      capabilityRevision: "3",
      bindingGeneration: "4",
      budgetUnits: 5,
      deadlineEpochMs: 2000,
      candidateRefs: ["candidate-one", "candidate-two"],
    },
    candidates: [candidate("candidate-one", 2), candidate("candidate-two", 1)],
  };
}

function rankedResult(value: DecisionBackendInput = input()): DecisionBackendResult {
  return {
    kind: "RANKED",
    modelResolved: null,
    ranks: value.candidates.map((row, index) => ({
      candidateId: row.candidateId,
      semanticRank: index + 1,
    })),
  };
}

function entry(
  value: DecisionBackendInput = input(),
  result: DecisionBackendResult = rankedResult(value),
): LocalDecisionTranscript {
  return { input: value, result };
}

function validationError(error: unknown): error is DecisionEngineError {
  return (
    error instanceof DecisionEngineError &&
    (error.code === "INVALID_INPUT" || error.code === "BACKEND_PROTOCOL")
  );
}

function assertConstructorRejects(run: () => unknown, label: string): void {
  Assert.throws(run, validationError, label);
}

async function assertInputRejects(value: unknown, label: string): Promise<void> {
  await Assert.rejects(
    () => createRulesBackend().evaluate(value as DecisionBackendInput),
    validationError,
    label,
  );
}

function replaceFirstCandidate(
  value: DecisionBackendInput,
  patch: Partial<EligibilityCandidate>,
): DecisionBackendInput {
  const first = value.candidates[0];
  if (first === undefined) throw new Error("candidate-one is required by this test");
  return {
    ...value,
    candidates: [{ ...first, ...patch }, ...value.candidates.slice(1)],
  };
}

type PackedValue = string | number | null;

const packedScenarioFields = [
  "id",
  "key",
  "family",
  "purpose",
  "ceiling",
  "s1Mode",
  "liveStatus",
] as const satisfies ReadonlyArray<keyof DecisionScenarioDefinition>;
const packedRequestFields = [
  "operationId",
  "scenarioId",
  "stateViewHash",
  "candidateHash",
  "questionVersion",
  "rubricVersion",
  "modelRequested",
  "taskRevision",
  "policyRevision",
  "capabilityRevision",
  "bindingGeneration",
  "budgetUnits",
  "deadlineEpochMs",
] as const satisfies ReadonlyArray<Exclude<keyof DecisionRequest, "candidateRefs">>;

function independentlyPackedValueUtf8Bytes(value: PackedValue): number {
  const text = value === null ? "" : String(value);
  const type = value === null ? "n" : typeof value === "number" ? "d" : "s";
  return new TextEncoder().encode(`${type}${text.length}:${text}`).byteLength;
}

function independentlyPackedUtf8Bytes(values: readonly PackedValue[]): number {
  return values.reduce<number>(
    (total, value) => total + independentlyPackedValueUtf8Bytes(value),
    0,
  );
}

// Independent test oracle for the documented length-prefixed identity bytes.
// The callable source-import graph itself is checked by read-only review, not a node:fs test.
function independentlySerializedInputBytes(value: DecisionBackendInput): number {
  const values: PackedValue[] = ["gogoke.local-decision-transcript.v1"];
  for (const key of packedScenarioFields) values.push(value.scenario[key]);
  for (const key of packedRequestFields) values.push(value.request[key]);
  values.push(value.request.candidateRefs.length);
  for (const ref of value.request.candidateRefs) values.push(ref);
  values.push(value.candidates.length);
  for (const row of value.candidates) {
    values.push(
      row.candidateId,
      row.authorization,
      row.capability,
      row.isolation,
      row.capacity.required,
      row.capacity.available,
      row.priorityClass,
      row.waitingMs,
      row.estimatedCost,
      row.recipeRef,
      row.resourceReservationRef,
      row.actionIntentRef,
    );
  }
  return independentlyPackedUtf8Bytes(values);
}

function independentlySerializedResultBytes(result: DecisionBackendResult): number {
  const values: PackedValue[] = [result.kind, result.modelResolved];
  if (result.kind === "RANKED") {
    for (const row of result.ranks) values.push(row.candidateId, row.semanticRank);
  }
  return independentlyPackedUtf8Bytes(values);
}

function boundaryTemplate(operationId: string) {
  const candidates = Array.from({ length: 256 }, (_, index) => ({
    candidateId: `boundary-candidate-${index}`,
    authorization: "ALLOWED" as const,
    capability: "QUALIFIED" as const,
    isolation: "QUALIFIED" as const,
    capacity: { required: 1, available: 2 },
    priorityClass: index,
    waitingMs: index,
    estimatedCost: 0,
    recipeRef: "r",
    resourceReservationRef: `boundary-reservation-${index}`,
    actionIntentRef: `boundary-action-${index}`,
  }));
  return {
    scenario,
    request: {
      operationId,
      scenarioId: scenario.id,
      stateViewHash: "state-view-one",
      candidateHash: "candidate-hash-one",
      questionVersion: "question-one",
      rubricVersion: "rubric-one",
      modelRequested: null,
      taskRevision: "1",
      policyRevision: "2",
      capabilityRevision: "3",
      bindingGeneration: "4",
      budgetUnits: 5,
      deadlineEpochMs: 2000,
      candidateRefs: candidates.map((row) => row.candidateId),
    },
    candidates,
  } satisfies DecisionBackendInput;
}

function inputWithExactSerializedBytes(
  targetBytes: number,
  operationId: string,
): DecisionBackendInput {
  const value = boundaryTemplate(operationId);
  let measured = independentlySerializedInputBytes(value);
  Assert.ok(
    measured <= targetBytes,
    "the boundary template must be smaller than the requested byte size",
  );
  for (const row of value.candidates) {
    const fixedBytes = measured - independentlyPackedValueUtf8Bytes(row.recipeRef);
    let low = 1;
    let high = 4096;
    let bestLength = 1;
    let bestBytes = fixedBytes + independentlyPackedValueUtf8Bytes("r");
    while (low <= high) {
      const length = Math.floor((low + high) / 2);
      const trialBytes = fixedBytes + independentlyPackedValueUtf8Bytes("r".repeat(length));
      if (trialBytes <= targetBytes) {
        bestLength = length;
        bestBytes = trialBytes;
        low = length + 1;
      } else {
        high = length - 1;
      }
    }
    row.recipeRef = "r".repeat(bestLength);
    measured = bestBytes;
    if (measured === targetBytes) break;
  }
  if (measured < targetBytes) {
    const missingBytes = targetBytes - measured;
    Assert.ok(
      missingBytes === 1 || missingBytes === 2,
      "remaining byte gap must fit a same-length UTF-8 character",
    );
    const first = value.candidates[0];
    if (first === undefined) throw new Error("boundary candidate is required by this test");
    first.recipeRef = `${missingBytes === 1 ? "é" : "中"}${first.recipeRef.slice(1)}`;
    measured = independentlySerializedInputBytes(value);
  }
  Assert.equal(measured, targetBytes, "fixture must measure at the exact UTF-8 boundary");
  return value;
}

function transcriptWithExactSerializedBytes(
  targetBytes: number,
  operationId: string,
): LocalDecisionTranscript {
  const seed = boundaryTemplate(operationId);
  const resultBytes = independentlySerializedResultBytes(rankedResult(seed));
  const value = inputWithExactSerializedBytes(targetBytes - resultBytes, operationId);
  const result = rankedResult(value);
  Assert.equal(independentlySerializedResultBytes(result), resultBytes);
  Assert.equal(independentlySerializedInputBytes(value) + resultBytes, targetBytes);
  return entry(value, result);
}

describe("local Decision backends", () => {
  it("keeps RULES deterministic across instances and ambient clock/random changes", async () => {
    const originalNow = Date.now;
    const originalRandom = Math.random;
    let ambientReads = 0;
    let clockValue = 10;
    let randomValue = 0.1;
    let firstPromise: ReturnType<ReturnType<typeof createRulesBackend>["evaluate"]>;
    let secondPromise: ReturnType<ReturnType<typeof createRulesBackend>["evaluate"]>;
    try {
      Date.now = () => {
        ambientReads++;
        return clockValue;
      };
      Math.random = () => {
        ambientReads++;
        return randomValue;
      };
      firstPromise = createRulesBackend().evaluate(input());
      clockValue = 20;
      randomValue = 0.9;
      secondPromise = createRulesBackend().evaluate(input());
    } finally {
      Date.now = originalNow;
      Math.random = originalRandom;
    }
    await Promise.resolve();
    const first = await firstPromise;
    const second = await secondPromise;

    Assert.deepEqual(first, second);
    Assert.equal(ambientReads, 0);
    Assert.equal(first.kind, "RANKED");
    if (first.kind === "RANKED") {
      Assert.equal(first.modelResolved, null);
      Assert.deepEqual(
        first.ranks.map((rank) => rank.semanticRank),
        [0, 0],
      );
    }
    const empty = input();
    const none = await createRulesBackend().evaluate({ ...empty, candidates: [] });
    Assert.equal(none.kind, "NONE");
    if (none.kind !== "NONE") return;
    Assert.equal(none.modelResolved, null);
  });

  it("snapshots FAKE and REPLAY inputs, results, and transcript records at construction", async () => {
    for (const [kind, factory] of [
      ["FAKE", createFixtureBackend],
      ["REPLAY", createReplayBackend],
    ] as const) {
      const inputSnapshot = input();
      const resultSnapshot = rankedResult(inputSnapshot);
      const transcriptSnapshot = entry(inputSnapshot, resultSnapshot);
      const backend = factory([transcriptSnapshot]);

      Object.assign(inputSnapshot.request, { policyRevision: "99" });
      const firstCandidate = inputSnapshot.candidates[0];
      if (firstCandidate === undefined) throw new Error("first candidate is required by this test");
      Object.assign(firstCandidate, {
        recipeRef: "changed-recipe",
        resourceReservationRef: "changed-reservation",
        actionIntentRef: "changed-action",
      });
      if (resultSnapshot.kind !== "RANKED") throw new Error("ranked result required by this test");
      const firstRank = resultSnapshot.ranks[0];
      if (firstRank === undefined) throw new Error("first rank is required by this test");
      Object.assign(firstRank, { semanticRank: 99 });
      Object.assign(transcriptSnapshot, {
        input: input("replacement-operation"),
        result: { kind: "NONE", modelResolved: null },
      });

      const returned = await backend.evaluate(input());
      Assert.equal(backend.kind, kind);
      Assert.equal(returned.kind, "RANKED");
      if (returned.kind !== "RANKED") continue;
      Assert.deepEqual(
        returned.ranks.map((rank) => rank.semanticRank),
        [1, 2],
      );
      Assert.equal(Object.isFrozen(returned), true);
      Assert.equal(Object.isFrozen(returned.ranks), true);
      Assert.equal(Object.isFrozen(returned.ranks[0]), true);
    }
  });

  it("requires exact request, revision, candidate, recipe, reservation, and action identity", async () => {
    type InputChange = (value: DecisionBackendInput) => DecisionBackendInput;
    const requestChanges = {
      operationId: (value) => ({
        ...value,
        request: { ...value.request, operationId: "operation-two" },
      }),
      scenarioId: (value) => {
        const next = decisionScenario("DF03");
        if (next === null) throw new Error("DF03 scenario is required by this test");
        return { ...value, scenario: next, request: { ...value.request, scenarioId: next.id } };
      },
      stateViewHash: (value) => ({
        ...value,
        request: { ...value.request, stateViewHash: "state-view-two" },
      }),
      candidateHash: (value) => ({
        ...value,
        request: { ...value.request, candidateHash: "candidate-hash-two" },
      }),
      questionVersion: (value) => ({
        ...value,
        request: { ...value.request, questionVersion: "question-two" },
      }),
      rubricVersion: (value) => ({
        ...value,
        request: { ...value.request, rubricVersion: "rubric-two" },
      }),
      modelRequested: (value) => ({
        ...value,
        request: { ...value.request, modelRequested: "model-two" },
      }),
      taskRevision: (value) => ({ ...value, request: { ...value.request, taskRevision: "2" } }),
      policyRevision: (value) => ({ ...value, request: { ...value.request, policyRevision: "3" } }),
      capabilityRevision: (value) => ({
        ...value,
        request: { ...value.request, capabilityRevision: "4" },
      }),
      bindingGeneration: (value) => ({
        ...value,
        request: { ...value.request, bindingGeneration: "5" },
      }),
      budgetUnits: (value) => ({ ...value, request: { ...value.request, budgetUnits: 6 } }),
      deadlineEpochMs: (value) => ({
        ...value,
        request: { ...value.request, deadlineEpochMs: 2001 },
      }),
      candidateRefs: (value) => ({
        ...value,
        request: { ...value.request, candidateRefs: [...value.request.candidateRefs].reverse() },
      }),
    } satisfies Record<keyof DecisionRequest, InputChange>;
    const candidateChanges = {
      candidateId: (value) => {
        const changed = replaceFirstCandidate(value, { candidateId: "candidate-renamed" });
        return {
          ...changed,
          request: {
            ...changed.request,
            candidateRefs: changed.request.candidateRefs.map((ref) =>
              ref === "candidate-one" ? "candidate-renamed" : ref,
            ),
          },
        };
      },
      authorization: (value) => replaceFirstCandidate(value, { authorization: "DENIED" }),
      capability: (value) => replaceFirstCandidate(value, { capability: "UNQUALIFIED" }),
      isolation: (value) => replaceFirstCandidate(value, { isolation: "UNQUALIFIED" }),
      capacity: (value) =>
        replaceFirstCandidate(value, { capacity: { required: 2, available: 4 } }),
      priorityClass: (value) => replaceFirstCandidate(value, { priorityClass: 3 }),
      waitingMs: (value) => replaceFirstCandidate(value, { waitingMs: 21 }),
      estimatedCost: (value) => replaceFirstCandidate(value, { estimatedCost: 0.5 }),
      recipeRef: (value) => replaceFirstCandidate(value, { recipeRef: "recipe-changed" }),
      resourceReservationRef: (value) =>
        replaceFirstCandidate(value, { resourceReservationRef: "reservation-changed" }),
      actionIntentRef: (value) =>
        replaceFirstCandidate(value, { actionIntentRef: "action-changed" }),
    } satisfies Record<keyof EligibilityCandidate, InputChange>;
    const capacityChanges = {
      required: (value: DecisionBackendInput, next: number) =>
        replaceFirstCandidate(value, {
          capacity: { required: next, available: 4 },
        }),
      available: (value: DecisionBackendInput, next: number) =>
        replaceFirstCandidate(value, {
          capacity: { required: 1, available: next },
        }),
    } satisfies Record<
      keyof EligibilityCandidate["capacity"],
      (value: DecisionBackendInput, next: number) => DecisionBackendInput
    >;
    const changes: Array<[string, InputChange]> = [
      ...Object.entries(requestChanges),
      ...Object.entries(candidateChanges),
      ["capacity.required", (value) => capacityChanges.required(value, 2)],
      ["capacity.available", (value) => capacityChanges.available(value, 5)],
    ];

    for (const [kind, factory] of [
      ["FAKE", createFixtureBackend],
      ["REPLAY", createReplayBackend],
    ] as const) {
      const backend = factory([entry(input())]);
      for (const [field, change] of changes) {
        const result = await backend.evaluate(change(input()));
        Assert.equal(result.kind, "NEEDS_EVIDENCE", `${kind}: changed ${field} must not replay`);
      }
    }
  });

  it("rejects proxies, accessors, thenables, and unknown fields without invoking them", async () => {
    const backend = createRulesBackend();
    let touches = 0;
    const proxy = new Proxy(input(), {
      get() {
        touches++;
        throw new Error("proxy get");
      },
      ownKeys() {
        touches++;
        throw new Error("proxy keys");
      },
      getOwnPropertyDescriptor() {
        touches++;
        throw new Error("proxy descriptor");
      },
      getPrototypeOf() {
        touches++;
        throw new Error("proxy prototype");
      },
    });
    await Assert.rejects(() => backend.evaluate(proxy), validationError);

    const getterInput = { ...input() };
    Object.defineProperty(getterInput, "scenario", {
      enumerable: true,
      get() {
        touches++;
        throw new Error("scenario getter");
      },
    });
    await Assert.rejects(
      () => backend.evaluate(getterInput as DecisionBackendInput),
      validationError,
    );

    const thenable = Object.defineProperty({ ...input() }, "then", {
      enumerable: true,
      get() {
        touches++;
        throw new Error("input then getter");
      },
    });
    await Assert.rejects(() => backend.evaluate(thenable as DecisionBackendInput), validationError);

    const extra = { ...input(), debug: "unknown" } as unknown as DecisionBackendInput;
    await Assert.rejects(() => backend.evaluate(extra), validationError);
    Assert.equal(touches, 0);
  });

  it("rejects unknown transcript fields, duplicate identities, non-finite values, and wrong candidates", async () => {
    const base = input();
    const result = rankedResult(base);

    assertConstructorRejects(
      () => createFixtureBackend([{ ...entry(base), extra: true } as LocalDecisionTranscript]),
      "extra transcript field",
    );
    assertConstructorRejects(
      () =>
        createFixtureBackend([entry({ ...base, extra: true } as unknown as DecisionBackendInput)]),
      "extra input field",
    );
    assertConstructorRejects(
      () =>
        createFixtureBackend([
          entry({
            ...base,
            request: { ...base.request, extra: true } as unknown as DecisionRequest,
          }),
        ]),
      "extra request field",
    );
    assertConstructorRejects(
      () =>
        createFixtureBackend([
          entry({
            ...base,
            candidates: [{ ...base.candidates[0]!, extra: true }, base.candidates[1]!],
          } as unknown as DecisionBackendInput),
        ]),
      "extra candidate field",
    );
    assertConstructorRejects(
      () =>
        createFixtureBackend([
          entry(base, { ...result, extra: true } as unknown as DecisionBackendResult),
        ]),
      "extra result field",
    );
    if (result.kind !== "RANKED") throw new Error("ranked result required by this test");
    assertConstructorRejects(
      () =>
        createFixtureBackend([
          entry(base, {
            ...result,
            ranks: [{ ...result.ranks[0]!, extra: true }, result.ranks[1]!],
          } as unknown as DecisionBackendResult),
        ]),
      "extra rank field",
    );
    const scenarioExtra = {
      ...base,
      scenario: { ...base.scenario, debug: true },
    } as unknown as DecisionBackendInput;
    assertConstructorRejects(
      () => createFixtureBackend([entry(scenarioExtra)]),
      "extra scenario field",
    );
    const first = base.candidates[0];
    if (first === undefined) throw new Error("first candidate is required by this test");
    const capacityExtra = {
      ...base,
      candidates: [{ ...first, capacity: { ...first.capacity, debug: true } }, base.candidates[1]!],
    } as unknown as DecisionBackendInput;
    assertConstructorRejects(
      () => createFixtureBackend([entry(capacityExtra)]),
      "extra capacity field",
    );

    const wrongInput = replaceFirstCandidate(base, { candidateId: "candidate-outside-request" });
    assertConstructorRejects(
      () => createFixtureBackend([entry(wrongInput)]),
      "unrequested candidate id",
    );
    const wrongResult: DecisionBackendResult = {
      ...result,
      ranks: [
        { candidateId: "candidate-outside-request", semanticRank: 1 },
        { candidateId: "candidate-two", semanticRank: 2 },
      ],
    };
    assertConstructorRejects(
      () => createFixtureBackend([entry(base, wrongResult)]),
      "wrong result candidate",
    );

    const duplicateRefs: DecisionBackendInput = {
      ...base,
      request: { ...base.request, candidateRefs: ["candidate-one", "candidate-one"] },
    };
    assertConstructorRejects(
      () => createFixtureBackend([entry(duplicateRefs)]),
      "duplicate request candidate",
    );
    const duplicateCandidates: DecisionBackendInput = {
      ...base,
      candidates: [base.candidates[0]!, { ...base.candidates[0]! }],
    };
    assertConstructorRejects(
      () => createFixtureBackend([entry(duplicateCandidates)]),
      "duplicate candidate identity",
    );
    assertConstructorRejects(
      () => createFixtureBackend([entry(base), entry(base)]),
      "duplicate transcript identity",
    );
    assertConstructorRejects(
      () =>
        createFixtureBackend([
          entry(base, {
            ...result,
            ranks: [result.ranks[0]!, result.ranks[0]!],
          }),
        ]),
      "duplicate result rank",
    );
  });

  it("rejects nested accessors without invoking them", async () => {
    const base = input();
    let getterTouches = 0;
    const request = { ...base.request };
    Object.defineProperty(request, "policyRevision", {
      enumerable: true,
      get() {
        getterTouches++;
        throw new Error("policyRevision getter invoked");
      },
    });
    await assertInputRejects({ ...base, request: request as DecisionRequest }, "request getter");

    const refs = [...base.request.candidateRefs];
    Object.defineProperty(refs, "0", {
      configurable: true,
      enumerable: true,
      get() {
        getterTouches++;
        throw new Error("candidateRefs getter invoked");
      },
    });
    await assertInputRejects(
      {
        ...base,
        request: { ...base.request, candidateRefs: refs },
      },
      "candidateRefs getter",
    );

    const first = base.candidates[0];
    if (first === undefined) throw new Error("first candidate is required by this test");
    const capacity = { ...first.capacity };
    Object.defineProperty(capacity, "available", {
      enumerable: true,
      get() {
        getterTouches++;
        throw new Error("capacity getter invoked");
      },
    });
    await assertInputRejects(
      {
        ...base,
        candidates: [{ ...first, capacity }, base.candidates[1]!],
      },
      "capacity getter",
    );
    const result = rankedResult(base);
    if (result.kind !== "RANKED") throw new Error("ranked result required by this test");
    const firstRank = { ...result.ranks[0]! };
    Object.defineProperty(firstRank, "semanticRank", {
      enumerable: true,
      get() {
        getterTouches++;
        throw new Error("semanticRank getter invoked");
      },
    });
    assertConstructorRejects(
      () =>
        createFixtureBackend([
          entry(base, {
            ...result,
            ranks: [firstRank, result.ranks[1]!],
          } as unknown as DecisionBackendResult),
        ]),
      "rank getter",
    );
    Assert.equal(getterTouches, 0);
  });

  it("checks every count, cost, revision, and rank numeric contract", async () => {
    type NumericInputChange = (value: DecisionBackendInput, next: number) => DecisionBackendInput;
    const countFields = {
      budgetUnits: (value, next) => ({
        ...value,
        request: { ...value.request, budgetUnits: next },
      }),
      deadlineEpochMs: (value, next) => ({
        ...value,
        request: { ...value.request, deadlineEpochMs: next },
      }),
      "capacity.required": (value, next) =>
        replaceFirstCandidate(value, {
          capacity: { required: next, available: 4 },
        }),
      "capacity.available": (value, next) =>
        replaceFirstCandidate(value, {
          capacity: { required: 1, available: next },
        }),
      priorityClass: (value, next) => replaceFirstCandidate(value, { priorityClass: next }),
      waitingMs: (value, next) => replaceFirstCandidate(value, { waitingMs: next }),
    } satisfies Record<
      | "budgetUnits"
      | "deadlineEpochMs"
      | "capacity.required"
      | "capacity.available"
      | "priorityClass"
      | "waitingMs",
      NumericInputChange
    >;
    const invalidCounts = [
      Number.NaN,
      Number.POSITIVE_INFINITY,
      Number.NEGATIVE_INFINITY,
      -1,
      1.5,
      Number.MAX_SAFE_INTEGER + 1,
    ];
    for (const [field, change] of Object.entries(countFields)) {
      for (const next of invalidCounts) {
        await assertInputRejects(change(input(), next), `${field} rejects ${String(next)}`);
      }
      const accepted = await createRulesBackend().evaluate(
        change(input(), Number.MAX_SAFE_INTEGER),
      );
      Assert.equal(accepted.kind, "RANKED", `${field} accepts the safe-integer upper bound`);
    }
    const nullableCapacity = replaceFirstCandidate(input(), {
      capacity: { required: 1, available: null },
    });
    Assert.equal((await createRulesBackend().evaluate(nullableCapacity)).kind, "RANKED");

    const costs = {
      estimatedCost: (value: DecisionBackendInput, next: number) =>
        replaceFirstCandidate(value, { estimatedCost: next }),
    } satisfies Record<"estimatedCost", NumericInputChange>;
    for (const next of [
      Number.NaN,
      Number.POSITIVE_INFINITY,
      Number.NEGATIVE_INFINITY,
      -1,
      Number.MAX_SAFE_INTEGER + 1,
    ]) {
      await assertInputRejects(
        costs.estimatedCost(input(), next),
        `estimatedCost rejects ${String(next)}`,
      );
    }
    for (const next of [0, Number.MAX_SAFE_INTEGER]) {
      Assert.equal(
        (await createRulesBackend().evaluate(costs.estimatedCost(input(), next))).kind,
        "RANKED",
      );
    }

    type RevisionChange = (value: DecisionBackendInput, next: string) => DecisionBackendInput;
    const revisionFields = {
      taskRevision: (value, next) => ({
        ...value,
        request: { ...value.request, taskRevision: next },
      }),
      policyRevision: (value, next) => ({
        ...value,
        request: { ...value.request, policyRevision: next },
      }),
      capabilityRevision: (value, next) => ({
        ...value,
        request: { ...value.request, capabilityRevision: next },
      }),
      bindingGeneration: (value, next) => ({
        ...value,
        request: { ...value.request, bindingGeneration: next },
      }),
    } satisfies Record<
      "taskRevision" | "policyRevision" | "capabilityRevision" | "bindingGeneration",
      RevisionChange
    >;
    for (const [field, change] of Object.entries(revisionFields)) {
      for (const next of ["-1", "01", "1.5", "18446744073709551616"]) {
        await assertInputRejects(change(input(), next), `${field} rejects ${next}`);
      }
      Assert.equal(
        (await createRulesBackend().evaluate(change(input(), "18446744073709551615"))).kind,
        "RANKED",
        `${field} accepts the u64 upper bound`,
      );
    }

    const base = input();
    const baseResult = rankedResult(base);
    if (baseResult.kind !== "RANKED") throw new Error("ranked result required by this test");
    for (const next of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
      const badResult: DecisionBackendResult = {
        ...baseResult,
        ranks: [{ candidateId: "candidate-one", semanticRank: next }, baseResult.ranks[1]!],
      };
      assertConstructorRejects(
        () => createFixtureBackend([entry(base, badResult)]),
        `rank rejects ${String(next)}`,
      );
    }
    for (const next of [-Number.MAX_VALUE, Number.MAX_VALUE]) {
      const finiteResult: DecisionBackendResult = {
        ...baseResult,
        ranks: [{ candidateId: "candidate-one", semanticRank: next }, baseResult.ranks[1]!],
      };
      Assert.equal(createFixtureBackend([entry(base, finiteResult)]).kind, "FAKE");
    }

    const operationAtLimit = {
      ...input(),
      request: { ...input().request, operationId: "o".repeat(4096) },
    };
    Assert.equal((await createRulesBackend().evaluate(operationAtLimit)).kind, "RANKED");
    const operationOverLimit = {
      ...input(),
      request: { ...input().request, operationId: "o".repeat(4097) },
    };
    await assertInputRejects(operationOverLimit, "operationId exceeds its 4096 character limit");
    Assert.equal(
      (
        await createRulesBackend().evaluate(
          replaceFirstCandidate(input(), { recipeRef: "r".repeat(4096) }),
        )
      ).kind,
      "RANKED",
    );
    await assertInputRejects(
      replaceFirstCandidate(input(), { recipeRef: "r".repeat(4097) }),
      "recipeRef exceeds its 4096 character limit",
    );
    const tooManyCandidates = Array.from({ length: 257 }, (_, index) =>
      candidate(`candidate-${index}`, index),
    );
    const tooManyInput = input();
    await assertInputRejects(
      {
        ...tooManyInput,
        request: {
          ...tooManyInput.request,
          candidateRefs: tooManyCandidates.map((row) => row.candidateId),
        },
        candidates: tooManyCandidates,
      },
      "candidate arrays exceed 256 rows",
    );
  });

  it("enforces exact 1 MiB and 1 MiB plus one UTF-8 byte boundaries", async () => {
    const limit = 1024 * 1024;
    const exactInput = inputWithExactSerializedBytes(limit, "input-boundary-1048576");
    const overInput = inputWithExactSerializedBytes(limit + 1, "input-boundary-1048577");
    Assert.equal(independentlySerializedInputBytes(exactInput), 1048576);
    Assert.equal(independentlySerializedInputBytes(overInput), 1048577);
    Assert.equal((await createRulesBackend().evaluate(exactInput)).kind, "RANKED");
    await assertInputRejects(overInput, "serialized input is one byte over 1 MiB");

    const exactTranscript = transcriptWithExactSerializedBytes(limit, "aggregate-boundary-1");
    const overTranscript = transcriptWithExactSerializedBytes(limit + 1, "aggregate-boundary-2");
    Assert.equal(
      independentlySerializedInputBytes(exactTranscript.input) +
        independentlySerializedResultBytes(exactTranscript.result),
      1048576,
    );
    Assert.equal(
      independentlySerializedInputBytes(overTranscript.input) +
        independentlySerializedResultBytes(overTranscript.result),
      1048577,
    );
    Assert.equal(createFixtureBackend([exactTranscript]).kind, "FAKE");
    assertConstructorRejects(
      () => createFixtureBackend([overTranscript]),
      "transcript is one byte over 1 MiB",
    );
  });
});
