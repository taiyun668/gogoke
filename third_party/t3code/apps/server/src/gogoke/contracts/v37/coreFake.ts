import { canonicalJson } from "../strictJson.ts";
import { decodeV37Request, encodeV37Receipt, V37_SCHEMA, V37_U64_MAX, type V37Port, type V37Receipt, type V37Request, type V37TrustedCaller } from "./protocol.ts";

type SessionState = "RESERVED" | "COMMITTED" | "RUNNING" | "STOPPING" | "STOPPED" | "RELEASED" | "GENERATION_UNKNOWN";
interface Session { state: SessionState; revision: bigint; generation: string; }
type InboxState = "PENDING" | "PREPARED" | "UNKNOWN" | "DELIVERED" | "CANCELLED" | "FAILED";
interface Inbox { state: InboxState; revision: bigint; generation: string; body: string;
  seatId: string; turnId: string; requeuedAs?: string; }
interface LedgerEvent { sourceId: string; cursor: bigint; scope: "GLOBAL" | "PROJECT" | "SIDE"; domain: string; }
interface StoredReply { request: string; receipt: V37Receipt; }
interface Subscription { revision: bigint; state: "ACTIVE" | "ENDED"; scope: LedgerEvent["scope"];
  cursor: bigint; epoch: string; }

/** Test store survives fake-port reconstruction; this is not production persistence. */
export class V37CoreFakeStore {
  readonly sessions = new Map<string, Session>();
  readonly inbox = new Map<string, Inbox>();
  readonly ledger: LedgerEvent[] = [];
  readonly replies = new Map<string, StoredReply>();
  readonly sourceIds = new Set<string>();
  readonly subscriptions = new Map<string, Subscription>();
  ledgerCursor = 0n;
  epoch = "1";
}

export interface V37CoreFakeOptions {
  readonly caller: () => V37TrustedCaller | null;
  readonly granted: (caller: V37TrustedCaller, request: V37Request) => boolean;
  readonly canReadLedgerScope?: (caller: V37TrustedCaller,
    scope: "GLOBAL" | "PROJECT" | "SIDE", domainId: string) => boolean;
  readonly prepareDelivery?: (messageId: string) => Promise<void>;
  readonly abortPreparedDelivery?: (messageId: string) => Promise<boolean>;
  readonly beginCommitted?: (messageId: string) => Promise<void>;
  readonly completeDelivery?: (messageId: string) => Promise<"completed" | "unknown" |
    { readonly state: "failed"; readonly error: string }>;
  readonly verifyPinnedBinary?: (digest: string) => boolean;
  readonly verifyStopProof?: (proofId: string) => boolean;
  readonly sessionCapabilities?: (sessionId: string) => V37Request["payload"] | null;
  readonly readOutput?: (sessionId: string, generation: string, afterCursor: string) =>
    { readonly cursor: string; readonly events: ReadonlyArray<{ readonly eventId: string; readonly kind: string }> } | null;
  readonly sendInput?: (sessionId: string, operation: "send" | "append-without-turn",
    generation: string, body: string) => { readonly receiptId: string; readonly createdTurn: boolean } | null;
  readonly changeGeneration?: (sessionId: string, operation: "compact" | "renew-session",
    oldGeneration: string) => { readonly newGeneration: string; readonly receiptId: string } | "unsupported" | "unknown";
  readonly reconnectGeneration?: (sessionId: string, claimedGeneration: string) =>
    { readonly generation: string; readonly receiptId: string } | null;
  readonly currentTurn?: (seatId: string) => string | null;
  readonly steerMode?: (seatId: string) => "NATIVE" | "INTERRUPT_RESUME" | null;
  readonly canRequeueTarget?: (seatId: string, turnId: string,
    generation: string, domainId: string) => boolean;
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
  const number = BigInt(value);
  if (number > V37_U64_MAX) throw new Error("V37_CORE_INVALID: uint64 overflow");
  return number;
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
    const subscriptionOperation = request.family === "K-LEDGER" &&
      ["subscribe", "resume-subscription", "end-subscription"].includes(request.operation);
    const current = request.family === "K-SESSION" ? this.store.sessions.get(storageKey)?.revision ?? 0n
      : request.family === "K-INBOX" ? this.store.inbox.get(storageKey)?.revision ?? 0n
      : subscriptionOperation ? this.store.subscriptions.get(storageKey)?.revision ?? 0n
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
    const scopeDenied = (scope: LedgerEvent["scope"]) => {
      const caller = this.options.caller();
      return caller === null || !this.options.canReadLedgerScope?.(caller, scope, request.domainId);
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
      numeric(generation);
      if (request.operation === "capability-probe" || request.operation === "output-stream") {
        if (!session || session.state !== "RUNNING" || session.generation !== generation) {
          return encodeV37Receipt(reply("CONFLICT", current, current));
        }
        if (request.operation === "capability-probe") {
          const capabilities = this.options.sessionCapabilities?.(request.targetId);
          return capabilities === null || capabilities === undefined
            ? encodeV37Receipt(reply("UNSUPPORTED", current, current))
            : committed(reply("APPLIED", current, current, { capabilities, generation }));
        }
        const afterCursor = field(request.payload, "afterCursor");
        numeric(afterCursor);
        const output = this.options.readOutput?.(request.targetId, generation, afterCursor);
        if (!output) return encodeV37Receipt(reply("UNSUPPORTED", current, current));
        if (numeric(output.cursor) < numeric(afterCursor)) {
          return encodeV37Receipt(reply("CONFLICT", current, current));
        }
        return committed(reply("APPLIED", current, current,
          { events: output.events, cursor: output.cursor, generation }));
      }
      if (request.operation === "send" || request.operation === "append-without-turn") {
        if (!session || session.state !== "RUNNING" || session.generation !== generation) {
          return encodeV37Receipt(reply("CONFLICT", current, current));
        }
        const body = field(request.payload, "body");
        const operation = request.operation;
        const input = this.options.sendInput?.(request.targetId, operation, generation, body);
        if (!input) return encodeV37Receipt(reply("UNSUPPORTED", current, current));
        if (!input.receiptId || input.createdTurn !== (operation === "send")) {
          return encodeV37Receipt(reply("FAILED", current, current,
            { reason: "TURN_SHAPE_MISMATCH" }));
        }
        const next = current + 1n;
        session.revision = next;
        return committed(reply("APPLIED", current, next,
          { generation, receiptId: input.receiptId, createdTurn: input.createdTurn }));
      }
      if (request.operation === "compact" || request.operation === "renew-session") {
        if (!session || session.state !== "RUNNING" || session.generation !== generation) {
          return encodeV37Receipt(reply("CONFLICT", current, current));
        }
        const capabilities = this.options.sessionCapabilities?.(request.targetId);
        if (capabilities?.[request.operation] !== true) {
          return encodeV37Receipt(reply("UNSUPPORTED", current, current));
        }
        const change = this.options.changeGeneration?.(request.targetId, request.operation, generation);
        if (!change || change === "unsupported") return encodeV37Receipt(reply("UNSUPPORTED", current, current));
        if (change === "unknown") {
          session.state = "GENERATION_UNKNOWN";
          session.revision += 1n;
          return committed(reply("UNKNOWN", current, session.revision,
            { oldGeneration: generation, state: session.state }));
        }
        if (change.newGeneration === generation || !change.receiptId) {
          return encodeV37Receipt(reply("FAILED", current, current));
        }
        numeric(change.newGeneration);
        session.generation = change.newGeneration;
        session.revision += 1n;
        return committed(reply("APPLIED", current, session.revision,
          { oldGeneration: generation, newGeneration: change.newGeneration,
            receiptId: change.receiptId, state: "RUNNING" }));
      }
      if (request.operation === "reconnect") {
        if (!session || !["RUNNING", "GENERATION_UNKNOWN"].includes(session.state)) {
          return encodeV37Receipt(reply("CONFLICT", current, current));
        }
        const observed = this.options.reconnectGeneration?.(request.targetId, generation);
        if (!observed) return encodeV37Receipt(reply("UNSUPPORTED", current, current));
        if (!observed.receiptId) {
          return encodeV37Receipt(reply("FAILED", current, current));
        }
        numeric(observed.generation);
        const oldGeneration = session.generation;
        session.generation = observed.generation;
        session.state = "RUNNING";
        session.revision += 1n;
        return committed(reply("APPLIED", current, session.revision,
          { oldGeneration, newGeneration: observed.generation,
            receiptId: observed.receiptId, state: session.state }));
      }
      if (request.operation === "admission-release") {
        if (!session || !["RESERVED", "STOPPED"].includes(session.state) || session.generation !== generation) {
          return encodeV37Receipt(reply("CONFLICT", current, current));
        }
        session.state = "RELEASED";
        session.revision += 1n;
        return committed(reply("APPLIED", current, session.revision,
          { generation, state: session.state }));
      }
      const transitions: Record<string, readonly [SessionState | null, SessionState]> = {
        "admission-reserve": [null, "RESERVED"],
        "admission-commit": ["RESERVED", "COMMITTED"],
        open: ["COMMITTED", "RUNNING"],
        stop: ["RUNNING", "STOPPING"],
        "exit-and-stop-receipt": ["STOPPING", "STOPPED"],
      };
      const transition = transitions[request.operation];
      if (transition === undefined) return encodeV37Receipt(reply("UNSUPPORTED", current, current));
      if ((session?.state ?? null) !== transition[0] &&
          !(request.operation === "stop" && session?.state === "GENERATION_UNKNOWN") ||
          (session && session.generation !== generation)) {
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
        if (since > this.store.ledgerCursor) return encodeV37Receipt(reply("CONFLICT", current, current,
          { reason: "CURSOR_AHEAD", cursor: this.store.ledgerCursor.toString() }));
        const requestedScope = field(request.payload, "scope");
        if (requestedScope !== "PROJECT" && requestedScope !== "GLOBAL" && requestedScope !== "SIDE") {
          throw new Error("V37_CORE_INVALID: scope");
        }
        if (scopeDenied(requestedScope)) return encodeV37Receipt(reply("DENIED", current, current));
        const events = this.store.ledger.filter((event) => event.cursor > since &&
          event.scope === requestedScope && event.domain === request.domainId)
          .map((event) => ({ sourceEventId: event.sourceId, cursor: event.cursor.toString() }));
        return committed(reply("APPLIED", current, current, { events, epoch: this.store.epoch,
          cursor: current.toString() }));
      }
      if (subscriptionOperation) {
        if (BigInt(request.expectedRevision) !== current) return encodeV37Receipt(reply("STALE", current, current));
        const subscription = this.store.subscriptions.get(storageKey);
        if (request.operation === "subscribe") {
          if (subscription) return encodeV37Receipt(reply("CONFLICT", current, current));
          if (field(request.payload, "epoch") !== this.store.epoch) return encodeV37Receipt(reply("STALE", current, current));
          const after = numeric(field(request.payload, "afterCursor"));
          if (after > this.store.ledgerCursor) return encodeV37Receipt(reply("CONFLICT", current, current,
            { reason: "CURSOR_AHEAD" }));
          const scope = field(request.payload, "scope");
          if (scope !== "GLOBAL" && scope !== "PROJECT" && scope !== "SIDE") {
            throw new Error("V37_CORE_INVALID: scope");
          }
          if (scopeDenied(scope)) return encodeV37Receipt(reply("DENIED", current, current));
          const events = this.store.ledger.filter((event) => event.cursor > after &&
            event.scope === scope && event.domain === request.domainId)
            .map((event) => ({ sourceEventId: event.sourceId, cursor: event.cursor.toString() }));
          this.store.subscriptions.set(storageKey, { revision: 1n, state: "ACTIVE",
            scope, cursor: this.store.ledgerCursor, epoch: this.store.epoch });
          return committed(reply("APPLIED", current, 1n,
            { events, cursor: this.store.ledgerCursor.toString(), epoch: this.store.epoch }));
        }
        if (!subscription || subscription.state !== "ACTIVE") {
          return encodeV37Receipt(reply("CONFLICT", current, current));
        }
        if (request.operation === "resume-subscription") {
          if (scopeDenied(subscription.scope)) return encodeV37Receipt(reply("DENIED", current, current));
          if (field(request.payload, "epoch") !== subscription.epoch) {
            return encodeV37Receipt(reply("STALE", current, current));
          }
          const after = numeric(field(request.payload, "afterCursor"));
          if (after !== subscription.cursor) {
            return encodeV37Receipt(reply("CONFLICT", current, current,
              { reason: after < subscription.cursor ? "CURSOR_REWIND" : "CURSOR_GAP" }));
          }
          const events = this.store.ledger.filter((event) => event.cursor > after &&
            event.scope === subscription.scope && event.domain === request.domainId)
            .map((event) => ({ sourceEventId: event.sourceId, cursor: event.cursor.toString() }));
          subscription.cursor = this.store.ledgerCursor;
          subscription.revision += 1n;
          return committed(reply("APPLIED", current, subscription.revision,
            { events, cursor: subscription.cursor.toString(), epoch: subscription.epoch }));
        }
        subscription.state = "ENDED";
        subscription.revision += 1n;
        return committed(reply("APPLIED", current, subscription.revision,
          { state: "ENDED", cursor: subscription.cursor.toString(), epoch: subscription.epoch }));
      }
      return encodeV37Receipt(reply("UNSUPPORTED", current, current));
    }

    const message = this.store.inbox.get(storageKey);
    if (request.operation === "check-unknown") {
      return committed(reply("APPLIED", current, current,
        { state: message?.state ?? "ABSENT", requeuedAs: message?.requeuedAs ?? null }));
    }
    if (BigInt(request.expectedRevision) !== current) return encodeV37Receipt(reply("STALE", current, current));
    if (request.operation === "enqueue") {
      if (message) return encodeV37Receipt(reply("CONFLICT", current, current));
      const seatId = field(request.payload, "seatId");
      const turnId = field(request.payload, "turnId");
      const generation = field(request.payload, "generation");
      const body = field(request.payload, "body");
      this.store.inbox.set(storageKey, { state: "PENDING", revision: 1n,
        generation, body, seatId, turnId });
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
    if (request.operation === "deliver" || request.operation === "steer") {
      if (message.state !== "PENDING" || field(request.payload, "generation") !== message.generation) {
        return encodeV37Receipt(reply("CONFLICT", current, current));
      }
      let mode: "NATIVE" | "INTERRUPT_RESUME" | undefined;
      if (request.operation === "steer") {
        if (field(request.payload, "turnId") !== message.turnId ||
            this.options.currentTurn?.(message.seatId) !== message.turnId) {
          return encodeV37Receipt(reply("CONFLICT", current, current, { reason: "TURN_ENDED" }));
        }
        mode = this.options.steerMode?.(message.seatId) ?? undefined;
        if (!mode) return encodeV37Receipt(reply("UNSUPPORTED", current, current));
      }
      if (!this.options.prepareDelivery || !this.options.abortPreparedDelivery ||
          !this.options.beginCommitted || !this.options.completeDelivery) {
        return encodeV37Receipt(reply("UNSUPPORTED", current, current));
      }
      await this.options.prepareDelivery?.(request.targetId);
      const abortPrepared = async (status: "DENIED" | "CONFLICT", reason?: string): Promise<Uint8Array> => {
        let aborted = false;
        let abortError: string | undefined;
        try { aborted = await this.options.abortPreparedDelivery!(request.targetId); }
        catch (error) { abortError = error instanceof Error ? error.message : String(error); }
        if (!aborted) {
          const before = message.revision;
          message.state = "UNKNOWN";
          message.revision += 1n;
          return committed(reply("UNKNOWN", before, message.revision,
            { state: "UNKNOWN", ...(abortError ? { error: abortError } : {}) }));
        }
        return encodeV37Receipt(reply(status, message.revision, message.revision,
          reason ? { reason } : {}));
      };
      if (denied()) return abortPrepared("DENIED");
      if (message.state !== "PENDING" || message.revision !== current) {
        return abortPrepared("CONFLICT");
      }
      if (request.operation === "steer" && this.options.currentTurn?.(message.seatId) !== message.turnId) {
        return abortPrepared("CONFLICT", "TURN_ENDED");
      }
      message.state = "PREPARED";
      let completion: "completed" | "unknown" | "failed" = "unknown";
      let errorDetail: string | undefined;
      try {
        await this.options.beginCommitted?.(request.targetId);
        const result = await this.options.completeDelivery?.(request.targetId) ?? "unknown";
        if (typeof result === "string") completion = result;
        else { completion = result.state; errorDetail = result.error; }
      } catch (error) {
        errorDetail = error instanceof Error ? error.message : String(error);
      }
      message.state = completion === "completed" ? "DELIVERED" :
        completion === "failed" ? "FAILED" : "UNKNOWN";
      message.revision += 1n;
      return committed(reply(completion === "completed" ? "APPLIED" :
        completion === "failed" ? "FAILED" : "UNKNOWN", current,
        message.revision, { state: message.state, ...(mode ? { mode } : {}),
          ...(errorDetail ? { error: errorDetail } : {}) }));
    }
    if (request.operation === "requeue") {
      if (message.state !== "FAILED" || message.requeuedAs) {
        return encodeV37Receipt(reply("CONFLICT", current, current));
      }
      const newMessageId = field(request.payload, "newMessageId");
      const seatId = field(request.payload, "seatId");
      const turnId = field(request.payload, "turnId");
      const generation = field(request.payload, "generation");
      numeric(generation);
      if (!/^[A-Za-z][A-Za-z0-9_-]{0,127}$/u.test(newMessageId) || newMessageId === request.targetId) {
        throw new Error("V37_CORE_INVALID: newMessageId");
      }
      if (!this.options.canRequeueTarget?.(seatId, turnId, generation, request.domainId)) {
        return encodeV37Receipt(reply("DENIED", current, current,
          { reason: "TARGET_NOT_ELIGIBLE" }));
      }
      const newKey = `K-INBOX:${request.domainId}:${newMessageId}`;
      if (this.store.inbox.has(newKey)) return encodeV37Receipt(reply("CONFLICT", current, current));
      this.store.inbox.set(newKey, { state: "PENDING", revision: 1n,
        generation, body: message.body, seatId, turnId });
      message.requeuedAs = newMessageId;
      message.revision += 1n;
      return committed(reply("APPLIED", current, message.revision,
        { state: "FAILED", newMessageId, newState: "PENDING", newRevision: "1" }));
    }
    return encodeV37Receipt(reply("UNSUPPORTED", current, current));
  }
}
