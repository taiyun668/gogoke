import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";

import {
  ACTION_INTENT_SCHEMA,
  TypedActionError,
  actionCommitmentForPackage,
  semanticDigestForAction,
  TypedActionDispatcher,
  type ActionBinding,
  type ActionBindingAuthority,
  type DurableActionReservation,
  type TypedActionIntent,
} from "../actions/typedAction.ts";
import type { ContextManifest, ContextObject } from "../contracts/model.ts";
import type { NativeBoundedContextSource } from "../context/assembly/nativeAuthorityPort.ts";
import { DecisionEngineError } from "../decision/engine/engine.ts";
import type { AuthorityCeiling, AuthorizedTaskPackage } from "../policy/types.ts";
import type {
  NativeAuthorizedContextReadSet,
  NativeContextAssemblyBasis,
  NativeContextAssemblySource,
  NativeContextManifestReceipt,
} from "../persistence/base/nativeHostClient.ts";
import type { NativeStoreSession } from "./nativeStoreService.ts";
import {
  CognitionLineageError,
  constructCognitionService,
  type CognitionVerticalConstruction,
} from "./cognitionService.ts";
import { hashData, hashText } from "../context/assembly/passive.ts";

const digest = (character: string): string => `sha256:${character.repeat(64)}`;
const ACTION_OPERATION = `opr_${"1".repeat(32)}`;

const binding: ActionBinding = Object.freeze({
  bindingId: "binding-one",
  sessionId: "session-one",
  executionId: "execution-one",
  runtimeInstanceId: "runtime-one",
  profileId: "profile-one",
  authRevision: "3",
  generation: "7",
});

const childCeiling: AuthorityCeiling = {
  allowedActions: ["return-result"],
  allowedTargetPrincipalIds: ["principal-controller"],
  allowedTargetDomainIds: ["domain-destination"],
  allowedSinks: ["task-package"],
  allowedMaterialClasses: ["spec"],
  explicitPrivateMaterialIds: [],
  allowedContinuationResponses: [],
  maxMaterialItems: 1,
  maxMaterialBytes: 1024,
  maxResponseBytes: 64,
};

const taskPackage: AuthorizedTaskPackage = Object.freeze({
  packageDigest: digest("a"),
  parentGrantRef: "parent-grant-one",
  parentGrantRevision: "1",
  parentGrantRevocationHead: "2",
  parentPolicyRevision: "3",
  parentSeatId: "seat-controller",
  parentGrantDigest: digest("f"),
  parentCeilingDigest: digest("b"),
  childCeiling,
  childCeilingDigest: digest("c"),
  action: "delegate",
  route: "controller-worker",
  source: Object.freeze({
    principalId: "principal-controller",
    projectId: "project-one",
    domainId: "domain-destination",
    role: "controller",
  }),
  target: Object.freeze({
    principalId: "principal-worker",
    projectId: "project-one",
    domainId: "domain-worker",
    role: "worker",
  }),
  sourceBinding: Object.freeze({
    sessionId: "session-controller",
    executionId: "execution-controller",
    generation: "1",
  }),
  targetBinding: Object.freeze({
    sessionId: binding.sessionId,
    executionId: binding.executionId,
    generation: binding.generation,
  }),
  targetBindingKind: "existing",
  sink: "task-package",
  instruction: "perform bounded synthetic action",
  instructionDigest: digest("d"),
  materialSetDigest: digest("e"),
  materials: Object.freeze([]),
});

const unsignedIntent = Object.freeze({
  schema: ACTION_INTENT_SCHEMA,
  operationId: ACTION_OPERATION,
  binding,
  taskPackage,
  action: Object.freeze({ kind: "prompt" as const, delivery: "queue" as const, text: "synthetic" }),
});
const actionIntent: TypedActionIntent = Object.freeze({
  ...unsignedIntent,
  semanticDigest: semanticDigestForAction(unsignedIntent),
});

const assemblySnapshot = Object.freeze({
  operationId: "assembly-operation",
  principalId: "principal-worker",
  seatId: "seat-worker",
  taskId: "task-one",
  sessionId: binding.sessionId,
  domainId: "domain-destination",
  bindingId: binding.bindingId,
  bindingGeneration: binding.generation,
  sourceEpoch: "9",
  runtimeInstanceId: binding.runtimeInstanceId,
  taskRevision: "1",
  policyRevision: "2",
  authRevision: binding.authRevision,
  revocationHead: "4",
  selectionDecisionId: "context-selection-one",
  manifestId: "manifest-one",
  admissionActionOperationId: ACTION_OPERATION,
  admissionDigest: actionIntent.semanticDigest,
  maxContentBytes: 1024,
  maxCandidates: 4,
  partitionBindings: Object.freeze([
    Object.freeze({
      sourceDomainId: "domain-source",
      destinationScope: "PROJECT" as const,
      promotionKind: "PROJECT_ONLY",
      grant: Object.freeze({ grantId: "grant-one", revision: "5", revocationHead: "4" }),
    }),
  ]),
});

const required = Object.freeze({
  sourceDomainId: "domain-source",
  contextId: "context-one",
  version: "1",
});
const sourceRow: NativeContextAssemblySource = Object.freeze({
  ...required,
  scope: "PROJECT",
  kind: "fact",
  contentHash: hashText("ordinary fixture signal"),
  sourceRef: "source://ordinary-fixture",
  sourceHash: digest("f"),
  sourceAuthorityKind: "repository",
  sourceAuthorityRef: "authority://fixture",
  accessPolicyRevision: "6",
  stateRevision: "7",
  grant: assemblySnapshot.partitionBindings[0]!.grant,
});
const contextObject: ContextObject = Object.freeze({
  contextId: sourceRow.contextId,
  version: sourceRow.version as ContextObject["version"],
  scope: "PROJECT",
  domainId: sourceRow.sourceDomainId,
  kind: sourceRow.kind,
  contentHash: sourceRow.contentHash,
  sourceRef: Object.freeze({ ref: sourceRow.sourceRef, hash: sourceRow.sourceHash }),
  sourceAuthority: Object.freeze({
    kind: sourceRow.sourceAuthorityKind,
    ref: sourceRow.sourceAuthorityRef,
  }),
  derivedFrom: Object.freeze([]),
  validity: "ACTIVE",
  supersedes: Object.freeze([]),
  accessPolicyRevision: sourceRow.accessPolicyRevision as ContextObject["accessPolicyRevision"],
});

class ControlledStore implements NativeStoreSession {
  readonly log: string[] = [];
  readonly decisionRecords: Array<Parameters<NativeStoreSession["commitDecision"]>[0]["record"]> =
    [];
  taskRevision: string = assemblySnapshot.taskRevision;
  revocationHead: string = assemblySnapshot.revocationHead;
  grantRevision: string = sourceRow.grant.revision;
  manifest: NativeContextManifestReceipt | undefined;
  beginCount = 0;
  afterDecisionSnapshot: (() => void) | undefined;
  afterReserve: (() => void) | undefined;
  afterBegin: (() => void) | undefined;

  async commitProject(): Promise<never> {
    throw new Error("unused");
  }
  async readSnapshot(): Promise<never> {
    throw new Error("unused");
  }
  async getReceipt(): Promise<never> {
    throw new Error("unused");
  }
  async commitContextVersion(): Promise<never> {
    throw new Error("unused");
  }
  async reserve(reservation: DurableActionReservation) {
    this.log.push("action:reserve");
    this.afterReserve?.();
    return {
      kind: "reserved" as const,
      reservationId: "reservation-one",
      operationId: reservation.operationId,
      semanticDigest: reservation.semanticDigest,
    };
  }
  async begin(reservationId: string, reservation: DurableActionReservation) {
    Assert.equal(reservationId, "reservation-one");
    this.log.push("action:begin");
    this.beginCount += 1;
    this.afterBegin?.();
    return {
      kind: "granted" as const,
      operationId: reservation.operationId,
      reservationId,
      sendAuthority: "send-authority-one",
    };
  }
  async recordDispatchOutcome(
    _reservationId: string,
    _operationId: string,
    _semanticDigest: string,
    outcome: Parameters<NativeStoreSession["recordDispatchOutcome"]>[3],
  ) {
    this.log.push(`action:outcome:${outcome.kind}`);
  }
  async publishDecisionSnapshot(
    input: Parameters<NativeStoreSession["publishDecisionSnapshot"]>[0],
  ) {
    this.log.push("decision:snapshot");
    Assert.equal(input.actionOperationId, ACTION_OPERATION);
    this.afterDecisionSnapshot?.();
  }
  async commitDecision(input: Parameters<NativeStoreSession["commitDecision"]>[0]) {
    if (input.record.taskRevision !== this.taskRevision) throw new Error("stale task revision");
    this.log.push("decision:commit");
    this.decisionRecords.push(input.record);
    return {
      kind: "committed" as const,
      operationId: input.record.operationId,
      decisionReceiptId: "decision-receipt-one",
    };
  }
  async readDecisionReplay(_domainId: string, operationId: string) {
    this.log.push("decision:replay");
    const record = this.decisionRecords.find((item) => item.operationId === operationId);
    if (record === undefined) throw new Error("missing decision");
    return {
      kind: "replayed" as const,
      operationId,
      decisionReceiptId: "decision-receipt-one",
      record,
    };
  }
  async publishContextAssemblySnapshot() {
    this.log.push("assembly:publish");
  }
  async commitTaskContextRequirements(): Promise<never> {
    throw new Error("unused");
  }
  async readTaskContextRequirements() {
    this.log.push("assembly:task");
    return Object.freeze({
      domainId: assemblySnapshot.domainId,
      taskId: assemblySnapshot.taskId,
      taskRevision: this.taskRevision,
      contentHash: digest("9"),
      mandatoryRefs: Object.freeze([required]),
    });
  }
  async readContextAssemblyBasis(): Promise<NativeContextAssemblyBasis> {
    this.log.push("assembly:basis");
    return Object.freeze({
      operationId: assemblySnapshot.operationId,
      bindingGeneration: assemblySnapshot.bindingGeneration,
      sourceEpoch: assemblySnapshot.sourceEpoch,
      taskRevision: assemblySnapshot.taskRevision,
      policyRevision: assemblySnapshot.policyRevision,
      authRevision: assemblySnapshot.authRevision,
      revocationHead: assemblySnapshot.revocationHead,
      maxContentBytes: assemblySnapshot.maxContentBytes,
      maxCandidates: assemblySnapshot.maxCandidates,
      partitionBindings: assemblySnapshot.partitionBindings,
      mandatoryRefs: Object.freeze([required]),
    });
  }
  async listContextAssemblySources() {
    this.log.push("assembly:sources");
    return [
      Object.freeze({
        ...sourceRow,
        grant: Object.freeze({
          ...sourceRow.grant,
          revision: this.grantRevision,
          revocationHead: this.revocationHead,
        }),
      }),
    ];
  }
  async readGranteeContextSet(): Promise<NativeAuthorizedContextReadSet> {
    this.log.push("assembly:read");
    return Object.freeze({
      principalId: assemblySnapshot.principalId,
      seatId: assemblySnapshot.seatId,
      policyRevision: assemblySnapshot.policyRevision,
      revocationHead: this.revocationHead,
      destinationDomainId: assemblySnapshot.domainId,
      destinationScope: "PROJECT",
      promotionKind: "PROJECT_ONLY",
      sources: Object.freeze([
        Object.freeze({
          grantRevision: this.grantRevision,
          revocationHead: this.revocationHead,
          state: "ACTIVE",
          stateRevision: sourceRow.stateRevision,
          sourceDomainId: sourceRow.sourceDomainId,
          contextId: sourceRow.contextId,
          version: sourceRow.version,
          scope: sourceRow.scope,
          kind: sourceRow.kind,
          contentHash: sourceRow.contentHash,
          sourceRef: sourceRow.sourceRef,
          sourceHash: sourceRow.sourceHash,
          sourceAuthorityKind: sourceRow.sourceAuthorityKind,
          sourceAuthorityRef: sourceRow.sourceAuthorityRef,
          accessPolicyRevision: sourceRow.accessPolicyRevision,
        }),
      ]),
    });
  }
  async commitContextManifest(input: Parameters<NativeStoreSession["commitContextManifest"]>[0]) {
    this.log.push("assembly:commit");
    const decoded = JSON.parse(input.canonicalManifest) as {
      manifestId: string;
      manifestHash: string;
    };
    this.manifest = Object.freeze({
      disposition: "COMMITTED" as const,
      operationId: input.operationId,
      manifestId: decoded.manifestId,
      manifestHash: decoded.manifestHash,
      canonicalManifest: input.canonicalManifest,
    });
    return this.manifest;
  }
  async readContextManifest() {
    this.log.push("assembly:replay");
    if (this.taskRevision !== assemblySnapshot.taskRevision) throw new Error("stale task revision");
    if (this.revocationHead !== assemblySnapshot.revocationHead) throw new Error("revoked");
    if (this.manifest === undefined) throw new Error("missing manifest");
    return Object.freeze({ ...this.manifest, disposition: "REPLAYED" as const });
  }
  async close() {}
}

class ControlledBindingAuthority implements ActionBindingAuthority {
  current: ActionBinding | null = binding;
  parentCurrent = true;
  revalidateCalls = 0;
  providerSends = 0;

  async currentBinding(bindingId: string) {
    return this.current?.bindingId === bindingId ? this.current : null;
  }
  async revalidateTaskPackage(value: AuthorizedTaskPackage, expected: ActionBinding) {
    this.revalidateCalls += 1;
    if (!this.parentCurrent) {
      throw new TypedActionError("STALE_BINDING", "parent grant changed");
    }
    Assert.equal(expected.generation, binding.generation);
    return actionCommitmentForPackage(value);
  }
  async dispatchIfCurrent(expected: ActionBinding, _action: unknown, sendAuthority: string) {
    if (JSON.stringify(this.current) !== JSON.stringify(expected)) {
      return { kind: "stale-before-send" as const };
    }
    Assert.equal(sendAuthority, "send-authority-one");
    this.providerSends += 1;
    return { kind: "accepted" as const, receiptRef: "fake-provider-receipt" };
  }
}

interface VerticalFixture {
  readonly store: ControlledStore;
  readonly authority: ControlledBindingAuthority;
  readonly service: ReturnType<typeof constructCognitionService>;
  readonly content: NativeBoundedContextSource & {
    afterSearch?: () => void;
    afterLoad?: () => void;
  };
  readonly vertical: ReturnType<ReturnType<typeof constructCognitionService>["createVertical"]>;
  readonly createSecondVertical: () => ReturnType<
    ReturnType<typeof constructCognitionService>["createVertical"]
  >;
}

function fixture(): VerticalFixture {
  const store = new ControlledStore();
  const authority = new ControlledBindingAuthority();
  const content: VerticalFixture["content"] = {
    async search() {
      content.afterSearch?.();
      return [required];
    },
    async load() {
      content.afterLoad?.();
      return [
        Object.freeze({
          object: contextObject,
          content: "ordinary fixture signal",
          state: "ACTIVE" as const,
          stateRevision: sourceRow.stateRevision,
          accessPolicyRevision: sourceRow.accessPolicyRevision,
        }),
      ];
    },
  };
  const construction: CognitionVerticalConstruction = {
    assemblyPlan: Object.freeze({
      snapshot: assemblySnapshot,
      recordedAt: "2026-09-21T12:00:00.000Z",
    }),
    contentSource: content,
    eligibility: {
      async resolveEligibility(request) {
        return Object.freeze({
          taskRevision: request.taskRevision,
          policyRevision: request.policyRevision,
          capabilityRevision: request.capabilityRevision,
          bindingGeneration: request.bindingGeneration,
          candidates: Object.freeze([
            Object.freeze({
              candidateId: "candidate-one",
              authorization: "ALLOWED" as const,
              capability: "QUALIFIED" as const,
              isolation: "QUALIFIED" as const,
              capacity: Object.freeze({ required: 1, available: 2 }),
              priorityClass: 0,
              waitingMs: 1,
              estimatedCost: 1,
              recipeRef: "recipe-one",
              resourceReservationRef: "capacity-one",
              actionIntentRef: ACTION_OPERATION,
            }),
          ]),
        });
      },
    },
    backend: {
      kind: "FAKE",
      async evaluate() {
        return Object.freeze({
          kind: "RANKED" as const,
          modelResolved: "fake-v1",
          ranks: Object.freeze([Object.freeze({ candidateId: "candidate-one", semanticRank: 1 })]),
        });
      },
    },
    nativeBasis: Object.freeze({
      operationId: "decision-operation",
      domainId: assemblySnapshot.domainId,
      decisionId: "decision-one",
      eventId: "decision-event",
      receiptId: "decision-receipt",
      recordedAt: "2026-09-21T12:00:01.000Z",
      candidates: Object.freeze([
        Object.freeze({
          candidateId: "candidate-one",
          requiredCapacityUnits: 1,
          snapshot: Object.freeze({
            operationId: "decision-operation",
            candidateId: "candidate-one",
            stateViewHash: digest("1"),
            candidateHash: digest("2"),
            taskRevision: assemblySnapshot.taskRevision,
            policyRevision: assemblySnapshot.policyRevision,
            capabilityRevision: "8",
            bindingId: binding.bindingId,
            bindingGeneration: binding.generation,
            authRevision: binding.authRevision,
            resourceRef: "pool-one",
            resourceRevision: "1",
            capacityTotal: 2,
            actionOperationId: ACTION_OPERATION,
            actionDigest: actionIntent.semanticDigest,
          }),
        }),
      ]),
    }),
  };
  const service = constructCognitionService(store, new TypedActionDispatcher(store, authority));
  const vertical = service.createVertical(construction);
  return {
    store,
    authority,
    service,
    content,
    vertical,
    createSecondVertical: () => service.createVertical(construction),
  };
}

const assemblyRequest = Object.freeze({
  principalId: assemblySnapshot.principalId,
  seatId: assemblySnapshot.seatId,
  taskId: assemblySnapshot.taskId,
  sessionId: assemblySnapshot.sessionId,
  domainId: assemblySnapshot.domainId,
  bindingId: assemblySnapshot.bindingId,
  bindingGeneration: assemblySnapshot.bindingGeneration,
  sourceEpoch: assemblySnapshot.sourceEpoch,
  runtimeInstanceId: assemblySnapshot.runtimeInstanceId,
  operationId: assemblySnapshot.operationId,
  manifestId: assemblySnapshot.manifestId,
  query: "ordinary fixture",
  maxContentBytes: assemblySnapshot.maxContentBytes,
});

const decisionRequest = Object.freeze({
  operationId: "decision-operation",
  scenarioId: "DF02",
  stateViewHash: digest("1"),
  candidateHash: digest("2"),
  questionVersion: "1",
  rubricVersion: "1",
  modelRequested: null,
  taskRevision: assemblySnapshot.taskRevision,
  policyRevision: assemblySnapshot.policyRevision,
  capabilityRevision: "8",
  bindingGeneration: binding.generation,
  budgetUnits: 1,
  deadlineEpochMs: 1000,
  candidateRefs: Object.freeze(["candidate-one"]),
});

describe("controlled cognition vertical", () => {
  it("binds Manifest, Decision and Action on one native session", async () => {
    const { store, authority, vertical } = fixture();
    const manifest = await vertical.assembleContext(assemblyRequest);
    Assert.equal(manifest.requiredConstraints.length, 1);

    const decision = await vertical.decideForContext({ manifest, request: decisionRequest });
    Assert.equal(decision.kind, "COMMITTED");
    const source = manifest.sourceSnapshot as Readonly<Record<string, unknown>>;
    const manifestScope = Object.freeze({
      schema: "gogoke.manifest-bound-decision.v1",
      manifestId: manifest.manifestId,
      manifestHash: manifest.manifestHash,
      taskId: manifest.taskId,
      seatId: manifest.seatId,
      domainId: manifest.domainId,
      sessionId: source.sessionId,
      bindingId: source.bindingId,
      bindingGeneration: manifest.bindingGeneration,
      authRevision: source.authRevision,
      runtimeInstanceId: source.runtimeInstanceId,
      taskRevision: source.taskRevision,
      policyRevision: manifest.policyRevision,
    });
    Assert.equal(store.decisionRecords.length, 1);
    Assert.equal(
      store.decisionRecords[0]!.stateViewHash,
      hashData(
        Object.freeze({ ...manifestScope, sourceStateViewHash: decisionRequest.stateViewHash }),
      ),
    );
    Assert.equal(
      store.decisionRecords[0]!.candidateHash,
      hashData(
        Object.freeze({ ...manifestScope, sourceCandidateHash: decisionRequest.candidateHash }),
      ),
    );
    const dispatch = await vertical.dispatchForDecision({
      manifest,
      decision,
      intent: actionIntent,
    });
    Assert.deepEqual(dispatch.result, {
      status: "dispatched",
      receiptRef: "fake-provider-receipt",
    });
    Assert.equal(dispatch.manifest.manifestHash, manifest.manifestHash);
    Assert.equal(dispatch.decision.receiptId, "decision-receipt-one");
    Assert.equal(dispatch.action.operationId, ACTION_OPERATION);
    Assert.equal(dispatch.action.semanticDigest, actionIntent.semanticDigest);
    Assert.equal(authority.providerSends, 1);
    Assert.equal(store.beginCount, 1);
    Assert.ok(store.log.indexOf("assembly:commit") < store.log.indexOf("decision:commit"));
    Assert.ok(store.log.indexOf("decision:commit") < store.log.indexOf("action:begin"));
  });

  it("fails closed when Task revision changes before manifest commit", async () => {
    const current = fixture();
    current.content.afterLoad = () => {
      current.store.taskRevision = "2";
    };
    await Assert.rejects(current.vertical.assembleContext(assemblyRequest));
    Assert.equal(current.authority.providerSends, 0);
  });

  it("fails closed when revocation advances between search and load", async () => {
    const current = fixture();
    current.content.afterSearch = () => {
      current.store.revocationHead = "5";
    };
    await Assert.rejects(current.vertical.assembleContext(assemblyRequest));
    Assert.equal(current.authority.providerSends, 0);
  });

  it("fails closed when current Task changes after manifest commit but before disclosure", async () => {
    const current = fixture();
    const commit = current.store.commitContextManifest.bind(current.store);
    current.store.commitContextManifest = async (input) => {
      const receipt = await commit(input);
      current.store.taskRevision = "2";
      return receipt;
    };
    await Assert.rejects(current.vertical.assembleContext(assemblyRequest));
    Assert.equal(current.authority.providerSends, 0);
  });

  it("fails closed when Task authority changes between Decision snapshot and commit", async () => {
    const current = fixture();
    const manifest = await current.vertical.assembleContext(assemblyRequest);
    current.store.afterDecisionSnapshot = () => {
      current.store.taskRevision = "2";
    };
    await Assert.rejects(
      current.vertical.decideForContext({ manifest, request: decisionRequest }),
      (error: unknown) => error instanceof DecisionEngineError && error.code === "COMMIT_UNKNOWN",
    );
    Assert.equal(current.authority.providerSends, 0);
  });

  it("records not-sent when binding generation changes after durable reserve", async () => {
    const current = fixture();
    const manifest = await current.vertical.assembleContext(assemblyRequest);
    const decision = await current.vertical.decideForContext({
      manifest,
      request: decisionRequest,
    });
    current.store.afterReserve = () => {
      current.authority.current = Object.freeze({ ...binding, generation: "8" });
    };
    await Assert.rejects(
      current.vertical.dispatchForDecision({ manifest, decision, intent: actionIntent }),
    );
    Assert.equal(current.authority.providerSends, 0);
    Assert.ok(current.store.log.includes("action:outcome:not-sent"));
  });

  it("records not-sent when the parent grant changes after durable reserve", async () => {
    const current = fixture();
    const manifest = await current.vertical.assembleContext(assemblyRequest);
    const decision = await current.vertical.decideForContext({
      manifest,
      request: decisionRequest,
    });
    current.store.afterReserve = () => {
      current.authority.parentCurrent = false;
    };
    await Assert.rejects(
      current.vertical.dispatchForDecision({ manifest, decision, intent: actionIntent }),
    );
    Assert.equal(current.authority.providerSends, 0);
    Assert.ok(current.store.log.includes("action:outcome:not-sent"));
  });

  it("uses the final authority fence and never sends after a post-begin binding change", async () => {
    const current = fixture();
    const manifest = await current.vertical.assembleContext(assemblyRequest);
    const decision = await current.vertical.decideForContext({
      manifest,
      request: decisionRequest,
    });
    current.store.afterBegin = () => {
      current.authority.current = Object.freeze({ ...binding, generation: "8" });
    };
    await Assert.rejects(
      current.vertical.dispatchForDecision({ manifest, decision, intent: actionIntent }),
    );
    Assert.equal(current.authority.providerSends, 0);
    Assert.equal(current.store.beginCount, 1);
    Assert.ok(current.store.log.includes("action:outcome:not-sent"));
  });

  it("rejects a Manifest from another session before writing a Decision", async () => {
    const current = fixture();
    const other = fixture();
    const foreignManifest = await other.vertical.assembleContext(assemblyRequest);
    await Assert.rejects(
      current.vertical.decideForContext({ manifest: foreignManifest, request: decisionRequest }),
      (error: unknown) =>
        error instanceof CognitionLineageError && error.code === "INVALID_MANIFEST_LINEAGE",
    );
    Assert.equal(current.store.log.includes("decision:commit"), false);
  });

  it("rebuilds dispatch lineage from durable identities after object cache loss", async () => {
    const current = fixture();
    const manifest = await current.vertical.assembleContext(assemblyRequest);
    const decision = await current.vertical.decideForContext({ manifest, request: decisionRequest });
    const restoredManifest = structuredClone(manifest) as ContextManifest;
    const restoredDecision = structuredClone(decision) as typeof decision;
    await current.vertical.dispatchForDecision({
      manifest: restoredManifest,
      decision: restoredDecision,
      intent: actionIntent,
    });
    Assert.equal(current.authority.providerSends, 1);
    Assert.ok(current.store.log.includes("assembly:replay"));
    Assert.ok(current.store.log.includes("decision:replay"));
  });

  it("rejects a decision paired with a different Action session", async () => {
    const current = fixture();
    const manifest = await current.vertical.assembleContext(assemblyRequest);
    const decision = await current.vertical.decideForContext({
      manifest,
      request: decisionRequest,
    });
    const otherSession = Object.freeze({ ...binding, sessionId: "session-other" });
    const forgedIntent = Object.freeze({ ...actionIntent, binding: otherSession });
    await Assert.rejects(
      current.vertical.dispatchForDecision({ manifest, decision, intent: forgedIntent }),
      (error: unknown) =>
        error instanceof CognitionLineageError && error.code === "INVALID_ACTION_LINEAGE",
    );
    Assert.equal(current.authority.providerSends, 0);
  });

  it("rejects a second vertical instead of creating a second dispatcher queue", () => {
    const current = fixture();
    Assert.throws(
      () => current.createSecondVertical(),
      (error: unknown) =>
        error instanceof CognitionLineageError && error.code === "VERTICAL_ALREADY_CREATED",
    );
  });

  it("rejects the assumption that caller-made Outcome or Evaluation refs are accepted", () => {
    const { vertical } = fixture();
    const exposed = vertical as unknown as Readonly<Record<string, unknown>>;
    for (const absentAuthority of [
      "appendObjectiveOutcome",
      "readOutcomeSnapshot",
      "evaluateOutcomes",
      "createDreamCandidateFromEvaluation",
    ]) {
      Assert.equal(
        absentAuthority in exposed,
        false,
        `${absentAuthority} must await the native authority seam`,
      );
    }
  });
});
