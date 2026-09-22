import { describe, expect, it, vi } from "vite-plus/test";

import { ContextRepository, ContextRepositoryError, contextVersionRef } from "./repository.ts";

const digest = (character: string): string => `sha256:${character.repeat(64)}`;
const object = (overrides: Record<string, unknown> = {}) => ({
  contextId: "context-one",
  version: "1",
  scope: "PROJECT",
  domainId: "domain-one",
  kind: "fact",
  contentHash: digest("a"),
  sourceRef: { ref: "source://one", hash: digest("b") },
  sourceAuthority: { kind: "repository", ref: "authority://one" },
  derivedFrom: [],
  validity: "ACTIVE",
  supersedes: [],
  accessPolicyRevision: "1",
  ...overrides,
});

const fixture = () => {
  const commitContextVersion = vi.fn(async (request) => ({
    operationId: request.operationId,
    contextId: request.object.contextId,
    version: request.object.version,
    disposition: "COMMITTED" as const,
    invalidatedVersionRefs: request.object.supersedes,
  }));
  return { commitContextVersion, repository: new ContextRepository({ commitContextVersion }) };
};

describe("R4-C-STORE repository boundary", () => {
  it("gogoke-s1-r4/R4-06 snapshots an immutable version and delegates one typed commit", async () => {
    const { repository, commitContextVersion } = fixture();
    const mutable = object({ supersedes: ["context-zero@1"] });
    const result = await repository.commit({
      operationId: "operation-one",
      object: mutable as never,
      access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
    });
    mutable.contextId = "forged";
    expect(result.invalidatedVersionRefs).toEqual(["context-zero@1"]);
    const submitted = commitContextVersion.mock.calls[0]![0];
    expect(submitted.object.contextId).toBe("context-one");
    expect(Object.isFrozen(submitted)).toBe(true);
    expect(Object.isFrozen(submitted.object)).toBe(true);
    expect(contextVersionRef(submitted.object)).toBe("context-one@1");
  });

  it("gogoke-s1-r4/R4-10 requires source and target grants plus provenance for GLOBAL", async () => {
    const { repository, commitContextVersion } = fixture();
    await expect(
      repository.commit({
        operationId: "operation-global",
        object: object({ scope: "GLOBAL", derivedFrom: ["project-context@3"] }) as never,
        access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
      }),
    ).rejects.toThrow(ContextRepositoryError);
    expect(commitContextVersion).not.toHaveBeenCalled();

    await repository.commit({
      operationId: "operation-global",
      object: object({ scope: "GLOBAL", derivedFrom: ["project-context@3"] }) as never,
      access: { visibility: "OWNER_PRIVATE", readGrantRefs: ["grant://reader"] },
      promotion: {
        sourceVersionRef: "project-context@3",
        sourceGrantRef: "grant://source",
        targetGrantRef: "grant://target",
        provenanceRefs: ["evidence://review"],
      },
    });
    expect(commitContextVersion).toHaveBeenCalledOnce();
  });

  it("GLOBAL remains owner-private unless an explicit access policy says otherwise", async () => {
    const { repository, commitContextVersion } = fixture();
    await repository.commit({
      operationId: "operation-private-global",
      object: object({ scope: "GLOBAL", derivedFrom: ["project-context@3"] }) as never,
      access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
      promotion: {
        sourceVersionRef: "project-context@3",
        sourceGrantRef: "grant://source",
        targetGrantRef: "grant://target",
        provenanceRefs: ["evidence://review"],
      },
    });
    expect(commitContextVersion.mock.calls[0]![0].access.visibility).toBe("OWNER_PRIVATE");
  });

  it("rejects self-supersede, missing source hash, active properties and extra authority", async () => {
    const { repository, commitContextVersion } = fixture();
    for (const invalidObject of [
      object({ supersedes: ["context-one@1"] }),
      object({ sourceRef: { ref: "source://one" } }),
      object({ sourceAuthority: { kind: "repository", ref: "authority://one", grant: true } }),
    ]) {
      await expect(
        repository.commit({
          operationId: "operation-invalid",
          object: invalidObject as never,
          access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
        }),
      ).rejects.toThrow(ContextRepositoryError);
    }
    let reads = 0;
    const active = object();
    Object.defineProperty(active, "contextId", {
      enumerable: true,
      get: () => {
        reads += 1;
        return "forged";
      },
    });
    await expect(
      repository.commit({
        operationId: "operation-active",
        object: active as never,
        access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
      }),
    ).rejects.toThrow(ContextRepositoryError);
    expect(reads).toBe(0);
    expect(commitContextVersion).not.toHaveBeenCalled();
  });

  it("snapshots synchronously and never executes array getters or hash coercion", async () => {
    const { repository, commitContextVersion } = fixture();
    const mutable = object();
    const committed = repository.commit({
      operationId: "operation-snapshot",
      object: mutable as never,
      access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
    });
    mutable.contextId = "caller-changed-before-await";
    await committed;
    expect(commitContextVersion.mock.calls[0]![0].object.contextId).toBe("context-one");

    let getterReads = 0;
    const activeArray = [] as unknown[];
    Object.defineProperty(activeArray, "0", {
      enumerable: true,
      get: () => {
        getterReads += 1;
        return "source@1";
      },
    });
    Object.defineProperty(activeArray, "length", { value: 1 });
    await expect(
      repository.commit({
        operationId: "operation-array",
        object: object({ derivedFrom: activeArray }) as never,
        access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
      }),
    ).rejects.toThrow(ContextRepositoryError);
    expect(getterReads).toBe(0);

    let coercions = 0;
    await expect(
      repository.commit({
        operationId: "operation-hash",
        object: object({
          contentHash: {
            toString: () => {
              coercions += 1;
              return digest("a");
            },
          },
        }) as never,
        access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
      }),
    ).rejects.toThrow(ContextRepositoryError);
    expect(coercions).toBe(0);
  });

  it("rejects an own __proto__ field instead of inheriting an authority field", async () => {
    const { repository, commitContextVersion } = fixture();
    const polluted = object() as Record<string, unknown>;
    delete polluted.domainId;
    polluted.padding = "keeps-the-own-key-count-at-twelve";
    Object.defineProperty(polluted, "__proto__", {
      value: { domainId: "forged-domain" },
      enumerable: true,
    });

    await expect(
      repository.commit({
        operationId: "operation-prototype",
        object: polluted as never,
        access: { visibility: "OWNER_PRIVATE", readGrantRefs: [] },
      }),
    ).rejects.toThrow(ContextRepositoryError);
    expect(commitContextVersion).not.toHaveBeenCalled();
  });
});
