import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import type { ContextManifest, ContextObject, U64String } from "../../contracts/model.ts";
import { ContextManifestAssembler } from "./assembler.ts";
import {
  ContextAssemblyError,
  type AssemblyBasis,
  type AssemblyRequest,
  type AssemblyIdentity,
  type CommitManifestRequest,
  type CommitManifestResult,
  type ContextAssemblyAuthorityPort,
  type ContextVersionRef,
  type ReplayManifestRequest,
  type ResolvedContextVersion,
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
const request = (overrides: Partial<AssemblyRequest> = {}): AssemblyRequest => ({
  ...identity,
  operationId: "operation-one",
  manifestId: "manifest-one",
  query: "relevant facts",
  maxContentBytes: 100,
  ...overrides,
});
const ref = (
  contextId: string,
  sourceDomainId = "domain-one",
  version = "1",
): ContextVersionRef => ({ sourceDomainId, contextId, version });
const source = (
  contextId: string,
  content: string,
  overrides: Partial<ContextObject> = {},
): ResolvedContextVersion => ({
  object: {
    contextId,
    version: "1" as U64String,
    scope: "PROJECT",
    domainId: "domain-one",
    kind: "fact",
    contentHash: hashText(content),
    sourceRef: { ref: "source-one", hash: hashText(content) },
    sourceAuthority: { kind: "repository", ref: "source-authority" },
    derivedFrom: [],
    validity: "ACTIVE",
    supersedes: [],
    accessPolicyRevision: "2" as U64String,
    ...overrides,
  },
  content,
  state: "ACTIVE",
  stateRevision: "4",
  accessPolicyRevision: "5",
});
const key = (r: ContextVersionRef): string => [r.sourceDomainId, r.contextId, r.version].join("|");
const rowRef = (r: ResolvedContextVersion): ContextVersionRef =>
  ref(r.object.contextId, r.object.domainId, r.object.version);
const stableBasis = (b: AssemblyBasis): string =>
  canonical(Object.fromEntries(Object.entries(b).filter(([name]) => name !== "readRef")));
const expectedVersionsCurrent = (
  command: CommitManifestRequest,
  rows: ResolvedContextVersion[],
): boolean =>
  command.expectedVersions.every((expected) =>
    rows.some(
      (row) =>
        key(rowRef(row)) === key(expected) &&
        row.state === "ACTIVE" &&
        row.object.contentHash === expected.contentHash &&
        row.stateRevision === expected.stateRevision &&
        row.accessPolicyRevision === expected.accessPolicyRevision,
    ),
  );

/** Test-only authority simulation. It is NOT a real Grant Store, FTS or SQLite transaction. */
function fixture() {
  const state = {
    permitted: true,
    basis: {
      ...identity,
      readRef: "read-one",
      admissionRef: "admission-one",
      taskRevision: "11",
      policyRevision: "12",
      authRevision: "13",
      revocationHead: "14",
      selectionDecisionId: "rules-selection-one",
      maxContentBytes: 100,
      maxCandidates: 10,
      partitions: [
        {
          sourceDomainId: "domain-one",
          authorizationRef: "grant-one",
          authorizationRevision: "15",
        },
      ],
      requiredConstraints: [ref("mandatory")],
    } as AssemblyBasis,
    found: [ref("optional")],
    rows: [source("mandatory", "required"), source("optional", "helpful")],
    calls: [] as string[],
    commits: [] as CommitManifestRequest[],
    loadedRefs: [] as ReadonlyArray<ContextVersionRef>,
    openedRequests: [] as AssemblyRequest[],
    stored: new Map<string, CommitManifestRequest>(),
    openOverride: undefined as undefined | (() => unknown),
    searchOverride: undefined as undefined | (() => unknown),
    loadOverride: undefined as undefined | (() => unknown),
    commitOverride: undefined as undefined | (() => unknown),
    readOverride: undefined as undefined | (() => unknown),
    beforeCommit: undefined as undefined | (() => void),
    afterCommit: undefined as undefined | (() => void),
    uiRevision: 0,
  };
  const ports: ContextAssemblyAuthorityPort = {
    async openAssembly(input) {
      state.calls.push("authorize");
      state.openedRequests.push(input);
      if (state.openOverride) return state.openOverride() as AssemblyBasis;
      return state.permitted ? state.basis : null;
    },
    async searchVisibleContext(basis, _query) {
      state.calls.push("search");
      Assert.equal(Object.isFrozen(basis.requiredConstraints), true);
      Assert.deepEqual(basis.partitions, state.basis.partitions);
      return state.searchOverride ? (state.searchOverride() as ContextVersionRef[]) : state.found;
    },
    async loadVisibleVersions(_basis, references) {
      state.calls.push("load");
      state.loadedRefs = references;
      return state.loadOverride
        ? (state.loadOverride() as ResolvedContextVersion[])
        : state.rows.filter((row) => references.some((r) => key(r) === key(rowRef(row))));
    },
    async commitManifest(command) {
      state.calls.push("commit");
      state.commits.push(command);
      state.beforeCommit?.();
      if (state.commitOverride) return state.commitOverride() as CommitManifestResult;
      if (!state.permitted) return { kind: "denied" };
      if (
        stableBasis(command.basis) !== stableBasis(state.basis) ||
        !expectedVersionsCurrent(command, state.rows)
      ) {
        return { kind: "stale" };
      }
      const old = state.stored.get(command.operationId);
      if (
        old &&
        (old.requestDigest !== command.requestDigest ||
          old.manifest.manifestHash !== command.manifest.manifestHash)
      ) {
        return { kind: "conflict" };
      }
      if (!old) state.stored.set(command.operationId, command);
      state.afterCommit?.();
      return {
        kind: old ? "replayed" : "committed",
        operationId: command.operationId,
        manifestId: command.manifest.manifestId,
        manifestHash: command.manifest.manifestHash,
      };
    },
    async readCurrentManifest(input) {
      state.calls.push("read-current");
      if (state.readOverride) return state.readOverride() as ContextManifest;
      const old = state.stored.get(input.operationId);
      if (
        !state.permitted ||
        !old ||
        stableBasis(old.basis) !== stableBasis(state.basis) ||
        !expectedVersionsCurrent(old, state.rows)
      )
        return null;
      if (
        Object.keys(identity).some(
          (name) =>
            input[name as keyof AssemblyIdentity] !== state.basis[name as keyof AssemblyIdentity],
        )
      )
        return null;
      return old.manifest;
    },
  };
  return { state, ports, assembler: new ContextManifestAssembler(ports) };
}
const denied =
  (code: string) =>
  (error: unknown): boolean =>
    error instanceof ContextAssemblyError && error.code === code && error.message === code;
const replay = (): ReplayManifestRequest => ({ ...identity, operationId: "operation-one" });

function withPrototype<T>(name: string, descriptor: PropertyDescriptor, run: () => T): T {
  Assert.equal(Object.hasOwn(Object.prototype, name), false);
  Object.defineProperty(Object.prototype, name, { ...descriptor, configurable: true });
  try {
    return run();
  } finally {
    Reflect.deleteProperty(Object.prototype, name);
  }
}

describe("R4-C-ASSEMBLE authority-scoped fixed-source construction", () => {
  it("authorizes before search and includes mandatory constraints absent from retrieval", async () => {
    const { state, assembler } = fixture();
    const result = await assembler.assemble(request());
    Assert.deepEqual(state.calls, ["authorize", "search", "load", "commit", "read-current"]);
    Assert.deepEqual(state.loadedRefs, [ref("mandatory"), ref("optional")]);
    Assert.equal(result.requiredConstraints.length, 1);
    Assert.equal(result.includedVersions.length, 2);
    Assert.deepEqual(state.commits[0]?.expectedVersions[0], {
      ...ref("mandatory"),
      contentHash: hashText("required"),
      stateRevision: "4",
      accessPolicyRevision: "5",
    });
    Assert.equal(Object.isFrozen(result), true);
    Assert.equal(Object.isFrozen(result.includedVersions), true);
    Assert.equal(canonical(result).includes('"content":'), false);
    Assert.equal(result.selectionDecisionId, "rules-selection-one");
  });

  it("does not run retrieval or reveal inaccessible identifiers when authorization fails", async () => {
    const { state, assembler } = fixture();
    state.permitted = false;
    await Assert.rejects(assembler.assemble(request()), denied("ACCESS_DENIED"));
    Assert.deepEqual(state.calls, ["authorize"]);
  });

  it("rejects caller-inline grants and permission flags before touching any authority port", async () => {
    const { state, assembler } = fixture();
    await Assert.rejects(
      assembler.assemble({ ...request(), grant: true, authorized: true } as never),
      denied("INVALID_INPUT"),
    );
    Assert.deepEqual(state.calls, []);
  });

  it("reports missing mandatory evidence rather than committing a partial manifest", async () => {
    const { state, assembler } = fixture();
    state.rows = state.rows.slice(1);
    await Assert.rejects(assembler.assemble(request()), denied("NEEDS_EVIDENCE"));
    Assert.equal(state.commits.length, 0);
  });

  it("checks actual source bytes against the immutable hash before inclusion", async () => {
    const { state, assembler } = fixture();
    state.rows[0] = { ...state.rows[0]!, content: "substituted" };
    await Assert.rejects(assembler.assemble(request()), denied("NEEDS_EVIDENCE"));
    Assert.equal(state.commits.length, 0);
  });

  it("uses current indexed state rather than the original immutable ACTIVE field", async () => {
    const { state, assembler } = fixture();
    state.rows[0] = { ...state.rows[0]!, state: "REVOKED" };
    Assert.equal(state.rows[0].object.validity, "ACTIVE");
    await Assert.rejects(assembler.assemble(request()), denied("NEEDS_EVIDENCE"));
    Assert.equal(state.commits.length, 0);
  });

  it("counts UTF-8 bytes and reports mandatory material exceeding budget", async () => {
    const { state, assembler } = fixture();
    state.rows[0] = source("mandatory", "中文");
    await Assert.rejects(
      assembler.assemble(request({ maxContentBytes: 5 })),
      denied("NEEDS_BUDGET"),
    );
    Assert.equal(state.commits.length, 0);
  });

  it("does not let optional ranking displace mandatory material under a tight budget", async () => {
    const { state, assembler } = fixture();
    const manifest = await assembler.assemble(request({ maxContentBytes: 8 }));
    Assert.equal(manifest.requiredConstraints.length, 1);
    Assert.equal(manifest.includedVersions.length, 1);
    Assert.equal(
      canonical((manifest.sourceSnapshot as any).excluded),
      canonical([{ ...ref("optional"), reason: "NEEDS_BUDGET" }]),
    );
    Assert.equal(state.commits[0]?.expectedVersions[0]?.contextId, "mandatory");
  });

  it("enforces the authoritative byte ceiling even if the caller asks for more", async () => {
    const { state, assembler } = fixture();
    state.basis = { ...state.basis, maxContentBytes: 7 };
    await Assert.rejects(
      assembler.assemble(request({ maxContentBytes: 100_000 })),
      denied("NEEDS_BUDGET"),
    );
    Assert.equal(state.commits.length, 0);
  });

  it("does not treat GLOBAL scope as a cross-domain read permission", async () => {
    const { state, assembler } = fixture();
    state.found = [ref("secret-global", "owner-private")];
    state.rows.push(
      source("secret-global", "secret", { scope: "GLOBAL", domainId: "owner-private" }),
    );
    await Assert.rejects(assembler.assemble(request()), denied("ACCESS_DENIED"));
    Assert.deepEqual(state.calls, ["authorize", "search"]);
  });

  it("keeps explicitly authorized cross-domain sources domain-qualified", async () => {
    const { state, assembler } = fixture();
    state.found = [ref("global-lesson", "owner-global")];
    state.basis = {
      ...state.basis,
      partitions: [
        ...state.basis.partitions,
        {
          sourceDomainId: "owner-global",
          authorizationRef: "grant-global-read",
          authorizationRevision: "3",
        },
      ],
    };
    state.rows.push(
      source("global-lesson", "lesson", { scope: "GLOBAL", domainId: "owner-global" }),
    );
    const manifest = await assembler.assemble(request());
    Assert.equal((manifest.includedVersions[1] as any).sourceDomainId, "owner-global");
    Assert.equal(manifest.domainId, "domain-one");
  });

  it("rejects duplicate retrieval references and unsolicited source rows", async () => {
    const first = fixture();
    first.state.found = [ref("optional"), ref("optional")];
    await Assert.rejects(first.assembler.assemble(request()), denied("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(first.state.commits.length, 0);
    const second = fixture();
    second.state.loadOverride = () => [
      ...second.state.rows,
      source("unsolicited", "not requested"),
    ];
    await Assert.rejects(second.assembler.assemble(request()), denied("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(second.state.commits.length, 0);
  });

  it("does not include two versions of the same Context identity", async () => {
    const { state, assembler } = fixture();
    state.found = [ref("mandatory", "domain-one", "2")];
    state.rows.push(source("mandatory", "other", { version: "2" as U64String }));
    const manifest = await assembler.assemble(request());
    Assert.equal(manifest.includedVersions.length, 1);
    Assert.equal((manifest.includedVersions[0] as any).version, "1");
    Assert.equal((manifest.sourceSnapshot as any).excluded[0].reason, "VERSION_NOT_SELECTED");
  });

  it("still loads mandatory references when optional retrieval has zero capacity", async () => {
    const { state, assembler } = fixture();
    state.basis = { ...state.basis, maxCandidates: 0 };
    state.found = [];
    const manifest = await assembler.assemble(request());
    Assert.equal(manifest.requiredConstraints.length, 1);
    Assert.deepEqual(state.loadedRefs, [ref("mandatory")]);
  });

  it("snapshots the caller request synchronously before any await", async () => {
    const { state, assembler } = fixture();
    const input = request();
    const pending = assembler.assemble(input);
    (input as any).principalId = "forged-owner";
    (input as any).query = "changed";
    await pending;
    Assert.equal(state.openedRequests[0]?.principalId, "owner-one");
    Assert.equal(state.openedRequests[0]?.query, "relevant facts");
    Assert.equal(Object.isFrozen(state.openedRequests[0]), true);
  });

  it("rejects active request and material properties without executing their getters", async () => {
    const { state, assembler } = fixture();
    let reads = 0;
    const input = request();
    Object.defineProperty(input, "principalId", {
      enumerable: true,
      get() {
        reads++;
        return "owner-one";
      },
    });
    await Assert.rejects(assembler.assemble(input), denied("INVALID_INPUT"));
    Assert.equal(reads, 0);
    Assert.deepEqual(state.calls, []);
    Object.defineProperty(state.rows[0]!, "content", {
      enumerable: true,
      get() {
        reads++;
        return "required";
      },
    });
    await Assert.rejects(assembler.assemble(request()), denied("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(reads, 0);
    Assert.equal(state.commits.length, 0);
  });

  it("rejects active array iterators and sparse references before loading sources", async () => {
    const { state, assembler } = fixture();
    let calls = 0;
    Object.defineProperty(state.found, Symbol.iterator, {
      value: function* () {
        calls++;
        yield ref("optional");
      },
    });
    await Assert.rejects(assembler.assemble(request()), denied("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(calls, 0);
    Assert.equal(state.calls.includes("load"), false);
    state.found = new Array(1);
    await Assert.rejects(assembler.assemble(request()), denied("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(state.commits.length, 0);
  });

  it("does not borrow missing request authority fields from the prototype", async () => {
    const { state, assembler } = fixture();
    const input = request() as any;
    delete input.principalId;
    const pending = withPrototype("principalId", { value: "owner-one", writable: true }, () =>
      assembler.assemble(input),
    );
    await Assert.rejects(pending, denied("INVALID_INPUT"));
    Assert.deepEqual(state.calls, []);
  });

  it("ignores inherited toJSON while preserving deterministic metadata hashes", () => {
    const value = { items: ["a", "b"], own: { enabled: true } };
    const expected = hashData(value);
    let calls = 0;
    const actual = withPrototype(
      "toJSON",
      {
        value() {
          calls++;
          return "forged";
        },
        writable: true,
      },
      () => hashData(value),
    );
    Assert.equal(actual, expected);
    Assert.equal(calls, 0);
  });

  const identityFields: ReadonlyArray<keyof AssemblyIdentity> = [
    "principalId",
    "seatId",
    "taskId",
    "sessionId",
    "domainId",
    "bindingId",
    "bindingGeneration",
    "sourceEpoch",
    "runtimeInstanceId",
  ];
  for (const field of identityFields) {
    it(`rejects an authoritative basis for a different ${field} before search`, async () => {
      const { state, assembler } = fixture();
      state.basis = {
        ...state.basis,
        [field]: field === "bindingGeneration" || field === "sourceEpoch" ? "99" : "other-identity",
      };
      await Assert.rejects(assembler.assemble(request()), denied("ACCESS_DENIED"));
      Assert.deepEqual(state.calls, ["authorize"]);
    });
  }

  const revisionFields = [
    "taskRevision",
    "policyRevision",
    "authRevision",
    "revocationHead",
  ] as const;
  for (const field of revisionFields) {
    it(`carries ${field} into the authoritative commit preconditions`, async () => {
      const { state, assembler } = fixture();
      state.beforeCommit = () => {
        state.basis = { ...state.basis, [field]: "99" };
      };
      await Assert.rejects(assembler.assemble(request()), denied("STALE_ASSEMBLY"));
      Assert.equal(state.stored.size, 0);
    });
  }

  it("carries source state and access revisions into commit preconditions", async () => {
    for (const field of ["stateRevision", "accessPolicyRevision"] as const) {
      const { state, assembler } = fixture();
      state.beforeCommit = () => {
        state.rows[0] = { ...state.rows[0]!, [field]: "99" };
      };
      await Assert.rejects(assembler.assemble(request()), denied("STALE_ASSEMBLY"));
      Assert.equal(state.stored.size, 0);
    }
  });

  it("does not invalidate assembly for an unrelated UI revision", async () => {
    const { state, assembler } = fixture();
    state.beforeCommit = () => {
      state.uiRevision++;
    };
    await assembler.assemble(request());
    Assert.equal(state.uiRevision, 1);
    Assert.equal(state.stored.size, 1);
  });

  it("rechecks permission after commit before returning any manifest", async () => {
    const { state, assembler } = fixture();
    state.afterCommit = () => {
      state.permitted = false;
    };
    await Assert.rejects(assembler.assemble(request()), denied("ACCESS_DENIED"));
    Assert.equal(state.stored.size, 1);
    Assert.equal(state.calls.at(-1), "read-current");
  });

  it("never satisfies replay from a local cache after revocation", async () => {
    const { state, assembler } = fixture();
    const first = await assembler.assemble(request());
    Assert.equal((await assembler.replay(replay())).manifestHash, first.manifestHash);
    state.permitted = false;
    await Assert.rejects(assembler.replay(replay()), denied("ACCESS_DENIED"));
    Assert.equal(state.calls.filter((c) => c === "read-current").length, 3);
  });

  it("does not disclose durable replay after a source is invalidated", async () => {
    const { state, assembler } = fixture();
    await assembler.assemble(request());
    state.rows[0] = { ...state.rows[0]!, state: "SUPERSEDED", stateRevision: "5" };
    await Assert.rejects(assembler.replay(replay()), denied("ACCESS_DENIED"));
  });

  it("rejects a corrupt replay hash and a replay for another binding", async () => {
    const { state, assembler } = fixture();
    const original = await assembler.assemble(request());
    state.readOverride = () => ({ ...original, manifestHash: hashText("different") });
    await Assert.rejects(assembler.replay(replay()), denied("AUTHORITY_PROTOCOL_ERROR"));
    const { manifestHash: _hash, ...body } = original;
    const changed = { ...body, bindingGeneration: "99" as U64String };
    state.readOverride = () => ({ ...changed, manifestHash: hashData(changed) });
    await Assert.rejects(assembler.replay(replay()), denied("ACCESS_DENIED"));
  });

  it("quarantines unknown commit responses without retrying a possible durable write", async () => {
    const { state, assembler } = fixture();
    state.commitOverride = () => ({ kind: "future-success" });
    await Assert.rejects(assembler.assemble(request()), denied("COMMIT_OUTCOME_UNKNOWN"));
    Assert.equal(state.commits.length, 1);
    Assert.equal(state.calls.includes("read-current"), false);
  });

  it("rejects active commit receipts without invoking getters", async () => {
    const { state, assembler } = fixture();
    let reads = 0;
    state.commitOverride = () => ({
      get kind() {
        reads++;
        return "committed";
      },
    });
    await Assert.rejects(assembler.assemble(request()), denied("COMMIT_OUTCOME_UNKNOWN"));
    Assert.equal(reads, 0);
    Assert.equal(state.commits.length, 1);
  });

  it("does not overwrite an operation with a different semantic request", async () => {
    const { state, assembler } = fixture();
    const original = await assembler.assemble(request());
    await Assert.rejects(
      assembler.assemble(request({ query: "different query" })),
      denied("OPERATION_CONFLICT"),
    );
    Assert.equal(state.stored.get("operation-one")?.manifest.manifestHash, original.manifestHash);
  });

  it("does not make ephemeral read references part of the stable manifest digest", async () => {
    const { state, assembler } = fixture();
    const original = await assembler.assemble(request());
    state.basis = { ...state.basis, readRef: "read-two" };
    const repeated = await assembler.assemble(request());
    Assert.equal(repeated.manifestHash, original.manifestHash);
    Assert.equal(state.stored.size, 1);
  });

  it("captures host methods and has no fallback when the authority fails", async () => {
    const { state, ports, assembler } = fixture();
    ports.openAssembly = async () => {
      throw new Error("replaced");
    };
    await assembler.assemble(request());
    Assert.equal(state.stored.size, 1);
    const other = fixture();
    other.state.openOverride = () => {
      throw new Error("private secret detail");
    };
    await Assert.rejects(other.assembler.assemble(request()), denied("AUTHORITY_UNAVAILABLE"));
    Assert.deepEqual(other.state.calls, ["authorize"]);
  });
});
