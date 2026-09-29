import { canonicalJson } from "../strictJson.ts";
import type { JsonObject, JsonValue } from "../model.ts";
import { decodeV37Request, encodeV37Receipt, V37_SCHEMA, type V37Port, type V37Receipt, type V37Request, type V37TrustedCaller } from "./protocol.ts";

interface Side { revision: bigint; state: "ACTIVE" | "ARCHIVED" | "DELETED"; sourceCursor: string; }
interface Gate { revision: bigint; state: "SUBMITTED" | "PASSED" | "REJECTED" | "ADVANCED"; reason?: string; }
interface Trigger { revision: bigint; state: "REGISTERED" | "CANCELLED"; }
interface Worktree { revision: bigint; state: "CREATED" | "REGISTERED" | "MERGED" | "MERGE_UNKNOWN" | "CLEANED";
  repositoryId: string; seatId: string; mergeReason?: string; }
interface Prior { readonly request: string; readonly receipt: V37Receipt; }

export class V37M2FakeStore {
  readonly side = new Map<string, Side>();
  readonly gates = new Map<string, Gate>();
  readonly triggers = new Map<string, Trigger>();
  readonly worktrees = new Map<string, Worktree>();
  readonly replies = new Map<string, Prior>();
}

export interface V37M2FakeOptions {
  readonly caller: () => V37TrustedCaller | null;
  readonly granted: (caller: V37TrustedCaller, request: V37Request) => boolean;
  readonly verifyRepository?: (repositoryId: string) => boolean;
  readonly verifyIsolation?: (worktreeId: string) => boolean;
  readonly stopConfirmed?: (worktreeId: string) => boolean;
  readonly activeAdmissions?: (worktreeId: string) => number;
  readonly mergeGranted?: (seatId: string) => boolean;
  readonly performMerge?: (worktreeId: string) => "merged" | "unknown" | "failed";
  readonly classify?: (worktreeId: string) => "SINGLE" | "MIXED" | null;
  readonly removeSideLedgerTier?: (sideId: string) => boolean;
  readonly scheduleTrigger?: (triggerId: string) => boolean;
  readonly cancelTrigger?: (triggerId: string) => boolean;
}

function field(payload: JsonObject, name: string): string {
  const value = payload[name];
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    throw new Error(`V37_M2_INVALID: payload.${name}`);
  }
  return value;
}

/** M2 test fake. Native identity, stop, repository and grant facts are injected observations. */
export class V37M2FakePort implements V37Port {
  readonly store: V37M2FakeStore;
  readonly options: V37M2FakeOptions;
  constructor(store: V37M2FakeStore, options: V37M2FakeOptions) {
    this.store = store; this.options = options;
  }

  async execute(bytes: Uint8Array): Promise<Uint8Array> {
    const request = decodeV37Request(bytes);
    if (!["K-SIDE", "K-POLICY", "K-WORKTREE"].includes(request.family)) {
      throw new Error("V37_M2_UNSUPPORTED_FAMILY");
    }
    const key = `${request.domainId}:${request.targetId}`;
    const side = request.family === "K-SIDE" ? this.store.side.get(key) : undefined;
    const gate = request.family === "K-POLICY" ? this.store.gates.get(key) : undefined;
    const trigger = request.family === "K-POLICY" ? this.store.triggers.get(key) : undefined;
    const worktree = request.family === "K-WORKTREE" ? this.store.worktrees.get(key) : undefined;
    const current = side?.revision ?? gate?.revision ?? trigger?.revision ?? worktree?.revision ?? 0n;
    const reply = (status: V37Receipt["status"], next = current,
      result: JsonObject = {}): V37Receipt => ({ schema: V37_SCHEMA, family: request.family,
      operation: request.operation, requestId: request.requestId, targetId: request.targetId,
      status, previousRevision: current.toString(), revision: next.toString(), result });
    const caller = this.options.caller();
    if (caller === null || caller.domainId !== request.domainId ||
        !this.options.granted(caller, request)) return encodeV37Receipt(reply("DENIED"));
    const replayKey = `${request.family}:${request.domainId}:${request.requestId}`;
    const canonical = canonicalJson(request as unknown as JsonValue);
    const prior = this.store.replies.get(replayKey);
    if (prior) return encodeV37Receipt(prior.request === canonical
      ? { ...prior.receipt, status: prior.receipt.status === "UNKNOWN" ? "UNKNOWN" : "REPLAYED" }
      : reply("CONFLICT"));
    const commit = (receipt: V37Receipt): Uint8Array => {
      this.store.replies.set(replayKey, { request: canonical, receipt });
      return encodeV37Receipt(receipt);
    };
    if (BigInt(request.expectedRevision) !== current) return encodeV37Receipt(reply("STALE"));

    if (request.family === "K-SIDE") {
      if (request.operation === "create") {
        if (side) return encodeV37Receipt(reply("CONFLICT"));
        const sourceCursor = field(request.payload, "sourceCursor");
        if (!/^(?:0|[1-9][0-9]*)$/u.test(sourceCursor)) throw new Error("V37_M2_INVALID: sourceCursor");
        this.store.side.set(key, { revision: 1n, state: "ACTIVE", sourceCursor });
        return commit(reply("APPLIED", 1n, { state: "ACTIVE", sourceCursor }));
      }
      if (!side || side.state === "DELETED") return encodeV37Receipt(reply("CONFLICT"));
      if (request.operation === "pending-delta" || request.operation === "read-thread") {
        return commit(reply("APPLIED", current, { state: side.state,
          sourceCursor: side.sourceCursor, mainContextCopy: false }));
      }
      if (request.operation === "resume") {
        if (side.state !== "ACTIVE") return encodeV37Receipt(reply("CONFLICT"));
      } else if (request.operation === "archive") {
        if (side.state !== "ACTIVE") return encodeV37Receipt(reply("CONFLICT"));
        side.state = "ARCHIVED";
      } else if (request.operation === "restore") {
        if (side.state !== "ARCHIVED") return encodeV37Receipt(reply("CONFLICT"));
        side.state = "ACTIVE";
      } else if (request.operation === "delete") {
        if (!this.options.removeSideLedgerTier?.(request.targetId)) return encodeV37Receipt(reply("DENIED"));
        side.state = "DELETED";
      } else return encodeV37Receipt(reply("UNSUPPORTED"));
      side.revision += 1n;
      return commit(reply("APPLIED", side.revision, { state: side.state }));
    }

    if (request.family === "K-POLICY") {
      if (request.operation === "call-permission-table") {
        return encodeV37Receipt(reply("UNSUPPORTED"));
      }
      if (request.operation.startsWith("trigger-")) {
        if (request.operation === "trigger-register") {
          if (trigger) return encodeV37Receipt(reply("CONFLICT"));
          field(request.payload, "eventRef");
          if (!this.options.scheduleTrigger?.(request.targetId)) return encodeV37Receipt(reply("DENIED"));
          this.store.triggers.set(key, { revision: 1n, state: "REGISTERED" });
          return commit(reply("APPLIED", 1n, { state: "REGISTERED" }));
        }
        if (!trigger) return encodeV37Receipt(reply("CONFLICT"));
        if (request.operation === "trigger-recover") {
          if (trigger.state !== "REGISTERED") return encodeV37Receipt(reply("CONFLICT"));
          trigger.revision += 1n;
          return commit(reply("APPLIED", trigger.revision, { state: trigger.state }));
        }
        if (request.operation === "trigger-cancel") {
          if (trigger.state !== "REGISTERED") return encodeV37Receipt(reply("CONFLICT"));
          if (!this.options.cancelTrigger?.(request.targetId)) return encodeV37Receipt(reply("DENIED"));
          trigger.state = "CANCELLED"; trigger.revision += 1n;
          return commit(reply("APPLIED", trigger.revision, { state: trigger.state }));
        }
      }
      if (request.operation === "gate-submit") {
        if (gate) return encodeV37Receipt(reply("CONFLICT"));
        this.store.gates.set(key, { revision: 1n, state: "SUBMITTED" });
        return commit(reply("APPLIED", 1n, { state: "SUBMITTED" }));
      }
      if (!gate) return encodeV37Receipt(reply("CONFLICT"));
      if (request.operation === "gate-decide") {
        if (gate.state !== "SUBMITTED") return encodeV37Receipt(reply("CONFLICT"));
        const decision = field(request.payload, "decision");
        if (decision !== "PASS" && decision !== "REJECT") throw new Error("V37_M2_INVALID: decision");
        if (decision === "REJECT") gate.reason = field(request.payload, "reason");
        gate.state = decision === "PASS" ? "PASSED" : "REJECTED";
      } else if (request.operation === "stage-transition") {
        if (gate.state !== "PASSED") return encodeV37Receipt(reply("CONFLICT"));
        gate.state = "ADVANCED";
      } else return encodeV37Receipt(reply("UNSUPPORTED"));
      gate.revision += 1n;
      return commit(reply("APPLIED", gate.revision,
        { state: gate.state, ...(gate.reason ? { reason: gate.reason } : {}) }));
    }

    if (request.operation === "create") {
      if (worktree) return encodeV37Receipt(reply("CONFLICT"));
      const repositoryId = field(request.payload, "repositoryId");
      const seatId = field(request.payload, "seatId");
      if (!this.options.verifyRepository?.(repositoryId) ||
          !this.options.verifyIsolation?.(request.targetId)) return encodeV37Receipt(reply("DENIED"));
      this.store.worktrees.set(key, { revision: 1n, state: "CREATED", repositoryId, seatId });
      return commit(reply("APPLIED", 1n, { state: "CREATED" }));
    }
    if (!worktree || worktree.state === "CLEANED") return encodeV37Receipt(reply("CONFLICT"));
    if (request.operation === "classify-single-or-mixed" || request.operation === "graph-query") {
      const classification = this.options.classify?.(request.targetId);
      if (!classification) return encodeV37Receipt(reply("UNSUPPORTED"));
      return commit(reply("APPLIED", current, { classification,
        repositoryId: worktree.repositoryId, seatId: worktree.seatId, state: worktree.state,
        mergeReason: worktree.mergeReason ?? null }));
    }
    if (request.operation === "register") {
      if (worktree.state !== "CREATED") return encodeV37Receipt(reply("CONFLICT"));
      worktree.state = "REGISTERED";
    } else if (request.operation === "merge") {
      if (worktree.state !== "REGISTERED") return encodeV37Receipt(reply("CONFLICT"));
      if (!this.options.mergeGranted?.(worktree.seatId) || caller.seatId !== worktree.seatId) {
        return encodeV37Receipt(reply("DENIED"));
      }
      if (field(request.payload, "decision") !== "MERGE") throw new Error("V37_M2_INVALID: merge decision");
      const reason = field(request.payload, "reason");
      const outcome = this.options.performMerge?.(request.targetId);
      if (outcome === undefined) return encodeV37Receipt(reply("UNSUPPORTED"));
      if (outcome === "failed") return encodeV37Receipt(reply("FAILED"));
      worktree.mergeReason = reason;
      worktree.state = outcome === "merged" ? "MERGED" : "MERGE_UNKNOWN";
      worktree.revision += 1n;
      return commit(reply(outcome === "merged" ? "APPLIED" : "UNKNOWN", worktree.revision,
        { state: worktree.state, mergeReason: reason }));
    } else if (request.operation === "cleanup") {
      if (!this.options.stopConfirmed?.(request.targetId) ||
          this.options.activeAdmissions?.(request.targetId) !== 0) {
        return encodeV37Receipt(reply("DENIED"));
      }
      worktree.state = "CLEANED";
    } else return encodeV37Receipt(reply("UNSUPPORTED"));
    worktree.revision += 1n;
    return commit(reply("APPLIED", worktree.revision,
      { state: worktree.state, mergeReason: worktree.mergeReason ?? null }));
  }
}
