import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";

import {
  decodeAuthorizedContextReadSet,
  decodeContextAssemblyBasis,
  decodeContextAssemblySources,
  decodeContextManifestReceipt,
  decodeTaskContextRequirements,
  decodeTaskContextRequirementsReceipt,
  encodeContextAssemblyReadFrame,
  encodeContextAssemblySnapshotFrame,
  encodeContextManifestCommitFrame,
  encodeContextManifestReplayFrame,
  encodeGranteeContextSetFrame,
  encodeTaskContextCommitFrame,
  encodeTaskContextReadFrame,
  NativeHostClient,
  NativeHostClientError,
  type NativeGranteeContextReadRequest,
} from "./nativeHostClient.ts";

const digest = (character: string): string => `sha256:${character.repeat(64)}`;

const read = (contextId = "context-one"): NativeGranteeContextReadRequest => ({
  principalId: "principal-one",
  seatId: "seat-one",
  source: {
    sourceDomainId: "domain-source",
    contextId,
    version: "1",
    expectedScope: "PROJECT",
    expectedContentHash: digest("a"),
    expectedAccessPolicyRevision: "1",
    destinationDomainId: "domain-one",
    destinationScope: "PROJECT",
    promotionKind: "PROJECT_ONLY",
    policyRevision: "1",
    grant: { grantId: "grant-one", revision: "2", revocationHead: "0" },
  },
});

const record = (tag: string, values: ReadonlyArray<string>): string =>
  `${tag}|1|${values.map((value) => `${Buffer.byteLength(value, "utf8")}:${value}`).join("")}`;

describe("authenticated native ContextManifest codec", () => {
  it("encodes exact snapshot and replay identities without actor booleans", () => {
    const input = {
      operationId: "manifest-operation",
      principalId: "principal-one",
      seatId: "seat-one",
      taskId: "task-one",
      sessionId: "session-one",
      domainId: "domain-one",
      bindingId: "binding-one",
      bindingGeneration: "7",
      sourceEpoch: "9",
      runtimeInstanceId: "runtime-one",
      taskRevision: "1",
      policyRevision: "1",
      authRevision: "2",
      revocationHead: "0",
      selectionDecisionId: "decision-one",
      manifestId: "manifest-one",
      admissionActionOperationId: "opr_11111111111111111111111111111111",
      admissionDigest: digest("c"),
      maxContentBytes: 4096,
      maxCandidates: 8,
      partitionBindings: [
        {
          sourceDomainId: "domain-source",
          destinationScope: "PROJECT",
          promotionKind: "PROJECT_ONLY",
          grant: { grantId: "grant-one", revision: "2", revocationHead: "0" },
        },
      ],
    } as const;
    const snapshot = JSON.parse(encodeContextAssemblySnapshotFrame(input)) as Record<
      string,
      unknown
    >;
    Assert.equal(snapshot.operation, "PublishContextAssemblySnapshot");
    Assert.equal(snapshot.maxCandidates, "8");
    Assert.match(
      String(snapshot.partitionBindings),
      /^gogoke\.context-assembly-partition-bindings\.v1\|1\|/,
    );
    Assert.equal("accessAdmitted" in snapshot, false);
    Assert.equal("grant" in snapshot, false);
    Assert.throws(
      () => encodeContextAssemblySnapshotFrame({ ...input, maxCandidates: -1 }),
      NativeHostClientError,
    );

    const replay = JSON.parse(
      encodeContextManifestReplayFrame({
        operationId: "manifest-operation",
        principalId: "principal-one",
        seatId: "seat-one",
        taskId: "task-one",
        sessionId: "session-one",
        domainId: "domain-one",
        bindingId: "binding-one",
        bindingGeneration: "7",
        sourceEpoch: "9",
        runtimeInstanceId: "runtime-one",
      }),
    ) as Record<string, unknown>;
    Assert.equal(replay.operation, "ReadContextManifest");
    Assert.deepEqual(Reflect.ownKeys(replay).sort(), [
      "bindingGeneration",
      "bindingId",
      "domainId",
      "operation",
      "operationId",
      "principalId",
      "runtimeInstanceId",
      "seatId",
      "sessionId",
      "sourceEpoch",
      "taskId",
    ]);
  });

  it("uses UTF-8 byte lengths and fixed-width bounded read records", () => {
    const frame = JSON.parse(encodeGranteeContextSetFrame([read("é")])) as {
      readRequests: string;
    };
    Assert.match(frame.readRequests, /2:é/);
    Assert.throws(() => encodeGranteeContextSetFrame([]), NativeHostClientError);
    Assert.throws(
      () => encodeGranteeContextSetFrame(Array.from({ length: 65 }, () => read())),
      NativeHostClientError,
    );

    const values = [
      "2",
      "0",
      "ACTIVE",
      "1",
      "domain-source",
      "é",
      "1",
      "PROJECT",
      "fact",
      digest("a"),
      "source://one",
      digest("b"),
      "repository",
      "authority://one",
      "1",
    ];
    const decoded = decodeAuthorizedContextReadSet(
      JSON.stringify({
        destinationDomainId: "domain-one",
        destinationScope: "PROJECT",
        policyRevision: "1",
        principalId: "principal-one",
        promotionKind: "PROJECT_ONLY",
        revocationHead: "0",
        seatId: "seat-one",
        sources: record("gogoke.authorized-context-read-sources.v1", values),
      }),
    );
    Assert.equal(decoded.sources[0]?.contextId, "é");
    Assert.equal(Object.isFrozen(decoded.sources), true);
    Assert.throws(
      () =>
        decodeAuthorizedContextReadSet(
          JSON.stringify({
            destinationDomainId: "domain-one",
            destinationScope: "PROJECT",
            policyRevision: "1",
            principalId: "principal-one",
            promotionKind: "PROJECT_ONLY",
            revocationHead: "0",
            seatId: "seat-one",
            sources: `${record("gogoke.authorized-context-read-sources.v1", values)}x`,
          }),
        ),
      NativeHostClientError,
    );
  });

  it("decodes strict assembly basis and currently authorized source provenance", async () => {
    const identity = {
      operationId: "manifest-operation",
      principalId: "principal-one",
      seatId: "seat-one",
      taskId: "task-one",
      sessionId: "session-one",
      domainId: "domain-one",
      bindingId: "binding-one",
      bindingGeneration: "7",
      sourceEpoch: "9",
      runtimeInstanceId: "runtime-one",
    } as const;
    const basisFrame = JSON.parse(
      encodeContextAssemblyReadFrame("ReadContextAssemblyBasis", identity),
    ) as Record<string, unknown>;
    Assert.equal(basisFrame.operation, "ReadContextAssemblyBasis");
    Assert.equal(basisFrame.operationId, identity.operationId);

    const partitionBindings = record("gogoke.context-assembly-partition-bindings.v1", [
      "domain-source",
      "PROJECT",
      "PROJECT_ONLY",
      "grant-one",
      "2",
      "0",
    ]);
    const basisBody = JSON.stringify({
      authRevision: "2",
      bindingGeneration: "7",
      mandatoryRefs: record("gogoke.task-mandatory-context-refs.v1", [
        "domain-source",
        "context-one",
        "1",
      ]),
      maxCandidates: "8",
      maxContentBytes: "4096",
      operationId: identity.operationId,
      partitionBindings,
      policyRevision: "1",
      revocationHead: "0",
      sourceEpoch: "9",
      taskRevision: "1",
    });
    const basis = decodeContextAssemblyBasis(basisBody, identity.operationId);
    Assert.equal(basis.partitionBindings[0]?.grant.grantId, "grant-one");
    Assert.deepEqual(basis.mandatoryRefs, [
      { sourceDomainId: "domain-source", contextId: "context-one", version: "1" },
    ]);
    Assert.equal(Object.isFrozen(basis.mandatoryRefs), true);
    Assert.equal(Object.isFrozen(basis.partitionBindings[0]?.grant), true);
    Assert.throws(
      () =>
        decodeContextAssemblyBasis(
          JSON.stringify({ ...basis, maxCandidates: "65", partitionBindings }),
          identity.operationId,
        ),
      NativeHostClientError,
    );
    Assert.throws(
      () => decodeContextAssemblyBasis(basisBody, "different-operation"),
      NativeHostClientError,
    );

    const sourceValues = [
      "domain-source",
      "é",
      "1",
      "PROJECT",
      "fact",
      digest("a"),
      "source://one",
      digest("b"),
      "repository",
      "authority://one",
      "1",
      "3",
      "grant-one",
      "2",
      "0",
    ];
    const sourceBody = JSON.stringify({
      operationId: identity.operationId,
      sources: record("gogoke.context-assembly-sources.v1", sourceValues),
    });
    const sources = decodeContextAssemblySources(sourceBody, identity.operationId);
    Assert.deepEqual(sources[0], {
      sourceDomainId: "domain-source",
      contextId: "é",
      version: "1",
      scope: "PROJECT",
      kind: "fact",
      contentHash: digest("a"),
      sourceRef: "source://one",
      sourceHash: digest("b"),
      sourceAuthorityKind: "repository",
      sourceAuthorityRef: "authority://one",
      accessPolicyRevision: "1",
      stateRevision: "3",
      grant: {
        grantId: "grant-one",
        revision: "2",
        revocationHead: "0",
      },
    });
    Assert.equal(Object.isFrozen(sources[0]), true);
    Assert.equal(Object.isFrozen(sources[0]?.grant), true);
    Assert.throws(
      () => decodeContextAssemblySources(sourceBody, "different-operation"),
      NativeHostClientError,
    );
    Assert.throws(
      () =>
        decodeContextAssemblySources(
          JSON.stringify({
            operationId: identity.operationId,
            sources: record("gogoke.context-assembly-sources.v1", sourceValues.slice(0, 14)),
          }),
          identity.operationId,
        ),
      NativeHostClientError,
    );

    const frames: string[] = [];
    const client = Object.assign(Object.create(NativeHostClient.prototype) as object, {
      request: (frame: string) => {
        frames.push(frame);
        const operation = (JSON.parse(frame) as { operation: string }).operation;
        return {
          ok: true,
          body:
            operation === "ReadContextAssemblyBasis"
              ? basisBody
              : JSON.stringify({
                  operationId: identity.operationId,
                  sources: "gogoke.context-assembly-sources.v1|0|",
                }),
          elapsedMicros: 0,
        };
      },
    }) as unknown as NativeHostClient;
    await client.readContextAssemblyBasis(identity);
    await client.listContextAssemblySources(identity);
    Assert.deepEqual(
      frames.map((frame) => (JSON.parse(frame) as { operation: string }).operation),
      ["ReadContextAssemblyBasis", "ListContextAssemblySources"],
    );
    Assert.equal(
      frames.every(
        (frame) =>
          (JSON.parse(frame) as { operationId: string }).operationId === identity.operationId,
      ),
      true,
    );
  });

  it("uses the fixed Task mandatory-ref wire for create, revise, UTF-8 and strict replies", async () => {
    const create = {
      operationId: "task-create",
      domainId: "domain-one",
      taskId: "task-one",
      expectedPreviousTaskRevision: null,
      mandatoryRefs: [{ sourceDomainId: "domain-source", contextId: "é", version: "1" }],
      eventId: "task-event-1",
      receiptId: "task-receipt-1",
      recordedAt: "2026-09-21T00:00:00Z",
    } as const;
    const createFrame = JSON.parse(encodeTaskContextCommitFrame(create)) as Record<
      string,
      unknown
    >;
    Assert.equal(createFrame.operation, "CommitTaskContextRequirements");
    Assert.equal(createFrame.expectedPreviousTaskRevision, "");
    Assert.equal(
      createFrame.mandatoryRefs,
      "gogoke.task-mandatory-context-refs.v1|1|13:domain-source2:é1:1",
    );
    const reviseFrame = JSON.parse(
      encodeTaskContextCommitFrame({
        ...create,
        operationId: "task-revise",
        expectedPreviousTaskRevision: "1",
        mandatoryRefs: [],
      }),
    ) as Record<string, unknown>;
    Assert.equal(reviseFrame.expectedPreviousTaskRevision, "1");
    Assert.equal(reviseFrame.mandatoryRefs, "gogoke.task-mandatory-context-refs.v1|0|");
    Assert.deepEqual(JSON.parse(encodeTaskContextReadFrame(create)), {
      domainId: "domain-one",
      operation: "ReadTaskContextRequirements",
      taskId: "task-one",
    });

    const currentBody = JSON.stringify({
      contentHash: digest("7"),
      domainId: create.domainId,
      mandatoryRefs: createFrame.mandatoryRefs,
      taskId: create.taskId,
      taskRevision: "1",
    });
    const current = decodeTaskContextRequirements(currentBody, create);
    Assert.equal(current.mandatoryRefs[0]?.contextId, "é");
    Assert.equal(Object.isFrozen(current.mandatoryRefs[0]), true);
    const receiptBody = JSON.stringify({
      contentHash: digest("7"),
      disposition: "COMMITTED",
      domainId: create.domainId,
      mandatoryRefs: createFrame.mandatoryRefs,
      operationId: create.operationId,
      taskId: create.taskId,
      taskRevision: "1",
    });
    Assert.equal(
      decodeTaskContextRequirementsReceipt(receiptBody, create).disposition,
      "COMMITTED",
    );
    Assert.throws(
      () => decodeTaskContextRequirementsReceipt(receiptBody, { ...create, operationId: "crossed" }),
      NativeHostClientError,
    );
    Assert.throws(
      () =>
        decodeTaskContextRequirements(
          JSON.stringify({
            ...current,
            mandatoryRefs: "gogoke.task-mandatory-context-refs.v1|1|1:é1:x1:1",
          }),
          create,
        ),
      NativeHostClientError,
    );
    Assert.throws(
      () =>
        decodeTaskContextRequirements(
          JSON.stringify({
            ...current,
            mandatoryRefs: "gogoke.task-mandatory-context-refs.v1|1|1:a1:b",
          }),
          create,
        ),
      NativeHostClientError,
    );
    Assert.throws(
      () =>
        decodeTaskContextRequirements(
          JSON.stringify({ ...current, mandatoryRefs: `${String(createFrame.mandatoryRefs)}x` }),
          create,
        ),
      NativeHostClientError,
    );

    const frames: string[] = [];
    const client = Object.assign(Object.create(NativeHostClient.prototype) as object, {
      request: (frame: string) => {
        frames.push(frame);
        const operation = (JSON.parse(frame) as { operation: string }).operation;
        return {
          ok: true,
          body: operation === "CommitTaskContextRequirements" ? receiptBody : currentBody,
          elapsedMicros: 0,
        };
      },
    }) as unknown as NativeHostClient;
    await client.commitTaskContextRequirements(create);
    await client.readTaskContextRequirements(create);
    Assert.deepEqual(
      frames.map((frame) => (JSON.parse(frame) as { operation: string }).operation),
      ["CommitTaskContextRequirements", "ReadTaskContextRequirements"],
    );
  });

  it("binds complete Manifest commit inputs and rejects widened replies", () => {
    const canonicalManifest = JSON.stringify({ manifestId: "manifest-one" });
    const frame = JSON.parse(
      encodeContextManifestCommitFrame({
        operationId: "manifest-operation",
        requestDigest: digest("f"),
        eventId: "manifest-event",
        receiptId: "manifest-receipt",
        recordedAt: "2026-09-21T00:00:00Z",
        readRequests: [read()],
        expectedVersions: [
          {
            sourceDomainId: "domain-source",
            contextId: "context-one",
            version: "1",
            contentHash: digest("a"),
            stateRevision: "1",
            accessPolicyRevision: "1",
          },
        ],
        canonicalManifest,
      }),
    ) as Record<string, unknown>;
    Assert.equal(frame.operation, "CommitContextManifest");
    Assert.equal(frame.canonicalManifest, canonicalManifest);
    Assert.match(String(frame.expectedVersions), /^gogoke\.manifest-expected-versions\.v1\|1\|/);

    const receipt = decodeContextManifestReceipt(
      JSON.stringify({
        canonicalManifest,
        disposition: "COMMITTED",
        manifestHash: digest("d"),
        manifestId: "manifest-one",
        operationId: "manifest-operation",
      }),
    );
    Assert.equal(receipt.disposition, "COMMITTED");
    Assert.throws(
      () => decodeContextManifestReceipt(JSON.stringify({ ...receipt, extra: true })),
      NativeHostClientError,
    );
    Assert.throws(
      () => decodeContextManifestReceipt(JSON.stringify({ ...receipt, disposition: "RECONCILED" })),
      NativeHostClientError,
    );
  });
});
