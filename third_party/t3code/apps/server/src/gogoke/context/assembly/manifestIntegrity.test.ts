import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import type { ContextManifest, ContextObject, SeatId, U64String } from "../../contracts/model.ts";
import { ContextManifestAssembler } from "./assembler.ts";
import {
  ContextAssemblyError,
  type AssemblyBasis,
  type AssemblyIdentity,
  type AssemblyRequest,
  type ContextAssemblyAuthorityPort,
  type ReplayManifestRequest,
} from "./model.ts";
import { canonical, hashData, hashText } from "./passive.ts";

const identity: AssemblyIdentity = {
  principalId: "owner-one",
  seatId: "seat-one",
  taskId: "task-one",
  sessionId: "session-one",
  domainId: "domain-one",
  bindingId: "binding-one",
  bindingGeneration: "7",
  sourceEpoch: "3",
  runtimeInstanceId: "runtime-one",
};
const query: ReplayManifestRequest = { ...identity, operationId: "operation-one" };
const expected = {
  sourceDomainId: "domain-one",
  contextId: "mandatory",
  version: "1",
  contentHash: hashText("required"),
  stateRevision: "4",
  accessPolicyRevision: "5",
};
const body = (): Omit<ContextManifest, "manifestHash"> => ({
  manifestId: "manifest-one",
  taskId: "task-one",
  seatId: "seat-one" as SeatId,
  bindingGeneration: "7" as U64String,
  domainId: "domain-one",
  policyRevision: "12" as U64String,
  sourceSnapshot: {
    assemblySchema: "gogoke.context-assembly.v1",
    operationId: "operation-one",
    requestDigest: hashText("request"),
    principalId: "owner-one",
    sessionId: "session-one",
    bindingId: "binding-one",
    sourceEpoch: "3",
    runtimeInstanceId: "runtime-one",
    taskRevision: "11",
    authRevision: "13",
    revocationHead: "14",
    partitions: [{ sourceDomainId: "domain-one" }],
    mode: "FIXED_SOURCE_RULES",
    excluded: [],
  },
  requiredConstraints: [{ ...expected }],
  includedVersions: [{ ...expected, reason: "MANDATORY_CONSTRAINT" }],
  redactions: [],
  selectionDecisionId: "rules-one",
});
const manifest = (value: Omit<ContextManifest, "manifestHash">): ContextManifest => ({
  ...value,
  manifestHash: hashData(value),
});
function replayOnly(value: ContextManifest): Promise<ContextManifest> {
  const unused = async (): Promise<never> => {
    throw new Error("unexpected port invocation");
  };
  return new ContextManifestAssembler({
    openAssembly: unused,
    searchVisibleContext: unused,
    loadVisibleVersions: unused,
    commitManifest: unused,
    readCurrentManifest: async () => value,
  }).replay(query);
}
const protocolError = (e: unknown): boolean =>
  e instanceof ContextAssemblyError && e.code === "AUTHORITY_PROTOCOL_ERROR";

async function assemblyWithPrivateBasis(): Promise<ContextManifest> {
  const request: AssemblyRequest = {
    ...identity,
    operationId: "operation-one",
    manifestId: "manifest-one",
    query: "fixed sources",
    maxContentBytes: 100,
  };
  const basis: AssemblyBasis = {
    ...identity,
    readRef: "read-private",
    admissionRef: "admission-private",
    taskRevision: "11",
    policyRevision: "12",
    authRevision: "13",
    revocationHead: "14",
    selectionDecisionId: "rules-one",
    maxContentBytes: 100,
    maxCandidates: 0,
    partitions: [
      {
        sourceDomainId: "domain-one",
        authorizationRef: "grant-private-issuer-record",
        authorizationRevision: "15",
      },
      {
        sourceDomainId: "unused-authorized-domain",
        authorizationRef: "unused-private-grant",
        authorizationRevision: "2",
      },
    ],
    requiredConstraints: [{ sourceDomainId: "domain-one", contextId: "mandatory", version: "1" }],
  };
  const object: ContextObject = {
    contextId: "mandatory",
    version: "1" as U64String,
    scope: "PROJECT",
    domainId: "domain-one",
    kind: "constraint",
    contentHash: hashText("required"),
    sourceRef: { ref: "source-one", hash: hashText("required") },
    sourceAuthority: { kind: "repository", ref: "source-one" },
    derivedFrom: [],
    validity: "ACTIVE",
    supersedes: [],
    accessPolicyRevision: "2" as U64String,
  };
  let stored: ContextManifest | null = null;
  const port: ContextAssemblyAuthorityPort = {
    openAssembly: async () => basis,
    searchVisibleContext: async () => [],
    loadVisibleVersions: async () => [
      {
        object,
        content: "required",
        state: "ACTIVE",
        stateRevision: "4",
        accessPolicyRevision: "5",
      },
    ],
    commitManifest: async (command) => {
      stored = command.manifest;
      return {
        kind: "committed",
        operationId: command.operationId,
        manifestId: stored.manifestId,
        manifestHash: stored.manifestHash,
      };
    },
    readCurrentManifest: async () => stored,
  };
  return new ContextManifestAssembler(port).assemble(request);
}

describe("R4-C-ASSEMBLE replay semantic integrity and minimum disclosure", () => {
  it("accepts a hash-valid self-consistent manifest with all mandatory versions", async () => {
    const value = manifest(body());
    Assert.equal((await replayOnly(value)).manifestHash, value.manifestHash);
  });

  it("rejects a hash-valid replay that omits a mandatory included version", async () => {
    await Assert.rejects(replayOnly(manifest({ ...body(), includedVersions: [] })), protocolError);
  });

  it("rejects mismatched mandatory and included revision tuples even with a valid hash", async () => {
    await Assert.rejects(
      replayOnly(
        manifest({
          ...body(),
          includedVersions: [{ ...expected, stateRevision: "99", reason: "MANDATORY_CONSTRAINT" }],
        }),
      ),
      protocolError,
    );
  });

  it("rejects duplicate included Context identities instead of replaying ambiguous selection", async () => {
    const entry = { ...expected, reason: "MANDATORY_CONSTRAINT" };
    await Assert.rejects(
      replayOnly(manifest({ ...body(), includedVersions: [entry, entry] })),
      protocolError,
    );
  });

  it("rejects multiple versions of one Context identity in durable replay", async () => {
    await Assert.rejects(
      replayOnly(
        manifest({
          ...body(),
          includedVersions: [
            { ...expected, reason: "MANDATORY_CONSTRAINT" },
            { ...expected, version: "2", reason: "AUTHORIZED_RETRIEVAL" },
          ],
        }),
      ),
      protocolError,
    );
  });

  it("rejects unknown selection reasons rather than interpreting them as authority", async () => {
    await Assert.rejects(
      replayOnly(
        manifest({
          ...body(),
          includedVersions: [{ ...expected, reason: "AUTOMATICALLY_GRANTED" }],
        }),
      ),
      protocolError,
    );
  });

  it("rejects a replay source absent from the manifest's disclosed source partitions", async () => {
    const value = body();
    await Assert.rejects(
      replayOnly(
        manifest({
          ...value,
          sourceSnapshot: {
            ...(value.sourceSnapshot as object),
            partitions: [],
          },
        }),
      ),
      protocolError,
    );
  });

  it("rejects version overflow in a hash-valid replay", async () => {
    const overflow = { ...expected, version: "18446744073709551616" };
    await Assert.rejects(
      replayOnly(
        manifest({
          ...body(),
          requiredConstraints: [overflow],
          includedVersions: [{ ...overflow, reason: "MANDATORY_CONSTRAINT" }],
        }),
      ),
      protocolError,
    );
  });

  it("rejects a mandatory reason whose required constraint record is missing", async () => {
    await Assert.rejects(
      replayOnly(manifest({ ...body(), requiredConstraints: [] })),
      protocolError,
    );
  });

  it("keeps private authorization handles in the authority command, not the caller manifest", async () => {
    const value = await assemblyWithPrivateBasis();
    Assert.equal(canonical(value).includes("grant-private-issuer-record"), false);
    Assert.equal(canonical(value).includes("admission-private"), false);
    Assert.equal(canonical(value).includes("read-private"), false);
  });

  it("does not disclose unused authorized-domain inventory in the caller manifest", async () => {
    const value = await assemblyWithPrivateBasis();
    Assert.equal(canonical(value).includes("unused-authorized-domain"), false);
    Assert.equal(canonical(value).includes("unused-private-grant"), false);
  });
});
