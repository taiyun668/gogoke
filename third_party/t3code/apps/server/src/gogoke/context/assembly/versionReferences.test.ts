import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import type { ContextManifest, ContextObject, U64String } from "../../contracts/model.ts";
import { ContextManifestAssembler } from "./assembler.ts";
import {
  ContextAssemblyError,
  type AssemblyBasis,
  type AssemblyRequest,
  type ContextAssemblyAuthorityPort,
} from "./model.ts";
import { hashText } from "./passive.ts";

async function assembleReferences(
  references: readonly string[],
  field: "derivedFrom" | "supersedes",
) {
  const identity = {
    principalId: "owner",
    seatId: "seat",
    taskId: "task",
    sessionId: "session",
    domainId: "domain",
    bindingId: "binding",
    bindingGeneration: "1",
    sourceEpoch: "1",
    runtimeInstanceId: "runtime",
  };
  const input: AssemblyRequest = {
    ...identity,
    operationId: "operation",
    manifestId: "manifest",
    query: "q",
    maxContentBytes: 100,
  };
  const basis: AssemblyBasis = {
    ...identity,
    readRef: "read",
    admissionRef: "admission",
    taskRevision: "1",
    policyRevision: "1",
    authRevision: "1",
    revocationHead: "1",
    selectionDecisionId: "decision",
    maxContentBytes: 100,
    maxCandidates: 0,
    partitions: [
      { sourceDomainId: "domain", authorizationRef: "grant", authorizationRevision: "1" },
    ],
    requiredConstraints: [{ sourceDomainId: "domain", contextId: "context", version: "1" }],
  };
  const object: ContextObject = {
    contextId: "context",
    version: "1" as U64String,
    scope: "PROJECT",
    domainId: "domain",
    kind: "fact",
    contentHash: hashText("bytes"),
    sourceRef: { ref: "source", hash: hashText("bytes") },
    sourceAuthority: { kind: "repository", ref: "source" },
    derivedFrom: [],
    validity: "ACTIVE",
    supersedes: [],
    accessPolicyRevision: "1" as U64String,
    [field]: references,
  };
  let stored: ContextManifest | null = null;
  const port: ContextAssemblyAuthorityPort = {
    openAssembly: async () => basis,
    searchVisibleContext: async () => [],
    loadVisibleVersions: async () => [
      { object, content: "bytes", state: "ACTIVE", stateRevision: "1", accessPolicyRevision: "1" },
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
  return new ContextManifestAssembler(port).assemble(input);
}
const invalid = (error: unknown): boolean =>
  error instanceof ContextAssemblyError && error.code === "AUTHORITY_PROTOCOL_ERROR";

describe("R4-C-ASSEMBLE Context version-reference compatibility", () => {
  it("accepts canonical compact Context version references", async () => {
    const result = await assembleReferences(["source-one@1"], "derivedFrom");
    Assert.equal(result.requiredConstraints.length, 1);
  });
  it("preserves the Context repository's maximum valid version-reference length", async () => {
    const reference = `${"a".repeat(256)}@18446744073709551615`;
    for (const field of ["derivedFrom", "supersedes"] as const) {
      Assert.equal((await assembleReferences([reference], field)).requiredConstraints.length, 1);
    }
  });
  it("rejects version-reference uint64 overflow rather than admitting it as a plain identifier", async () => {
    for (const field of ["derivedFrom", "supersedes"] as const) {
      await Assert.rejects(assembleReferences(["parent@18446744073709551616"], field), invalid);
    }
  });
  it("rejects Context provenance references without an immutable version", async () => {
    await Assert.rejects(assembleReferences(["parent-without-version"], "derivedFrom"), invalid);
  });
  it("rejects noncanonical leading-zero provenance versions", async () => {
    await Assert.rejects(assembleReferences(["parent@01"], "supersedes"), invalid);
  });
});
