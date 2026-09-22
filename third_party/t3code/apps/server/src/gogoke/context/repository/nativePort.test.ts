import { describe, expect, it } from "vite-plus/test";

import {
  decodeContextCommitReply,
  encodeContextVersionFrame,
  NativeHostClientError,
} from "../../persistence/base/nativeHostClient.ts";
import type { CommitContextVersionRequest } from "./repository.ts";

const digest = (character: string): string => `sha256:${character.repeat(64)}`;

const request = {
  operationId: "operation-one",
  object: {
    contextId: "context-one",
    version: "1",
    scope: "GLOBAL",
    domainId: "domain-one",
    kind: "fact",
    contentHash: digest("a"),
    sourceRef: { ref: "source://one", hash: digest("b") },
    sourceAuthority: { kind: "repository", ref: "authority://one" },
    derivedFrom: ["project@1"],
    validity: "ACTIVE",
    supersedes: [],
    accessPolicyRevision: "1",
  },
  access: { visibility: "OWNER_PRIVATE", readGrantRefs: ["grant://reader"] },
  promotion: {
    sourceVersionRef: "project@1",
    sourceGrantRef: "grant://source",
    targetGrantRef: "grant://target",
    provenanceRefs: ["evidence://review"],
  },
} as const satisfies Omit<CommitContextVersionRequest, "object"> & { readonly object: object };

describe("native context typed frame", () => {
  it("contains one closed operation and no SQL, path or authority shortcut", () => {
    const frame = encodeContextVersionFrame(request as unknown as CommitContextVersionRequest);
    expect(JSON.parse(frame)).toEqual({
      accessPolicyRevision: "1",
      contentHash: digest("a"),
      contextId: "context-one",
      derivedFrom: "project@1",
      domainId: "domain-one",
      kind: "fact",
      operation: "CommitContextVersion",
      operationId: "operation-one",
      provenanceRefs: "evidence://review",
      readGrantRefs: "grant://reader",
      scope: "GLOBAL",
      sourceAuthorityKind: "repository",
      sourceAuthorityRef: "authority://one",
      sourceGrantRef: "grant://source",
      sourceHash: digest("b"),
      sourceRef: "source://one",
      sourceVersionRef: "project@1",
      supersedes: "",
      targetGrantRef: "grant://target",
      version: "1",
      visibility: "OWNER_PRIVATE",
    });
    expect(frame).not.toContain("sql");
    expect(frame).not.toContain("databasePath");
  });

  it("accepts only the exact native receipt shape", () => {
    expect(
      decodeContextCommitReply(
        '{"disposition":"COMMITTED","operationId":"operation-one","contextId":"context-one","version":"1","invalidatedVersionRefs":["project@1"]}',
      ),
    ).toEqual({
      disposition: "COMMITTED",
      operationId: "operation-one",
      contextId: "context-one",
      version: "1",
      invalidatedVersionRefs: ["project@1"],
    });
    expect(() =>
      decodeContextCommitReply(
        '{"disposition":"COMMITTED","operationId":"operation-one","contextId":"context-one","version":"1","invalidatedVersionRefs":[],"sql":"forged"}',
      ),
    ).toThrow(NativeHostClientError);
  });
});
