import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import type { NativeStoreSession } from "../bootstrap/nativeStoreService.ts";
import {
  createNativeDecisionCommitPort,
  type NativeDecisionCommitBasis,
} from "./nativeAuthority.ts";
import type { DecisionCommitCommand, DecisionRecord } from "./engine/engine.ts";

const record = (): DecisionRecord => ({
  operationId: "decision-op",
  scenarioId: "DF02",
  family: "RESOURCE_SELECTION",
  state: "COMMITTED",
  stateViewHash: "sha256:" + "a".repeat(64),
  candidateHash: "sha256:" + "b".repeat(64),
  questionVersion: "1",
  rubricVersion: "1",
  modelRequested: null,
  modelResolved: "fake-v1",
  taskRevision: "1",
  policyRevision: "1",
  capabilityRevision: "3",
  bindingGeneration: "7",
  backendKind: "FAKE",
  choice: "candidate-one",
  reason: "QUALIFIED_BOUNDED_SELECTION",
  budgetUnits: 1,
  deadlineEpochMs: 1000,
});
const basis = (): NativeDecisionCommitBasis => ({
  operationId: "decision-op",
  domainId: "domain-one",
  decisionId: "decision-one",
  eventId: "event-one",
  receiptId: "receipt-one",
  recordedAt: "2026-09-21T00:00:00Z",
  candidates: [
    {
      candidateId: "candidate-one",
      requiredCapacityUnits: 1,
      snapshot: {
        operationId: "decision-op",
        candidateId: "candidate-one",
        stateViewHash: "sha256:" + "a".repeat(64),
        candidateHash: "sha256:" + "b".repeat(64),
        taskRevision: "1",
        policyRevision: "1",
        capabilityRevision: "3",
        bindingId: "binding-one",
        bindingGeneration: "7",
        authRevision: "2",
        resourceRef: "pool-one",
        resourceRevision: "5",
        capacityTotal: 2,
        actionOperationId: "opr_11111111111111111111111111111111",
        actionDigest: "sha256:" + "c".repeat(64),
      },
    },
  ],
});
const command = (): DecisionCommitCommand => ({
  record: record(),
  reservation: {
    candidateId: "candidate-one",
    resourceReservationRef: "capacity-one",
  },
  action: { actionIntentRef: "opr_11111111111111111111111111111111" },
});

function store(log: string[]): NativeStoreSession {
  return {
    async publishDecisionSnapshot(input) {
      log.push("publish:" + input.candidateId);
    },
    async commitDecision(input) {
      log.push("commit:" + input.record.choice);
      return {
        kind: "committed",
        operationId: "decision-op",
        decisionReceiptId: "receipt-one",
      };
    },
    async readDecisionReplay() {
      throw new Error("unused");
    },
    async publishContextAssemblySnapshot() {
      throw new Error("unused");
    },
    async commitTaskContextRequirements() {
      throw new Error("unused");
    },
    async readTaskContextRequirements() {
      throw new Error("unused");
    },
    async readContextAssemblyBasis() {
      throw new Error("unused");
    },
    async listContextAssemblySources() {
      throw new Error("unused");
    },
    async readGranteeContextSet() {
      throw new Error("unused");
    },
    async commitContextManifest() {
      throw new Error("unused");
    },
    async readContextManifest() {
      throw new Error("unused");
    },
    async commitProject() {
      throw new Error("unused");
    },
    async readSnapshot() {
      throw new Error("unused");
    },
    async getReceipt() {
      throw new Error("unused");
    },
    async commitContextVersion() {
      throw new Error("unused");
    },
    async reserve() {
      throw new Error("unused");
    },
    async begin() {
      throw new Error("unused");
    },
    async recordDispatchOutcome() {
      throw new Error("unused");
    },
    async close() {},
  };
}

describe("native Decision commit adapter", () => {
  it("publishes the exact selected snapshot before native atomic commit", async () => {
    const log: string[] = [];
    const result = await createNativeDecisionCommitPort(
      store(log),
      basis(),
    ).commitDecisionReservation(command());
    Assert.deepEqual(log, ["publish:candidate-one", "commit:candidate-one"]);
    Assert.equal(result.kind, "committed");
  });
  it("rejects stale basis before touching native authority", async () => {
    const log: string[] = [];
    const base = command();
    const c: DecisionCommitCommand = { ...base, record: { ...base.record, policyRevision: "2" } };
    const result = await createNativeDecisionCommitPort(
      store(log),
      basis(),
    ).commitDecisionReservation(c);
    Assert.equal(result.kind, "stale");
    Assert.deepEqual(log, []);
  });
  it("rejects Action intent substitution before touching native authority", async () => {
    const log: string[] = [];
    const base = command();
    const c: DecisionCommitCommand = {
      ...base,
      action: { actionIntentRef: "opr_22222222222222222222222222222222" },
    };
    const result = await createNativeDecisionCommitPort(
      store(log),
      basis(),
    ).commitDecisionReservation(c);
    Assert.equal(result.kind, "conflict");
    Assert.deepEqual(log, []);
  });
  it("does not admit live external backends into the S1 native commit seam", async () => {
    const log: string[] = [];
    const base = command();
    const c: DecisionCommitCommand = { ...base, record: { ...base.record, backendKind: "JEV" } };
    const result = await createNativeDecisionCommitPort(
      store(log),
      basis(),
    ).commitDecisionReservation(c);
    Assert.equal(result.kind, "denied");
    Assert.deepEqual(log, []);
  });
  it("returns native durable replay record instead of the recomputed record", async () => {
    const log: string[] = [];
    const s = store(log);
    s.commitDecision = async (input) => ({
      kind: "replayed",
      operationId: "decision-op",
      decisionReceiptId: "receipt-old",
      record: { ...input.record, choice: "candidate-old" },
    });
    const result = await createNativeDecisionCommitPort(s, basis()).commitDecisionReservation(
      command(),
    );
    Assert.equal(result.kind, "replayed");
    if (result.kind === "replayed") Assert.equal(result.record.choice, "candidate-old");
  });
  it("lets uncertain native transport escape instead of converting it into retryable denial", async () => {
    const log: string[] = [];
    const s = store(log);
    s.commitDecision = async () => {
      throw new Error("transport lost");
    };
    await Assert.rejects(
      () => createNativeDecisionCommitPort(s, basis()).commitDecisionReservation(command()),
      /transport lost/,
    );
  });
});
