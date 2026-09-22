import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import { ContextAssemblyError, type AssemblyIdentity } from "../assembly/model.ts";
import { canonical, hashText } from "../assembly/passive.ts";
import {
  ContextViewMaintenance,
  type MaintenanceBatch,
  type MaintenanceCommand,
  type MaintenanceCommitResult,
  type MaintenanceReceipt,
  type MaintenanceRequest,
  type ContextMaintenanceAuthorityPort,
  type ReadProjectionInvalidation,
} from "./maintenance.ts";

const identity: AssemblyIdentity = {
  principalId: "owner",
  seatId: "seat",
  taskId: "task",
  sessionId: "session",
  domainId: "domain",
  bindingId: "binding",
  bindingGeneration: "3",
  sourceEpoch: "4",
  runtimeInstanceId: "runtime",
};
const request = (overrides: Partial<MaintenanceRequest> = {}): MaintenanceRequest => ({
  ...identity,
  operationId: "maintenance-one",
  triggerId: "trigger-one",
  maxItems: 10,
  ...overrides,
});
const replay = () => ({ ...identity, operationId: "maintenance-one" });
const invalidation = (contextId = "context"): ReadProjectionInvalidation => ({
  sourceDomainId: "domain",
  contextId,
  version: "1",
  currentState: "STALE",
  stateRevision: "5",
  accessPolicyRevision: "6",
  projectionRevision: "7",
  reason: "SOURCE_STATE_CHANGED",
});
const normalized = (basis: MaintenanceBatch): string => {
  const { readRef: _readRef, ...stable } = basis;
  return canonical({
    ...stable,
    invalidations: [...stable.invalidations].sort((a, b) =>
      canonical([a.sourceDomainId, a.contextId, a.version]) <
      canonical([b.sourceDomainId, b.contextId, b.version])
        ? -1
        : 1,
    ),
  });
};
const error =
  (code: string) =>
  (value: unknown): boolean =>
    value instanceof ContextAssemblyError && value.code === code && value.message === code;

/** No real SQLite/FTS/grants here: this fake observes the coordinator protocol only. */
function fixture() {
  const state = {
    permitted: true,
    basis: {
      ...identity,
      triggerId: "trigger-one",
      readRef: "private-read",
      batchId: "batch",
      batchRevision: "8",
      graphRevision: "9",
      policyRevision: "10",
      authRevision: "11",
      revocationHead: "12",
      maxItems: 10,
      complete: true,
      cause: {
        sourceDomainId: "secret-origin-domain",
        sourceOperationId: "secret-origin-operation",
        fingerprint: hashText("native-receipt"),
      },
      invalidations: [invalidation()],
    } as MaintenanceBatch,
    calls: [] as string[],
    commands: [] as MaintenanceCommand[],
    opened: [] as MaintenanceRequest[],
    rows: new Map<string, { command: MaintenanceCommand; receipt: MaintenanceReceipt }>(),
    originalContent: Object.freeze({
      content: "original bytes",
      hash: hashText("original bytes"),
      state: "ACTIVE",
    }),
    beforeCommit: undefined as undefined | (() => void),
    afterCommit: undefined as undefined | (() => void),
    openOverride: undefined as undefined | (() => unknown),
    commitOverride: undefined as undefined | (() => unknown),
    readOverride: undefined as undefined | (() => unknown),
  };
  const port: ContextMaintenanceAuthorityPort = {
    async openMaintenance(input) {
      state.calls.push("authorize-and-read");
      state.opened.push(input);
      if (state.openOverride) return state.openOverride() as MaintenanceBatch;
      return state.permitted ? state.basis : null;
    },
    async commitReadProjectionInvalidation(command) {
      state.calls.push("commit");
      state.commands.push(command);
      state.beforeCommit?.();
      if (state.commitOverride) return state.commitOverride() as MaintenanceCommitResult;
      if (!state.permitted) return { kind: "denied" };
      if (normalized(command.basis) !== normalized(state.basis)) return { kind: "stale" };
      const old = state.rows.get(command.operationId);
      if (old && old.receipt.semanticDigest !== command.semanticDigest) return { kind: "conflict" };
      const receipt = old?.receipt ?? {
        operationId: command.operationId,
        receiptId: "receipt-one",
        semanticDigest: command.semanticDigest,
        invalidatedViews: command.invalidations.length,
      };
      if (!old) state.rows.set(command.operationId, { command, receipt });
      state.afterCommit?.();
      return { kind: old ? "replayed" : "committed", ...receipt };
    },
    async readCurrentMaintenanceReceipt(input) {
      state.calls.push("read-current");
      if (state.readOverride) return state.readOverride() as MaintenanceReceipt;
      if (
        !state.permitted ||
        Object.keys(identity).some(
          (key) =>
            input[key as keyof AssemblyIdentity] !== state.basis[key as keyof AssemblyIdentity],
        )
      )
        return null;
      return state.rows.get(input.operationId)?.receipt ?? null;
    },
  };
  return { state, port, maintenance: new ContextViewMaintenance(port) };
}

describe("R4-C-ASSEMBLE authoritative read-projection maintenance", () => {
  it("uses one complete authority batch and never changes original content or source state", async () => {
    const { state, maintenance } = fixture();
    const original = canonical(state.originalContent);
    const result = await maintenance.maintain(request());
    Assert.deepEqual(state.calls, ["authorize-and-read", "commit", "read-current"]);
    Assert.equal(state.commands[0]?.operation, "InvalidateContextReadProjections");
    Assert.equal(state.commands[0]?.invalidations.length, 1);
    Assert.equal(result.invalidatedViews, 1);
    Assert.equal(canonical(state.originalContent), original);
    Assert.equal(Object.isFrozen(state.commands[0]?.basis.invalidations), true);
    Assert.equal(Object.isFrozen(state.commands[0]?.invalidations[0]), true);
  });
  it("does not write when authorization fails", async () => {
    const { state, maintenance } = fixture();
    state.permitted = false;
    await Assert.rejects(maintenance.maintain(request()), error("ACCESS_DENIED"));
    Assert.deepEqual(state.calls, ["authorize-and-read"]);
  });
  it("does not accept caller-inline invalidations or permission flags", async () => {
    const { state, maintenance } = fixture();
    await Assert.rejects(
      maintenance.maintain({ ...request(), invalidations: [], allowed: true } as never),
      error("INVALID_INPUT"),
    );
    Assert.deepEqual(state.calls, []);
  });
  it("refuses incomplete authority batches instead of treating a page as the full cascade", async () => {
    const { state, maintenance } = fixture();
    state.openOverride = () => ({ ...state.basis, complete: false });
    await Assert.rejects(maintenance.maintain(request()), error("NEEDS_EVIDENCE"));
    Assert.equal(state.commands.length, 0);
  });
  it("does not silently truncate a mandatory invalidation batch to the caller budget", async () => {
    const { state, maintenance } = fixture();
    await Assert.rejects(maintenance.maintain(request({ maxItems: 0 })), error("NEEDS_BUDGET"));
    Assert.equal(state.commands.length, 0);
  });
  it("does not allow the caller to raise the authority batch ceiling", async () => {
    const { state, maintenance } = fixture();
    state.basis = { ...state.basis, maxItems: 0 };
    await Assert.rejects(maintenance.maintain(request({ maxItems: 1000 })), error("NEEDS_BUDGET"));
    Assert.equal(state.commands.length, 0);
  });
  it("evicts revoked reader projections without declaring the underlying ACTIVE content revoked", async () => {
    const { state, maintenance } = fixture();
    state.basis = {
      ...state.basis,
      invalidations: [
        { ...invalidation(), currentState: "ACTIVE", reason: "READ_PERMISSION_REVOKED" },
      ],
    };
    await maintenance.maintain(request());
    Assert.equal(state.commands[0]?.invalidations[0]?.currentState, "ACTIVE");
    Assert.equal(state.originalContent.state, "ACTIVE");
  });
  it("rejects contradictory source-state invalidation rather than inventing a lifecycle transition", async () => {
    const { state, maintenance } = fixture();
    state.basis = {
      ...state.basis,
      invalidations: [{ ...invalidation(), currentState: "ACTIVE" }],
    };
    await Assert.rejects(maintenance.maintain(request()), error("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(state.commands.length, 0);
  });
  it("rejects unknown invalidation reasons and source-deletion commands", async () => {
    const { state, maintenance } = fixture();
    state.openOverride = () => ({
      ...state.basis,
      invalidations: [{ ...invalidation(), reason: "DELETE_SOURCE" }],
    });
    await Assert.rejects(maintenance.maintain(request()), error("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(state.commands.length, 0);
  });
  it("rejects duplicate or conflicting entries for one immutable Context version", async () => {
    const { state, maintenance } = fixture();
    state.basis = {
      ...state.basis,
      invalidations: [invalidation(), { ...invalidation(), stateRevision: "99" }],
    };
    await Assert.rejects(maintenance.maintain(request()), error("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(state.commands.length, 0);
  });
  it("normalizes batch order without changing operation identity or mutating caller arrays", async () => {
    const { state, maintenance } = fixture();
    state.basis = { ...state.basis, invalidations: [invalidation("z"), invalidation("a")] };
    const first = await maintenance.maintain(request());
    Assert.deepEqual(
      state.basis.invalidations.map((x) => x.contextId),
      ["z", "a"],
    );
    Assert.deepEqual(
      state.commands[0]?.invalidations.map((x) => x.contextId),
      ["a", "z"],
    );
    state.basis = {
      ...state.basis,
      invalidations: [...state.basis.invalidations].reverse(),
      readRef: "new-private-read",
    };
    Assert.equal((await maintenance.maintain(request())).semanticDigest, first.semanticDigest);
    Assert.equal(state.rows.size, 1);
  });
  it("rejects a target from a different domain before submitting a maintenance command", async () => {
    const { state, maintenance } = fixture();
    state.basis = {
      ...state.basis,
      invalidations: [{ ...invalidation(), sourceDomainId: "other-domain" }],
    };
    await Assert.rejects(maintenance.maintain(request()), error("ACCESS_DENIED"));
    Assert.equal(state.commands.length, 0);
  });
  it("does not disclose private origin, read references or affected Context inventory in the receipt", async () => {
    const { maintenance } = fixture();
    const result = await maintenance.maintain(request());
    Assert.deepEqual(Object.keys(result).sort(), [
      "invalidatedViews",
      "operationId",
      "receiptId",
      "semanticDigest",
    ]);
    Assert.equal(canonical(result).includes("secret-origin"), false);
    Assert.equal(canonical(result).includes("private-read"), false);
    Assert.equal(canonical(result).includes("context"), false);
  });
  it("snapshots a maintenance request before the first await", async () => {
    const { state, maintenance } = fixture();
    const input = request();
    const pending = maintenance.maintain(input);
    (input as any).principalId = "forged";
    (input as any).triggerId = "changed";
    await pending;
    Assert.equal(state.opened[0]?.principalId, "owner");
    Assert.equal(state.opened[0]?.triggerId, "trigger-one");
    Assert.equal(Object.isFrozen(state.opened[0]), true);
  });
  it("rejects active request and invalidation properties without invoking their getters", async () => {
    const { state, maintenance } = fixture();
    let reads = 0;
    const input = request();
    Object.defineProperty(input, "principalId", {
      enumerable: true,
      get() {
        reads++;
        return "owner";
      },
    });
    await Assert.rejects(maintenance.maintain(input), error("INVALID_INPUT"));
    Assert.equal(reads, 0);
    Assert.equal(state.calls.length, 0);
    Object.defineProperty(state.basis.invalidations[0]!, "stateRevision", {
      enumerable: true,
      get() {
        reads++;
        return "5";
      },
    });
    await Assert.rejects(maintenance.maintain(request()), error("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(reads, 0);
    Assert.equal(state.commands.length, 0);
  });
  it("rejects proxy requests without invoking proxy traps", async () => {
    const { state, maintenance } = fixture();
    let traps = 0;
    const input = new Proxy(request(), {
      get(target, property) {
        traps++;
        return Reflect.get(target, property);
      },
      getPrototypeOf(target) {
        traps++;
        return Reflect.getPrototypeOf(target);
      },
    });
    await Assert.rejects(maintenance.maintain(input), error("INVALID_INPUT"));
    Assert.equal(traps, 0);
    Assert.equal(state.calls.length, 0);
  });
  it("rejects sparse or active invalidation arrays rather than losing cascade entries", async () => {
    const { state, maintenance } = fixture();
    let calls = 0;
    const invalidations = [invalidation()];
    Object.defineProperty(invalidations, Symbol.iterator, {
      value: function* () {
        calls++;
      },
    });
    state.basis = { ...state.basis, invalidations };
    await Assert.rejects(maintenance.maintain(request()), error("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(calls, 0);
    state.basis = { ...state.basis, invalidations: new Array(1) };
    await Assert.rejects(maintenance.maintain(request()), error("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(state.commands.length, 0);
  });
  it("captures host methods and suppresses private backend error messages", async () => {
    const { state, port, maintenance } = fixture();
    port.openMaintenance = async () => {
      throw new Error("replaced");
    };
    await maintenance.maintain(request());
    Assert.equal(state.rows.size, 1);
    const other = fixture();
    other.state.openOverride = () => {
      throw new Error("secret backend content");
    };
    await Assert.rejects(other.maintenance.maintain(request()), error("AUTHORITY_UNAVAILABLE"));
    Assert.equal(other.state.commands.length, 0);
  });
  it("rejects different semantics under the same maintenance operation identity", async () => {
    const { state, maintenance } = fixture();
    const first = await maintenance.maintain(request());
    await Assert.rejects(
      maintenance.maintain(request({ maxItems: 9 })),
      error("OPERATION_CONFLICT"),
    );
    Assert.equal(state.rows.get("maintenance-one")?.receipt.semanticDigest, first.semanticDigest);
  });
  it("does not retry unknown commit results or call them successful", async () => {
    const { state, maintenance } = fixture();
    state.commitOverride = () => ({ kind: "future-success" });
    await Assert.rejects(maintenance.maintain(request()), error("COMMIT_OUTCOME_UNKNOWN"));
    Assert.equal(state.commands.length, 1);
    Assert.equal(state.calls.includes("read-current"), false);
  });
  it("rejects contradictory committed counts and identities instead of returning partial success", async () => {
    for (const changes of [
      { invalidatedViews: 0 },
      { operationId: "other-operation" },
      { semanticDigest: hashText("wrong") },
    ]) {
      const { state, maintenance } = fixture();
      state.commitOverride = () => ({
        kind: "committed",
        operationId: "maintenance-one",
        receiptId: "receipt-one",
        semanticDigest: state.commands[0]!.semanticDigest,
        invalidatedViews: 1,
        ...changes,
      });
      await Assert.rejects(maintenance.maintain(request()), error("COMMIT_OUTCOME_UNKNOWN"));
      Assert.equal(state.commands.length, 1);
    }
  });
  it("reauthorizes after commit before disclosing its durable receipt", async () => {
    const { state, maintenance } = fixture();
    state.afterCommit = () => {
      state.permitted = false;
    };
    await Assert.rejects(maintenance.maintain(request()), error("ACCESS_DENIED"));
    Assert.equal(state.rows.size, 1);
    Assert.equal(state.calls.at(-1), "read-current");
  });
  it("always reenters authority on replay and denies a revoked caller", async () => {
    const { state, maintenance } = fixture();
    await maintenance.maintain(request());
    await maintenance.replay(replay());
    state.permitted = false;
    await Assert.rejects(maintenance.replay(replay()), error("ACCESS_DENIED"));
    Assert.equal(state.calls.filter((x) => x === "read-current").length, 3);
  });
  it("refuses a replay receipt with extra private fields or a mismatched operation", async () => {
    const { state, maintenance } = fixture();
    const result = await maintenance.maintain(request());
    state.readOverride = () => ({ ...result, privateSource: "secret" });
    await Assert.rejects(maintenance.replay(replay()), error("AUTHORITY_PROTOCOL_ERROR"));
    state.readOverride = () => ({ ...result, operationId: "other" });
    await Assert.rejects(maintenance.replay(replay()), error("AUTHORITY_PROTOCOL_ERROR"));
  });
  it("rejects a changed binding when reading a previous maintenance receipt", async () => {
    const { state, maintenance } = fixture();
    await maintenance.maintain(request());
    state.basis = { ...state.basis, bindingGeneration: "99" };
    await Assert.rejects(maintenance.replay(replay()), error("ACCESS_DENIED"));
  });
  it("rejects version overflow in the invalidation tuple", async () => {
    const { state, maintenance } = fixture();
    state.basis = {
      ...state.basis,
      invalidations: [{ ...invalidation(), version: "18446744073709551616" }],
    };
    await Assert.rejects(maintenance.maintain(request()), error("AUTHORITY_PROTOCOL_ERROR"));
    Assert.equal(state.commands.length, 0);
  });
  const fields: ReadonlyArray<keyof AssemblyIdentity> = [
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
  for (const field of fields) {
    it(`rejects a maintenance batch for a different ${field}`, async () => {
      const { state, maintenance } = fixture();
      state.basis = {
        ...state.basis,
        [field]: field === "bindingGeneration" || field === "sourceEpoch" ? "99" : "different",
      };
      await Assert.rejects(maintenance.maintain(request()), error("ACCESS_DENIED"));
      Assert.equal(state.commands.length, 0);
    });
  }
  for (const field of [
    "batchRevision",
    "graphRevision",
    "policyRevision",
    "authRevision",
    "revocationHead",
  ] as const) {
    it(`carries ${field} into the authoritative maintenance preconditions`, async () => {
      const { state, maintenance } = fixture();
      state.beforeCommit = () => {
        state.basis = { ...state.basis, [field]: "99" };
      };
      await Assert.rejects(maintenance.maintain(request()), error("STALE_ASSEMBLY"));
      Assert.equal(state.rows.size, 0);
    });
  }
  for (const field of ["stateRevision", "accessPolicyRevision", "projectionRevision"] as const) {
    it(`carries per-source ${field} into the authoritative maintenance preconditions`, async () => {
      const { state, maintenance } = fixture();
      state.beforeCommit = () => {
        state.basis = { ...state.basis, invalidations: [{ ...invalidation(), [field]: "99" }] };
      };
      await Assert.rejects(maintenance.maintain(request()), error("STALE_ASSEMBLY"));
      Assert.equal(state.rows.size, 0);
    });
  }
});
