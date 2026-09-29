import { canonicalJson } from "../strictJson.ts";
import { decodeV37Request, encodeV37Receipt, V37_SCHEMA, type V37Port, type V37Receipt, type V37Request, type V37TrustedCaller } from "./protocol.ts";

type SessionState = "RESERVED" | "COMMITTED" | "RUNNING" | "STOPPING" | "STOPPED";
interface Session { state: SessionState; revision: bigint; generation: string; }
type InboxState = "PENDING" | "PREPARED" | "UNKNOWN" | "DELIVERED" | "CANCELLED";
interface Inbox { state: InboxState; revision: bigint; generation: string; body: string; }
interface LedgerEvent { sourceId: string; cursor: bigint; scope: "GLOBAL" | "PROJECT" | "SIDE"; domain: string; }
interface StoredReply { request: string; receipt: V37Receipt; }

/** Test store survives fake-port reconstruction; this is not production persistence. */
export class V37CoreFakeStore {
  readonly sessions = new Map<string, Session>();
  readonly inbox = new Map<string, Inbox>();
  readonly ledger: LedgerEvent[] = [];
  readonly replies = new Map<string, StoredReply>();
  readonly sourceIds = new Set<string>();
  ledgerCursor = 0n;
  epoch = "1";
}

export interface V37CoreFakeOptions {
  readonly caller: () => V37TrustedCaller | null;
  readonly granted: (caller: V37TrustedCaller, request: V37Request) => boolean;
  readonly prepareDelivery?: (messageId: string) => Promise<void>;
  readonly beginCommitted?: (messageId: string) => Promise<void>;
  readonly completeDelivery?: (messageId: string) => Promise<"completed" | "unknown">;
  readonly verifyPinnedBinary?: (digest: string) => boolean;
  readonly verifyStopProof?: (proofId: string) => boolean;
}

function field(payload: V37Request["payload"], name: string): string {
  const value = payload[name];
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    throw new Error(`V37_CORE_INVALID: payload.${name}`);
  }
  return value;
}

function numeric(value: string): bigint {
  if (!/^(?:0|[1-9][0-9]*)$/u.test(value)) throw new Error("V37_CORE_INVALID: cursor");
  return BigInt(value);
}

export class V37CoreFakePort implements V37Port {
  readonly store: V37CoreFakeStore;
  readonly options: V37CoreFakeOptions;
  constructor(store: V37CoreFakeStore, options: V37CoreFakeOptions) {
    this.store = store;
    this.options = options;
  }

  async execute(bytes: Uint8Array): Promise<Uint8Array> {
    const request = decodeV37Request(bytes);
    if (!["K-SESSION", "K-INBOX", "K-LEDGER"].includes(request.family)) {
      throw new Error("V37_CORE_UNSUPPORTED_FAMILY");
    }
    const storageKey = `${request.family}:${request.domainId}:${request.targetId}`;
    const current = request.family === "K-SESSION" ? this.store.sessions.get(storageKey)?.revision ?? 0n
      : request.family === "K-INBOX" ? this.store.inbox.get(storageKey)?.revision ?? 0n
      : this.store.ledgerCursor;
    const reply = (status: V37Receipt["status"], previous: bigint, next: bigint,
      result: V37Receipt["result"] = {}): V37Receipt => ({
      schema: V37_SCHEMA, family: request.family, operation: request.operation,
      requestId: request.requestId, targetId: request.targetId, status,
      previousRevision: previous.toString(), revision: next.toString(), result,
    });
    const denied = () => {
      const caller = this.options.caller();
      return caller === null || caller.domainId !== request.domainId ||
        !this.options.granted(caller, request);
    };
    if (denied()) return encodeV37Receipt(reply("DENIED", current, current));
    const replayKey = `${request.family}:${request.domainId}:${request.requestId}`;
    const canonical = canonicalJson(request as unknown as Parameters<typeof canonicalJson>[0]);
    const prior = this.store.replies.get(replayKey);
    if (prior !== undefined) return encodeV37Receipt(prior.request === canonical
      ? { ...prior.receipt, status: prior.receipt.status === "UNKNOWN" ? "UNKNOWN" : "REPLAYED" }
      : reply("CONFLICT", current, current));
    const committed = (receipt: V37Receipt): Uint8Array => {
      this.store.replies.set(replayKey, { request: canonical, receipt });
      return encodeV37Receipt(receipt);
    };

    if (request.family === "K-SESSION") {
      const session = this.store.sessions.get(storageKey);
      if (BigInt(request.expectedRevision) !== current) return encodeV37Receipt(reply("STALE", current, current));
      const generation = field(request.payload, "generation");
      const transitions: Record<string, readonly [SessionState | null, SessionState]> = {
        "admission-reserve": [null, "RESERVED"],
        "admission-commit": ["RESERVED", "COMMITTED"],
        open: ["COMMITTED", "RUNNING"],
        stop: ["RUNNING", "STOPPING"],
        "exit-and-stop-receipt": ["STOPPING", "STOPPED"],
      };
      const transition = transitions[request.operation];
      if (transition === undefined) return encodeV37Receipt(reply("UNSUPPORTED", current, current));
      if ((session?.state ?? null) !== transition[0] || (session && session.generation !== generation)) {
        return encodeV37Receipt(reply("CONFLICT", current, current));
      }
      if (request.operation === "open" &&
          !this.options.verifyPinnedBinary?.(field(request.payload, "pinnedBinaryDigest"))) {
        return encodeV37Receipt(reply("DENIED", current, current));
      }
      if (request.operation === "exit-and-stop-receipt" &&
          !this.options.verifyStopProof?.(field(request.payload, "nativeStopProofId"))) {
        return encodeV37Receipt(reply("DENIED", current, current));
      }
      const next = current + 1n;
      this.store.sessions.set(storageKey, { state: transition[1], revision: next, generation });
      return committed(reply("APPLIED", current, next, { state: transition[1], generation }));
    }

    if (request.family === "K-LEDGER") {
      if (request.operation === "record") {
        if (BigInt(request.expectedRevision) !== current) return encodeV37Receipt(reply("STALE", current, current));
        const sourceId = field(request.payload, "sourceEventId");
        const sourceCursor = numeric(field(request.payload, "sourceCursor"));
        const scope = field(request.payload, "scope");
        if (!["GLOBAL", "PROJECT", "SIDE"].includes(scope)) throw new Error("V37_CORE_INVALID: scope");
        if (this.store.sourceIds.has(sourceId)) return encodeV37Receipt(reply("CONFLICT", current, current));
        if (sourceCursor !== current + 1n) return encodeV37Receipt(reply("CONFLICT", current, current,
          { expectedSourceCursor: (current + 1n).toString() }));
        const next = current + 1n;
        this.store.ledger.push({ sourceId, cursor: next, scope: scope as LedgerEvent["scope"], domain: request.domainId });
        this.store.sourceIds.add(sourceId);
        this.store.ledgerCursor = next;
        return committed(reply("APPLIED", current, next, { cursor: next.toString(), epoch: this.store.epoch }));
      }
      if (request.operation === "scoped-query") {
        if (field(request.payload, "epoch") !== this.store.epoch) return encodeV37Receipt(reply("STALE", current, current));
        const since = numeric(field(request.payload, "afterCursor"));
        const requestedScope = field(request.payload, "scope");
        if (requestedScope !== "PROJECT" && requestedScope !== "GLOBAL" && requestedScope !== "SIDE") {
          throw new Error("V37_CORE_INVALID: scope");
        }
        const events = this.store.ledger.filter((event) => event.cursor > since &&
          event.scope === requestedScope && event.domain === request.domainId)
          .map((event) => ({ sourceEventId: event.sourceId, cursor: event.cursor.toString() }));
        return committed(reply("APPLIED", current, current, { events, epoch: this.store.epoch,
          cursor: current.toString() }));
      }
      return encodeV37Receipt(reply("UNSUPPORTED", current, current));
    }

    const message = this.store.inbox.get(storageKey);
    if (request.operation === "check-unknown") {
      return committed(reply("APPLIED", current, current, { state: message?.state ?? "ABSENT" }));
    }
    if (BigInt(request.expectedRevision) !== current) return encodeV37Receipt(reply("STALE", current, current));
    if (request.operation === "enqueue") {
      if (message) return encodeV37Receipt(reply("CONFLICT", current, current));
      field(request.payload, "seatId"); field(request.payload, "turnId");
      const generation = field(request.payload, "generation");
      const body = field(request.payload, "body");
      this.store.inbox.set(storageKey, { state: "PENDING", revision: 1n, generation, body });
      return committed(reply("APPLIED", current, 1n, { state: "PENDING" }));
    }
    if (!message) return encodeV37Receipt(reply("CONFLICT", current, current));
    if (request.operation === "edit" || request.operation === "cancel") {
      if (message.state !== "PENDING") return encodeV37Receipt(reply("CONFLICT", current, current));
      if (request.operation === "edit") message.body = field(request.payload, "body");
      else message.state = "CANCELLED";
      message.revision += 1n;
      return committed(reply("APPLIED", current, message.revision, { state: message.state }));
    }
    if (request.operation === "deliver") {
      if (message.state !== "PENDING" || field(request.payload, "generation") !== message.generation) {
        return encodeV37Receipt(reply("CONFLICT", current, current));
      }
      await this.options.prepareDelivery?.(request.targetId);
      if (denied()) return encodeV37Receipt(reply("DENIED", message.revision, message.revision));
      if (message.state !== "PENDING" || message.revision !== current) {
        return encodeV37Receipt(reply("CONFLICT", message.revision, message.revision));
      }
      message.state = "PREPARED";
      let completion: "completed" | "unknown" = "unknown";
      let errorDetail: string | undefined;
      try {
        await this.options.beginCommitted?.(request.targetId);
        completion = await this.options.completeDelivery?.(request.targetId) ?? "unknown";
      } catch (error) {
        errorDetail = error instanceof Error ? error.message : String(error);
      }
      message.state = completion === "completed" ? "DELIVERED" : "UNKNOWN";
      message.revision += 1n;
      return committed(reply(completion === "completed" ? "APPLIED" : "UNKNOWN", current,
        message.revision, { state: message.state, ...(errorDetail ? { error: errorDetail } : {}) }));
    }
    if (request.operation === "requeue" && message.state === "UNKNOWN") {
      return encodeV37Receipt(reply("CONFLICT", current, current));
    }
    return encodeV37Receipt(reply("UNSUPPORTED", current, current));
  }
}
