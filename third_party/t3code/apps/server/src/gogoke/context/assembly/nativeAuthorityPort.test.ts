import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";

import type { ContextManifest, ContextObject } from "../../contracts/model.ts";
import type { NativeStoreSession } from "../../bootstrap/nativeStoreService.ts";
import type {
  NativeAuthorizedContextReadSet,
  NativeAuthorizedContextReadSource,
  NativeContextAssemblyBasis,
  NativeContextAssemblySource,
  NativeContextManifestReceipt,
} from "../../persistence/base/nativeHostClient.ts";
import { NativeHostClientError } from "../../persistence/base/nativeHostClient.ts";
import {
  ContextAssemblyError,
  type AssemblyBasis,
  type AssemblyRequest,
  type ContextVersionRef,
} from "./model.ts";
import {
  NativeContextAssemblyAuthorityPort,
  type NativeBoundedContextSource,
  type NativeContextAssemblyPlan,
} from "./nativeAuthorityPort.ts";
import { canonical, hashData, hashText } from "./passive.ts";
import type { NativeContextAssemblySnapshot as WireSnapshot } from "../../persistence/base/nativeHostClient.ts";

const textHash = hashText("payload");
const sourceHash = `sha256:${"b".repeat(64)}`;

const snapshot: WireSnapshot = Object.freeze({
  operationId: "assembly-operation",
  principalId: "principal-one",
  seatId: "seat-one",
  taskId: "task-one",
  sessionId: "session-one",
  domainId: "domain-destination",
  bindingId: "binding-one",
  bindingGeneration: "7",
  sourceEpoch: "9",
  runtimeInstanceId: "runtime-one",
  taskRevision: "1",
  policyRevision: "2",
  authRevision: "3",
  revocationHead: "4",
  selectionDecisionId: "decision-one",
  manifestId: "manifest-one",
  admissionActionOperationId: "admission-one",
  admissionDigest: `sha256:${"c".repeat(64)}`,
  maxContentBytes: 100,
  maxCandidates: 2,
  partitionBindings: Object.freeze([
    Object.freeze({
      sourceDomainId: "domain-source",
      destinationScope: "PROJECT",
      promotionKind: "PROJECT_ONLY",
      grant: Object.freeze({ grantId: "grant-one", revision: "5", revocationHead: "4" }),
    }),
  ]),
});

const required: ContextVersionRef = Object.freeze({
  sourceDomainId: "domain-source",
  contextId: "context-one",
  version: "1",
});

const plan: NativeContextAssemblyPlan = Object.freeze({
  snapshot,
  recordedAt: "2026-09-21T12:00:00.000Z",
});

const row: NativeContextAssemblySource = Object.freeze({
  sourceDomainId: "domain-source",
  contextId: "context-one",
  version: "1",
  scope: "PROJECT",
  kind: "fact",
  contentHash: textHash,
  sourceRef: "source://one",
  sourceHash,
  sourceAuthorityKind: "repository",
  sourceAuthorityRef: "authority://one",
  accessPolicyRevision: "6",
  stateRevision: "7",
  grant: Object.freeze({ grantId: "grant-one", revision: "5", revocationHead: "4" }),
});

const excludedRow: NativeContextAssemblySource = Object.freeze({
  ...row,
  contextId: "context-two",
  contentHash: `sha256:${"e".repeat(64)}`,
  sourceRef: "source://two",
  sourceHash: `sha256:${"f".repeat(64)}`,
});

const nativeBasis: NativeContextAssemblyBasis = Object.freeze({
  operationId: snapshot.operationId,
  bindingGeneration: snapshot.bindingGeneration,
  sourceEpoch: snapshot.sourceEpoch,
  taskRevision: snapshot.taskRevision,
  policyRevision: snapshot.policyRevision,
  authRevision: snapshot.authRevision,
  revocationHead: snapshot.revocationHead,
  maxContentBytes: snapshot.maxContentBytes,
  maxCandidates: snapshot.maxCandidates,
  partitionBindings: snapshot.partitionBindings,
  mandatoryRefs: Object.freeze([required]),
});

const currentTask = Object.freeze({
  domainId: snapshot.domainId,
  taskId: snapshot.taskId,
  taskRevision: snapshot.taskRevision,
  contentHash: `sha256:${"9".repeat(64)}`,
  mandatoryRefs: Object.freeze([required]),
});

const nativeRead: NativeAuthorizedContextReadSource = Object.freeze({
  grantRevision: row.grant.revision,
  revocationHead: row.grant.revocationHead,
  state: "ACTIVE",
  stateRevision: row.stateRevision,
  sourceDomainId: row.sourceDomainId,
  contextId: row.contextId,
  version: row.version,
  scope: row.scope,
  kind: row.kind,
  contentHash: row.contentHash,
  sourceRef: row.sourceRef,
  sourceHash: row.sourceHash,
  sourceAuthorityKind: row.sourceAuthorityKind,
  sourceAuthorityRef: row.sourceAuthorityRef,
  accessPolicyRevision: row.accessPolicyRevision,
});

const nativeReadSet: NativeAuthorizedContextReadSet = Object.freeze({
  principalId: snapshot.principalId,
  seatId: snapshot.seatId,
  policyRevision: snapshot.policyRevision,
  revocationHead: snapshot.revocationHead,
  destinationDomainId: snapshot.domainId,
  destinationScope: "PROJECT",
  promotionKind: "PROJECT_ONLY",
  sources: Object.freeze([nativeRead]),
});

const object: ContextObject = Object.freeze({
  contextId: row.contextId,
  version: row.version as ContextObject["version"],
  scope: "PROJECT",
  domainId: row.sourceDomainId,
  kind: row.kind,
  contentHash: row.contentHash,
  sourceRef: Object.freeze({ ref: row.sourceRef, hash: row.sourceHash }),
  sourceAuthority: Object.freeze({ kind: row.sourceAuthorityKind, ref: row.sourceAuthorityRef }),
  derivedFrom: Object.freeze([]),
  validity: "ACTIVE",
  supersedes: Object.freeze([]),
  accessPolicyRevision: row.accessPolicyRevision as ContextObject["accessPolicyRevision"],
});

const resolved = Object.freeze({
  object,
  content: "payload",
  state: "ACTIVE" as const,
  stateRevision: row.stateRevision,
  accessPolicyRevision: row.accessPolicyRevision,
});

const request: AssemblyRequest = Object.freeze({
  principalId: snapshot.principalId,
  seatId: snapshot.seatId,
  taskId: snapshot.taskId,
  sessionId: snapshot.sessionId,
  domainId: snapshot.domainId,
  bindingId: snapshot.bindingId,
  bindingGeneration: snapshot.bindingGeneration,
  sourceEpoch: snapshot.sourceEpoch,
  runtimeInstanceId: snapshot.runtimeInstanceId,
  operationId: snapshot.operationId,
  manifestId: snapshot.manifestId,
  query: "payload",
  maxContentBytes: snapshot.maxContentBytes,
});

const replayRequest = () => ({
  principalId: snapshot.principalId,
  seatId: snapshot.seatId,
  taskId: snapshot.taskId,
  sessionId: snapshot.sessionId,
  domainId: snapshot.domainId,
  bindingId: snapshot.bindingId,
  bindingGeneration: snapshot.bindingGeneration,
  sourceEpoch: snapshot.sourceEpoch,
  runtimeInstanceId: snapshot.runtimeInstanceId,
  operationId: snapshot.operationId,
});

function makeBasis(): AssemblyBasis {
  return {
    principalId: snapshot.principalId,
    seatId: snapshot.seatId,
    taskId: snapshot.taskId,
    sessionId: snapshot.sessionId,
    domainId: snapshot.domainId,
    bindingId: snapshot.bindingId,
    bindingGeneration: snapshot.bindingGeneration,
    sourceEpoch: snapshot.sourceEpoch,
    runtimeInstanceId: snapshot.runtimeInstanceId,
    readRef: snapshot.operationId,
    admissionRef: snapshot.admissionActionOperationId,
    taskRevision: snapshot.taskRevision,
    policyRevision: snapshot.policyRevision,
    authRevision: snapshot.authRevision,
    revocationHead: snapshot.revocationHead,
    selectionDecisionId: snapshot.selectionDecisionId,
    maxContentBytes: snapshot.maxContentBytes,
    maxCandidates: snapshot.maxCandidates,
    partitions: [
      {
        sourceDomainId: row.sourceDomainId,
        authorizationRef: row.grant.grantId,
        authorizationRevision: row.grant.revision,
      },
    ],
    requiredConstraints: [required],
  };
}

function replayManifest(excluded = false): {
  readonly receipt: NativeContextManifestReceipt;
  readonly manifest: ContextManifest;
} {
  const body = {
    manifestId: snapshot.manifestId,
    taskId: snapshot.taskId,
    seatId: snapshot.seatId,
    bindingGeneration: snapshot.bindingGeneration,
    domainId: snapshot.domainId,
    policyRevision: snapshot.policyRevision,
    sourceSnapshot: {
      assemblySchema: "gogoke.context-assembly.v1",
      operationId: snapshot.operationId,
      requestDigest: `sha256:${"d".repeat(64)}`,
      principalId: snapshot.principalId,
      sessionId: snapshot.sessionId,
      bindingId: snapshot.bindingId,
      sourceEpoch: snapshot.sourceEpoch,
      runtimeInstanceId: snapshot.runtimeInstanceId,
      taskRevision: snapshot.taskRevision,
      authRevision: snapshot.authRevision,
      revocationHead: snapshot.revocationHead,
      partitions: [{ sourceDomainId: row.sourceDomainId }],
      mode: "FIXED_SOURCE_RULES",
      excluded: excluded
        ? [
            {
              sourceDomainId: excludedRow.sourceDomainId,
              contextId: excludedRow.contextId,
              version: excludedRow.version,
              reason: "NEEDS_BUDGET",
            },
          ]
        : [],
    },
    requiredConstraints: [
      {
        sourceDomainId: required.sourceDomainId,
        contextId: required.contextId,
        version: required.version,
        contentHash: row.contentHash,
        stateRevision: row.stateRevision,
        accessPolicyRevision: row.accessPolicyRevision,
      },
    ],
    includedVersions: [
      {
        sourceDomainId: required.sourceDomainId,
        contextId: required.contextId,
        version: required.version,
        contentHash: row.contentHash,
        stateRevision: row.stateRevision,
        accessPolicyRevision: row.accessPolicyRevision,
        reason: "MANDATORY_CONSTRAINT",
      },
    ],
    redactions: [],
    selectionDecisionId: snapshot.selectionDecisionId,
  };
  const manifest = Object.freeze({
    ...body,
    manifestHash: hashData(body),
  }) as unknown as ContextManifest;
  return {
    manifest,
    receipt: Object.freeze({
      disposition: "REPLAYED",
      operationId: snapshot.operationId,
      manifestId: manifest.manifestId,
      manifestHash: manifest.manifestHash,
      canonicalManifest: canonical(manifest),
    }),
  };
}

function makeStore(overrides: Partial<NativeStoreSession> = {}): NativeStoreSession {
  return {
    publishContextAssemblySnapshot: async () => undefined,
    commitTaskContextRequirements: async () => {
      throw new Error("unused");
    },
    readTaskContextRequirements: async () => currentTask,
    readContextAssemblyBasis: async () => nativeBasis,
    listContextAssemblySources: async () => [row],
    readGranteeContextSet: async () => nativeReadSet,
    commitContextManifest: async () => ({
      disposition: "COMMITTED",
      operationId: snapshot.operationId,
      manifestId: snapshot.manifestId,
      manifestHash: replayManifest().manifest.manifestHash,
      canonicalManifest: canonical(replayManifest().manifest),
    }),
    readContextManifest: async () => replayManifest().receipt,
    ...overrides,
  } as NativeStoreSession;
}

function makeContent(
  overrides: Partial<NativeBoundedContextSource> = {},
): NativeBoundedContextSource {
  return {
    search: async () => [required],
    load: async () => [resolved],
    ...overrides,
  };
}

async function openedPort(
  store: NativeStoreSession = makeStore(),
  content: NativeBoundedContextSource = makeContent(),
): Promise<{ readonly port: NativeContextAssemblyAuthorityPort; readonly basis: AssemblyBasis }> {
  const port = new NativeContextAssemblyAuthorityPort({ store, plan, content });
  const basis = await port.openAssembly(request);
  Assert.ok(basis);
  return { port, basis };
}

describe("native ContextAssembly authority port", () => {
  it("rejects a plan that attempts to inject mandatory refs", () => {
    Assert.throws(
      () =>
        new NativeContextAssemblyAuthorityPort({
          store: makeStore(),
          plan: { ...plan, requiredConstraints: [] } as unknown as NativeContextAssemblyPlan,
          content: makeContent(),
        }),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "AUTHORITY_PROTOCOL_ERROR",
    );
  });

  it("takes required constraints from the native basis and supports an empty Task set", async () => {
    const first = await openedPort();
    Assert.deepEqual(first.basis.requiredConstraints, [required]);
    const emptyBasis = Object.freeze({ ...nativeBasis, mandatoryRefs: Object.freeze([]) });
    const emptyStore = makeStore({
      readContextAssemblyBasis: async () => emptyBasis,
      readTaskContextRequirements: async () =>
        Object.freeze({ ...currentTask, mandatoryRefs: Object.freeze([]) }),
      listContextAssemblySources: async () => [],
    });
    const empty = await openedPort(emptyStore, makeContent({ search: async () => [] }));
    Assert.deepEqual(empty.basis.requiredConstraints, []);
    Assert.deepEqual(await empty.port.searchVisibleContext(empty.basis, "query"), []);
  });

  it("rejects over-cap mandatory native basis before issuing its process-local identity", async () => {
    const secondRequired = Object.freeze({ ...required, contextId: "context-two" });
    const lowSnapshot = Object.freeze({ ...snapshot, maxCandidates: 1 });
    const overCapBasis = Object.freeze({
      ...nativeBasis,
      maxCandidates: 1,
      mandatoryRefs: Object.freeze([required, secondRequired]),
    });
    let publishCalls = 0;
    let basisCalls = 0;
    let taskCalls = 0;
    let listCalls = 0;
    const port = new NativeContextAssemblyAuthorityPort({
      store: makeStore({
        publishContextAssemblySnapshot: async () => {
          publishCalls += 1;
        },
        readContextAssemblyBasis: async () => {
          basisCalls += 1;
          return overCapBasis;
        },
        readTaskContextRequirements: async () => {
          taskCalls += 1;
          return Object.freeze({
            ...currentTask,
            mandatoryRefs: overCapBasis.mandatoryRefs,
          });
        },
        listContextAssemblySources: async () => {
          listCalls += 1;
          return [row];
        },
      }),
      plan: Object.freeze({ ...plan, snapshot: lowSnapshot }),
      content: makeContent(),
    });
    await Assert.rejects(
      port.openAssembly(Object.freeze({ ...request, maxContentBytes: lowSnapshot.maxContentBytes })),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "AUTHORITY_PROTOCOL_ERROR",
    );
    Assert.equal(publishCalls, 1);
    Assert.equal(basisCalls, 1);

    const forged = Object.freeze({
      ...makeBasis(),
      maxCandidates: 1,
      partitions: Object.freeze(makeBasis().partitions.map((partition) => Object.freeze(partition))),
      requiredConstraints: overCapBasis.mandatoryRefs,
    });
    await Assert.rejects(
      port.searchVisibleContext(forged, "query"),
      (error: unknown) => error instanceof ContextAssemblyError && error.code === "ACCESS_DENIED",
    );
    Assert.equal(taskCalls, 0);
    Assert.equal(listCalls, 0);
  });

  it("rejects non-positive, over-bound and unsafe plan candidate counts", () => {
    for (const maxCandidates of [0, 65, Number.NaN, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
      Assert.throws(
        () =>
          new NativeContextAssemblyAuthorityPort({
            store: makeStore(),
            plan: Object.freeze({
              ...plan,
              snapshot: Object.freeze({ ...snapshot, maxCandidates }),
            }),
            content: makeContent(),
          }),
        (error: unknown) =>
          error instanceof ContextAssemblyError && error.code === "AUTHORITY_PROTOCOL_ERROR",
        String(maxCandidates),
      );
    }
  });

  it("fails closed when a mandatory source is absent from the fresh native relation", async () => {
    let contentCalls = 0;
    const { port, basis } = await openedPort(
      makeStore({ listContextAssemblySources: async () => [] }),
      makeContent({
        search: async () => {
          contentCalls += 1;
          return [];
        },
      }),
    );
    await Assert.rejects(
      port.searchVisibleContext(basis, "query"),
      (error: unknown) => error instanceof ContextAssemblyError && error.code === "ACCESS_DENIED",
    );
    Assert.equal(contentCalls, 0);
  });

  it("revalidates current Task before search, load, commit and replay", async () => {
    const changedTask = Object.freeze({ ...currentTask, taskRevision: "2" });
    for (const operation of ["search", "load", "commit"] as const) {
      let downstreamCalls = 0;
      const { port, basis } = await openedPort(
        makeStore({
          readTaskContextRequirements: async () => changedTask,
          listContextAssemblySources: async () => {
            downstreamCalls += 1;
            return [row];
          },
          commitContextManifest: async () => {
            downstreamCalls += 1;
            return replayManifest().receipt;
          },
        }),
      );
      const action =
        operation === "search"
          ? port.searchVisibleContext(basis, "query")
          : operation === "load"
            ? port.loadVisibleVersions(basis, [required])
            : port.commitManifest({
                operationId: snapshot.operationId,
                requestDigest: `sha256:${"d".repeat(64)}`,
                basis,
                expectedVersions: [
                  {
                    ...required,
                    contentHash: row.contentHash,
                    stateRevision: row.stateRevision,
                    accessPolicyRevision: row.accessPolicyRevision,
                  },
                ],
                manifest: replayManifest().manifest,
              });
      await Assert.rejects(
        action,
        (error: unknown) =>
          error instanceof ContextAssemblyError && error.code === "STALE_ASSEMBLY",
      );
      Assert.equal(downstreamCalls, 0, operation);
    }

    let replayCalls = 0;
    const replayPort = new NativeContextAssemblyAuthorityPort({
      store: makeStore({
        readTaskContextRequirements: async () => changedTask,
        readContextManifest: async () => {
          replayCalls += 1;
          return replayManifest().receipt;
        },
      }),
      plan,
      content: makeContent(),
    });
    await Assert.rejects(
      replayPort.readCurrentManifest(replayRequest()),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "STALE_ASSEMBLY",
    );
    Assert.equal(replayCalls, 0);
  });

  it("treats same-revision mandatory-ref drift as stale Task authority", async () => {
    let listCalls = 0;
    const changedRefs = Object.freeze([
      Object.freeze({ ...required, contextId: "different-mandatory" }),
    ]);
    const { port, basis } = await openedPort(
      makeStore({
        readTaskContextRequirements: async () =>
          Object.freeze({ ...currentTask, mandatoryRefs: changedRefs }),
        listContextAssemblySources: async () => {
          listCalls += 1;
          return [row];
        },
      }),
    );
    await Assert.rejects(
      port.searchVisibleContext(basis, "query"),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "STALE_ASSEMBLY",
    );
    Assert.equal(listCalls, 0);
  });
  it("denies an open before any content-source call", async () => {
    let calls = 0;
    const port = new NativeContextAssemblyAuthorityPort({
      store: makeStore(),
      plan,
      content: makeContent({
        search: async () => {
          calls += 1;
          return [];
        },
      }),
    });
    const deniedRequest = { ...request, domainId: "other-domain" };
    Assert.equal(await port.openAssembly(deniedRequest), null);
    Assert.equal(calls, 0);
  });

  it("passes only native-authorized rows to search and rejects unsolicited refs", async () => {
    const seen: NativeContextAssemblySource[][] = [];
    const { port, basis } = await openedPort(
      makeStore(),
      makeContent({
        search: async (input) => {
          seen.push([...input.authorizedSources]);
          return [{ ...required, contextId: "not-authorized" }];
        },
      }),
    );
    await Assert.rejects(
      port.searchVisibleContext(basis, "query"),
      (error: unknown) => error instanceof ContextAssemblyError && error.code === "ACCESS_DENIED",
    );
    Assert.deepEqual(
      seen.map((rows) => rows.map((value) => value.contextId)),
      [[row.contextId]],
    );
  });

  it("blocks load when the fresh native list is unavailable and does not call content", async () => {
    let contentCalls = 0;
    const { port, basis } = await openedPort(
      makeStore({
        listContextAssemblySources: async () => {
          throw new Error("revoked");
        },
      }),
      makeContent({
        load: async () => {
          contentCalls += 1;
          return [];
        },
      }),
    );
    await Assert.rejects(
      port.loadVisibleVersions(basis, [required]),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "AUTHORITY_UNAVAILABLE",
    );
    Assert.equal(contentCalls, 0);
  });

  it("requires fresh grant provenance for the native read and content load", async () => {
    let seenGrant: string | undefined;
    let seenReadRevision: string | undefined;
    const { port, basis } = await openedPort(
      makeStore(),
      makeContent({
        load: async (input) => {
          seenGrant = input.authorizedSources[0]?.grant.grantId;
          seenReadRevision = input.authorizedReads[0]?.grantRevision;
          return [resolved];
        },
      }),
    );
    const loaded = await port.loadVisibleVersions(basis, [required]);
    Assert.equal(loaded[0]?.object.contextId, required.contextId);
    Assert.equal(seenGrant, row.grant.grantId);
    Assert.equal(seenReadRevision, row.grant.revision);
  });

  it("rejects a content hash mismatch after native authorization", async () => {
    const { port, basis } = await openedPort(
      makeStore(),
      makeContent({ load: async () => [{ ...resolved, content: "tampered" }] }),
    );
    await Assert.rejects(
      port.loadVisibleVersions(basis, [required]),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "AUTHORITY_PROTOCOL_ERROR",
    );
  });

  it("maps typed commit and replay results without a local cache", async () => {
    const replay = replayManifest();
    const commitInputs: Array<{
      readonly readRequests: ReadonlyArray<unknown>;
      readonly recordedAt: string;
    }> = [];
    const { port, basis } = await openedPort(
      makeStore({
        commitContextManifest: async (input) => {
          commitInputs.push(input);
          return Object.freeze({
            disposition: "COMMITTED",
            operationId: snapshot.operationId,
            manifestId: replay.manifest.manifestId,
            manifestHash: replay.manifest.manifestHash,
            canonicalManifest: replay.receipt.canonicalManifest,
          });
        },
        readContextManifest: async () => replay.receipt,
      }),
    );
    const commitRequest = {
      operationId: snapshot.operationId,
      requestDigest: `sha256:${"d".repeat(64)}`,
      basis,
      expectedVersions: [
        {
          sourceDomainId: row.sourceDomainId,
          contextId: row.contextId,
          version: row.version,
          contentHash: row.contentHash,
          stateRevision: row.stateRevision,
          accessPolicyRevision: row.accessPolicyRevision,
        },
      ],
      manifest: replay.manifest,
    };
    const result = await port.commitManifest(commitRequest);
    Assert.equal(result.kind, "committed");
    Assert.equal(commitInputs[0]!.readRequests.length, 1);
    const retry = await port.commitManifest(commitRequest);
    Assert.equal(retry.kind, "committed");
    Assert.deepEqual(
      commitInputs.map((input) => input.recordedAt),
      [plan.recordedAt, plan.recordedAt],
    );
    const manifest = await port.readCurrentManifest({
      principalId: snapshot.principalId,
      seatId: snapshot.seatId,
      taskId: snapshot.taskId,
      sessionId: snapshot.sessionId,
      domainId: snapshot.domainId,
      bindingId: snapshot.bindingId,
      bindingGeneration: snapshot.bindingGeneration,
      sourceEpoch: snapshot.sourceEpoch,
      runtimeInstanceId: snapshot.runtimeInstanceId,
      operationId: snapshot.operationId,
    });
    Assert.equal(manifest?.manifestHash, replay.manifest.manifestHash);
  });

  it("rejects same-ref required metadata tampering against the fresh native row", async () => {
    let commitCalls = 0;
    const { port, basis } = await openedPort(
      makeStore({
        listContextAssemblySources: async () => [row],
        commitContextManifest: async () => {
          commitCalls += 1;
          return replayManifest().receipt;
        },
      }),
    );
    const original = replayManifest().manifest;
    const requiredVersion = {
      sourceDomainId: required.sourceDomainId,
      contextId: required.contextId,
      version: required.version,
      contentHash: `sha256:${"a".repeat(64)}`,
      stateRevision: "8",
      accessPolicyRevision: row.accessPolicyRevision,
    };
    const includedVersion = { ...requiredVersion, reason: "MANDATORY_CONSTRAINT" };
    const { manifestHash: ignored, ...withoutHash } = {
      ...original,
      requiredConstraints: [requiredVersion],
      includedVersions: [includedVersion],
    };
    const tamperedManifest = Object.freeze({
      ...withoutHash,
      manifestHash: hashData(withoutHash),
    }) as unknown as ContextManifest;
    const result = await port.commitManifest({
      operationId: snapshot.operationId,
      requestDigest: `sha256:${"d".repeat(64)}`,
      basis,
      expectedVersions: [requiredVersion],
      manifest: tamperedManifest,
    });
    Assert.equal(result.kind, "stale");
    Assert.equal(commitCalls, 0);
  });

  it("requires mandatory included metadata to match the required record", async () => {
    let listCalls = 0;
    const { port, basis } = await openedPort(
      makeStore({
        listContextAssemblySources: async () => {
          listCalls += 1;
          return [row];
        },
      }),
    );
    const original = replayManifest().manifest;
    const wrongReason = {
      ...(original.includedVersions[0] as Record<string, unknown>),
      reason: "AUTHORIZED_RETRIEVAL",
    };
    const { manifestHash: ignored, ...withoutHash } = {
      ...original,
      includedVersions: [wrongReason],
    };
    const malformed = Object.freeze({
      ...withoutHash,
      manifestHash: hashData(withoutHash),
    }) as unknown as ContextManifest;
    await Assert.rejects(
      port.commitManifest({
        operationId: snapshot.operationId,
        requestDigest: `sha256:${"d".repeat(64)}`,
        basis,
        expectedVersions: [
          {
            sourceDomainId: row.sourceDomainId,
            contextId: row.contextId,
            version: row.version,
            contentHash: row.contentHash,
            stateRevision: row.stateRevision,
            accessPolicyRevision: row.accessPolicyRevision,
          },
        ],
        manifest: malformed,
      }),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "AUTHORITY_PROTOCOL_ERROR",
    );
    Assert.equal(listCalls, 0);
  });

  it("includes excluded manifest refs in native reads while keeping expected versions included-only", async () => {
    const replay = replayManifest(true);
    let commitInput:
      | {
          readonly readRequests: ReadonlyArray<unknown>;
          readonly expectedVersions: ReadonlyArray<unknown>;
        }
      | undefined;
    const { port, basis } = await openedPort(
      makeStore({
        listContextAssemblySources: async () => [row, excludedRow],
        commitContextManifest: async (input) => {
          commitInput = input;
          return Object.freeze({
            disposition: "COMMITTED",
            operationId: snapshot.operationId,
            manifestId: replay.manifest.manifestId,
            manifestHash: replay.manifest.manifestHash,
            canonicalManifest: replay.receipt.canonicalManifest,
          });
        },
      }),
    );
    const result = await port.commitManifest({
      operationId: snapshot.operationId,
      requestDigest: `sha256:${"d".repeat(64)}`,
      basis,
      expectedVersions: [
        {
          sourceDomainId: row.sourceDomainId,
          contextId: row.contextId,
          version: row.version,
          contentHash: row.contentHash,
          stateRevision: row.stateRevision,
          accessPolicyRevision: row.accessPolicyRevision,
        },
      ],
      manifest: replay.manifest,
    });
    Assert.equal(result.kind, "committed");
    Assert.equal(commitInput?.readRequests.length, 2);
    Assert.equal(commitInput?.expectedVersions.length, 1);
  });

  it("maps native Rust debug spellings for conflict and access denial", async () => {
    const replay = replayManifest();
    const { port: conflictPort, basis: conflictBasis } = await openedPort(
      makeStore({
        commitContextManifest: async () => {
          throw new NativeHostClientError("HOST_OPERATION", "ERR\tOperationConflict\t10us");
        },
      }),
    );
    const input = {
      operationId: snapshot.operationId,
      requestDigest: `sha256:${"d".repeat(64)}`,
      basis: conflictBasis,
      expectedVersions: [
        {
          sourceDomainId: row.sourceDomainId,
          contextId: row.contextId,
          version: row.version,
          contentHash: row.contentHash,
          stateRevision: row.stateRevision,
          accessPolicyRevision: row.accessPolicyRevision,
        },
      ],
      manifest: replay.manifest,
    };
    Assert.equal((await conflictPort.commitManifest(input)).kind, "conflict");

    const { port: deniedPort, basis: deniedBasis } = await openedPort(
      makeStore({
        commitContextManifest: async () => {
          throw new NativeHostClientError("HOST_OPERATION", "ERR\tAccessDenied\t11us");
        },
      }),
    );
    Assert.equal(
      (await deniedPort.commitManifest({ ...input, basis: deniedBasis })).kind,
      "denied",
    );
  });

  it("does not map ordinary transport text containing authority words", async () => {
    const replay = replayManifest();
    const { port, basis } = await openedPort(
      makeStore({
        commitContextManifest: async () => {
          throw new Error("transport failure mentions access grant state but has unknown outcome");
        },
      }),
    );
    await Assert.rejects(
      port.commitManifest({
        operationId: snapshot.operationId,
        requestDigest: `sha256:${"d".repeat(64)}`,
        basis,
        expectedVersions: [
          {
            sourceDomainId: row.sourceDomainId,
            contextId: row.contextId,
            version: row.version,
            contentHash: row.contentHash,
            stateRevision: row.stateRevision,
            accessPolicyRevision: row.accessPolicyRevision,
          },
        ],
        manifest: replay.manifest,
      }),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "COMMIT_OUTCOME_UNKNOWN",
    );
  });

  it("rejects a crossed canonical replay receipt", async () => {
    const crossed = replayManifest();
    const { port } = await openedPort(
      makeStore({
        readContextManifest: async () =>
          Object.freeze({
            ...crossed.receipt,
            manifestId: "other-manifest",
          }),
      }),
    );
    await Assert.rejects(
      port.readCurrentManifest({
        principalId: snapshot.principalId,
        seatId: snapshot.seatId,
        taskId: snapshot.taskId,
        sessionId: snapshot.sessionId,
        domainId: snapshot.domainId,
        bindingId: snapshot.bindingId,
        bindingGeneration: snapshot.bindingGeneration,
        sourceEpoch: snapshot.sourceEpoch,
        runtimeInstanceId: snapshot.runtimeInstanceId,
        operationId: snapshot.operationId,
      }),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "AUTHORITY_PROTOCOL_ERROR",
    );
  });

  it("rejects noncanonical replay receipt bytes even when parsed JSON matches", async () => {
    const canonical = replayManifest();
    const port = new NativeContextAssemblyAuthorityPort({
      store: makeStore({
        readContextManifest: async () => ({
          ...canonical.receipt,
          canonicalManifest: ` ${canonical.receipt.canonicalManifest}`,
        }),
      }),
      plan,
      content: makeContent(),
    });
    await Assert.rejects(
      port.readCurrentManifest({
        principalId: snapshot.principalId,
        seatId: snapshot.seatId,
        taskId: snapshot.taskId,
        sessionId: snapshot.sessionId,
        domainId: snapshot.domainId,
        bindingId: snapshot.bindingId,
        bindingGeneration: snapshot.bindingGeneration,
        sourceEpoch: snapshot.sourceEpoch,
        runtimeInstanceId: snapshot.runtimeInstanceId,
        operationId: snapshot.operationId,
      }),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "AUTHORITY_PROTOCOL_ERROR",
    );
  });

  it("rejects an omitted current-Task constraint before source or commit transport", async () => {
    let listCalls = 0;
    let commitCalls = 0;
    const { port, basis } = await openedPort(
      makeStore({
        listContextAssemblySources: async () => {
          listCalls += 1;
          return [row];
        },
        commitContextManifest: async () => {
          commitCalls += 1;
          return replayManifest().receipt;
        },
      }),
    );
    const original = replayManifest().manifest;
    const omittedBody = {
      ...original,
      requiredConstraints: [],
      includedVersions: [],
    };
    const { manifestHash: ignored, ...omittedWithoutHash } = omittedBody;
    const omitted = Object.freeze({
      ...omittedWithoutHash,
      manifestHash: hashData(omittedWithoutHash),
    }) as unknown as ContextManifest;
    await Assert.rejects(
      port.commitManifest({
        operationId: snapshot.operationId,
        requestDigest: `sha256:${"d".repeat(64)}`,
        basis,
        expectedVersions: [],
        manifest: omitted,
      }),
      (error: unknown) => error instanceof ContextAssemblyError && error.code === "ACCESS_DENIED",
    );
    Assert.equal(listCalls, 0);
    Assert.equal(commitCalls, 0);
  });

  it("rejects an extra required constraint not present in current Task authority", async () => {
    let listCalls = 0;
    const { port, basis } = await openedPort(
      makeStore({
        listContextAssemblySources: async () => {
          listCalls += 1;
          return [row, excludedRow];
        },
      }),
    );
    const original = replayManifest().manifest;
    const extra = {
      sourceDomainId: excludedRow.sourceDomainId,
      contextId: excludedRow.contextId,
      version: excludedRow.version,
      contentHash: excludedRow.contentHash,
      stateRevision: excludedRow.stateRevision,
      accessPolicyRevision: excludedRow.accessPolicyRevision,
    };
    const { manifestHash: ignored, ...withoutHash } = {
      ...original,
      requiredConstraints: [...original.requiredConstraints, extra],
      includedVersions: [
        ...original.includedVersions,
        { ...extra, reason: "MANDATORY_CONSTRAINT" },
      ],
    };
    const manifest = Object.freeze({
      ...withoutHash,
      manifestHash: hashData(withoutHash),
    }) as unknown as ContextManifest;
    await Assert.rejects(
      port.commitManifest({
        operationId: snapshot.operationId,
        requestDigest: `sha256:${"d".repeat(64)}`,
        basis,
        expectedVersions: [
          {
            ...required,
            contentHash: row.contentHash,
            stateRevision: row.stateRevision,
            accessPolicyRevision: row.accessPolicyRevision,
          },
          extra,
        ],
        manifest,
      }),
      (error: unknown) => error instanceof ContextAssemblyError && error.code === "ACCESS_DENIED",
    );
    Assert.equal(listCalls, 0);
  });

  it("accepts replay whose canonical manifest discloses only used domains", async () => {
    const unusedPartition = Object.freeze({
      sourceDomainId: "unused-domain",
      destinationScope: "PROJECT" as const,
      promotionKind: "PROJECT_ONLY",
      grant: Object.freeze({ grantId: "grant-unused", revision: "1", revocationHead: "4" }),
    });
    const unusedPlan: NativeContextAssemblyPlan = Object.freeze({
      ...plan,
      snapshot: Object.freeze({
        ...snapshot,
        partitionBindings: Object.freeze([...snapshot.partitionBindings, unusedPartition]),
      }),
    });
    const replay = replayManifest();
    const port = new NativeContextAssemblyAuthorityPort({
      store: makeStore({ readContextManifest: async () => replay.receipt }),
      plan: unusedPlan,
      content: makeContent(),
    });
    const result = await port.readCurrentManifest({
      principalId: snapshot.principalId,
      seatId: snapshot.seatId,
      taskId: snapshot.taskId,
      sessionId: snapshot.sessionId,
      domainId: snapshot.domainId,
      bindingId: snapshot.bindingId,
      bindingGeneration: snapshot.bindingGeneration,
      sourceEpoch: snapshot.sourceEpoch,
      runtimeInstanceId: snapshot.runtimeInstanceId,
      operationId: snapshot.operationId,
    });
    Assert.equal(result?.manifestHash, replay.manifest.manifestHash);
  });

  it("rejects a commit receipt with crossed canonical bytes", async () => {
    const replay = replayManifest();
    const { port, basis } = await openedPort(
      makeStore({
        commitContextManifest: async () =>
          Object.freeze({
            disposition: "COMMITTED",
            operationId: snapshot.operationId,
            manifestId: replay.manifest.manifestId,
            manifestHash: replay.manifest.manifestHash,
            canonicalManifest: "{}",
          }),
      }),
    );
    await Assert.rejects(
      port.commitManifest({
        operationId: snapshot.operationId,
        requestDigest: `sha256:${"d".repeat(64)}`,
        basis,
        expectedVersions: [
          {
            sourceDomainId: row.sourceDomainId,
            contextId: row.contextId,
            version: row.version,
            contentHash: row.contentHash,
            stateRevision: row.stateRevision,
            accessPolicyRevision: row.accessPolicyRevision,
          },
        ],
        manifest: replay.manifest,
      }),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "COMMIT_OUTCOME_UNKNOWN",
    );
  });

  it("rejects a forged basis outside the port-issued frozen identity", async () => {
    let listCalls = 0;
    const { port, basis } = await openedPort(
      makeStore({
        listContextAssemblySources: async () => {
          listCalls += 1;
          return [row];
        },
      }),
    );
    await Assert.rejects(
      port.searchVisibleContext(
        Object.freeze({ ...basis, maxContentBytes: basis.maxContentBytes - 1 }),
        "query",
      ),
      (error: unknown) =>
        error instanceof ContextAssemblyError && error.code === "ACCESS_DENIED",
    );
    Assert.equal(listCalls, 0);
  });

  it("performs fresh Task reads before each downstream native or content call", async () => {
    const calls: string[] = [];
    const replay = replayManifest();
    const store = makeStore({
      publishContextAssemblySnapshot: async () => {
        calls.push("publish");
      },
      readContextAssemblyBasis: async () => {
        calls.push("basis");
        return nativeBasis;
      },
      readTaskContextRequirements: async () => {
        calls.push("task");
        return currentTask;
      },
      listContextAssemblySources: async () => {
        calls.push("list");
        return [row];
      },
      readGranteeContextSet: async () => {
        calls.push("read");
        return nativeReadSet;
      },
      commitContextManifest: async () => {
        calls.push("commit");
        return Object.freeze({ ...replay.receipt, disposition: "COMMITTED" as const });
      },
      readContextManifest: async () => {
        calls.push("replay");
        return replay.receipt;
      },
    });
    const content = makeContent({
      search: async () => {
        calls.push("search");
        return [required];
      },
      load: async () => {
        calls.push("load");
        return [resolved];
      },
    });
    const opened = await openedPort(store, content);
    await opened.port.searchVisibleContext(opened.basis, "query");
    await opened.port.loadVisibleVersions(opened.basis, [required]);
    await opened.port.commitManifest({
      operationId: snapshot.operationId,
      requestDigest: `sha256:${"d".repeat(64)}`,
      basis: opened.basis,
      expectedVersions: [
        {
          ...required,
          contentHash: row.contentHash,
          stateRevision: row.stateRevision,
          accessPolicyRevision: row.accessPolicyRevision,
        },
      ],
      manifest: replay.manifest,
    });
    await opened.port.readCurrentManifest(replayRequest());
    Assert.deepEqual(calls, [
      "publish",
      "basis",
      "task",
      "list",
      "search",
      "task",
      "list",
      "read",
      "load",
      "task",
      "list",
      "commit",
      "task",
      "replay",
    ]);
  });
});
