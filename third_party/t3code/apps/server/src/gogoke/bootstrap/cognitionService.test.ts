import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import { TypedActionDispatcher, type ActionBindingAuthority } from "../actions/typedAction.ts";
import type { NativeStoreSession } from "./nativeStoreService.ts";
import { constructCognitionService } from "./cognitionService.ts";

function store(log: string[]): NativeStoreSession {
  return {
    async commitProject() {
      throw new Error("unused");
    },
    async readSnapshot() {
      throw new Error("unused");
    },
    async getReceipt() {
      throw new Error("unused");
    },
    async commitContextVersion(input) {
      log.push("context:" + input.operationId);
      return {
        disposition: "COMMITTED",
        operationId: input.operationId,
        contextId: input.object.contextId,
        version: input.object.version,
        invalidatedVersionRefs: [],
      };
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
    async publishDecisionSnapshot(input) {
      log.push("snapshot:" + input.candidateId);
    },
    async commitDecision(input) {
      log.push("decision:" + input.record.choice);
      return {
        kind: "committed",
        operationId: input.record.operationId,
        decisionReceiptId: "decision-receipt",
      };
    },
    async readDecisionReplay() {
      throw new Error("unused");
    },
    async close() {},
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
  };
}

const unusedBindingAuthority: ActionBindingAuthority = {
  async currentBinding() {
    return null;
  },
  async revalidateTaskPackage() {
    throw new Error("unused");
  },
  async dispatchIfCurrent() {
    throw new Error("unused");
  },
};

function service(log: string[]) {
  const native = store(log);
  return constructCognitionService(
    native,
    new TypedActionDispatcher(native, unusedBindingAuthority),
  );
}

describe("internal cognition service composition", () => {
  it("binds ContextRepository to the one native store session", async () => {
    const log: string[] = [];
    const cognition = service(log);
    const receipt = await cognition.contextRepository.commit({
      operationId: "ctx-op",
      object: {
        contextId: "ctx-one",
        version: "1" as never,
        scope: "PROJECT",
        domainId: "domain-one",
        kind: "fact",
        contentHash: "sha256:" + "a".repeat(64),
        sourceRef: { ref: "source://one", hash: "sha256:" + "b".repeat(64) },
        sourceAuthority: { kind: "repository", ref: "authority://one" },
        derivedFrom: [],
        validity: "ACTIVE",
        supersedes: [],
        accessPolicyRevision: "1" as never,
      },
      access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
    });
    Assert.equal(receipt.operationId, "ctx-op");
    Assert.deepEqual(log, ["context:ctx-op"]);
  });

  it("exposes Evaluation and Dream as pure cognition functions, not services with lifecycle authority", () => {
    const cognition = service([]);
    Assert.equal(typeof cognition.evaluation.calibrationReport, "function");
    Assert.equal(typeof cognition.dream.createCandidate, "function");
    Assert.equal(Object.isFrozen(cognition.dream), true);
    Assert.equal("scheduler" in cognition.dream, false);
  });
});
