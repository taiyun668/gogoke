import { wrapInboxMessage } from "./envelope.ts";

/** C coordinates one delivery. Durable identity and authority stay in the native store. */
export interface InboxTarget {
  readonly domainId: string;
  readonly messageId: string;
  readonly senderSeatId: string;
  readonly seatId: string;
  readonly turnId: string;
  readonly generation: string;
  readonly body: string;
}

export interface InboxDeliveryRequest {
  readonly requestId: string;
  readonly rawRequest: Uint8Array;
  readonly target: InboxTarget;
  readonly expectedRevision: string;
  readonly kind: "deliver" | "steer";
}

export interface InboxDeliveryReceipt {
  readonly status: "APPLIED" | "REPLAYED" | "DENIED" | "STALE" | "CONFLICT" |
    "UNSUPPORTED" | "UNKNOWN" | "FAILED";
  readonly previousRevision: string;
  readonly revision: string;
  readonly state: "PENDING" | "DELIVERED" | "UNKNOWN" | "FAILED";
  readonly nativeReceiptId?: string;
  readonly mode?: "NATIVE" | "INTERRUPT_RESUME";
  readonly reason?: string;
}

export interface NativeInboxDeliveryStore {
  /** Reserves the exact raw request and message revision durably; replay never grants a second dispatch. */
  reserve(request: InboxDeliveryRequest): Promise<
    { readonly state: "RESERVED"; readonly target: InboxTarget } | InboxDeliveryReceipt>;
  /** Reads the native issuer's current grant, after H has prepared the exact target. */
  currentGrant(request: InboxDeliveryRequest): Promise<boolean>;
  /** Reads H's current turn through the native host, never from a Node cache. */
  currentTurn(seatId: string): Promise<string | null>;
  /** Abort confirmed by H allows PENDING; unconfirmed abort is durable UNKNOWN. */
  abort(request: InboxDeliveryRequest, confirmed: boolean,
    reason: string): Promise<InboxDeliveryReceipt>;
  /** Persist UNKNOWN before the first possibly irreversible H send. */
  markCommitUnknown(request: InboxDeliveryRequest): Promise<InboxDeliveryReceipt>;
  /** Only a native completion receipt may turn UNKNOWN into DELIVERED. */
  settle(request: InboxDeliveryRequest, result:
    { readonly kind: "completed"; readonly nativeReceiptId: string;
      readonly mode?: "NATIVE" | "INTERRUPT_RESUME" } |
    { readonly kind: "failed"; readonly reason: string }): Promise<InboxDeliveryReceipt>;
}

export interface HInboxDelivery {
  prepare(request: InboxDeliveryRequest, formattedBody: string): Promise<unknown>;
  abortPrepared(prepared: unknown): Promise<boolean>;
  beginCommitted(prepared: unknown): Promise<void>;
  complete(prepared: unknown): Promise<
    { readonly kind: "completed"; readonly nativeReceiptId: string } |
    { readonly kind: "failed"; readonly reason: string } |
    { readonly kind: "unknown" }>;
  steerMode(seatId: string): Promise<"NATIVE" | "INTERRUPT_RESUME" | null>;
}

function detail(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** No H port is installed by this module. The product entry must supply both native ports. */
export class InboxDeliveryCoordinator {
  private readonly store: NativeInboxDeliveryStore;
  private readonly host: HInboxDelivery;
  constructor(store: NativeInboxDeliveryStore, host: HInboxDelivery) {
    this.store = store;
    this.host = host;
  }

  async deliver(request: InboxDeliveryRequest): Promise<InboxDeliveryReceipt> {
    const reservation = await this.store.reserve(request);
    if (reservation.state !== "RESERVED") return reservation;
    const target = reservation.target;
    if (target.domainId !== request.target.domainId ||
        target.messageId !== request.target.messageId ||
        target.senderSeatId !== request.target.senderSeatId ||
        target.seatId !== request.target.seatId ||
        target.turnId !== request.target.turnId ||
        target.generation !== request.target.generation ||
        target.body !== request.target.body) {
      return this.store.abort(request, true, "DENIED");
    }
    let mode: "NATIVE" | "INTERRUPT_RESUME" | undefined;
    if (request.kind === "steer") {
      if (await this.store.currentTurn(target.seatId) !== target.turnId) {
        return this.store.abort(request, true, "TURN_ENDED");
      }
      mode = await this.host.steerMode(target.seatId) ?? undefined;
      if (!mode) return this.store.abort(request, true, "DENIED");
    }
    let prepared: unknown;
    try {
      prepared = await this.host.prepare(request,
        wrapInboxMessage(target.senderSeatId, target.messageId, target.body));
    } catch (error) {
      return this.store.abort(request, false, `PREPARE_UNKNOWN: ${detail(error)}`);
    }
    let allowed = false;
    let sameTurn = request.kind !== "steer";
    try {
      allowed = await this.store.currentGrant(request);
      if (request.kind === "steer") sameTurn = await this.store.currentTurn(target.seatId) === target.turnId;
    } catch (error) {
      const reason = `OBSERVATION_UNKNOWN: ${detail(error)}`;
      let aborted = false;
      try { aborted = await this.host.abortPrepared(prepared); }
      catch (abortError) { return this.store.abort(request, false, `${reason}; H_ABORT: ${detail(abortError)}`); }
      return this.store.abort(request, aborted, reason);
    }
    if (!allowed || !sameTurn) {
      let aborted = false;
      let reason = !allowed ? "DENIED" : "TURN_ENDED";
      try { aborted = await this.host.abortPrepared(prepared); }
      catch (error) { reason += `; H_ABORT: ${detail(error)}`; }
      return this.store.abort(request, aborted, reason);
    }
    let uncertain: InboxDeliveryReceipt;
    try {
      uncertain = await this.store.markCommitUnknown(request);
    } catch (error) {
      let aborted = false;
      let hostAbortError: unknown;
      try { aborted = await this.host.abortPrepared(prepared); } catch (abortError) { hostAbortError = abortError; }
      try { await this.store.abort(request, aborted,
        `COMMIT_UNKNOWN: ${detail(error)}${hostAbortError === undefined ? "" : `; H_ABORT: ${detail(hostAbortError)}`}`); }
      catch (abortError) { throw new AggregateError([error, abortError], "delivery admission and abort failed"); }
      if (hostAbortError !== undefined) {
        throw new AggregateError([error, hostAbortError], "delivery admission and H abort failed");
      }
      throw error;
    }
    if (uncertain.status !== "UNKNOWN") {
      let aborted = false;
      let reason = uncertain.status;
      try { aborted = await this.host.abortPrepared(prepared); }
      catch (error) { reason += `; H_ABORT: ${detail(error)}`; }
      return this.store.abort(request, aborted, reason);
    }
    let completion: Awaited<ReturnType<HInboxDelivery["complete"]>> = { kind: "unknown" };
    try {
      await this.host.beginCommitted(prepared);
      completion = await this.host.complete(prepared);
    } catch (error) {
      return { ...uncertain, reason: detail(error) };
    }
    if (completion.kind === "unknown") return uncertain;
    if (completion.kind === "completed") {
      if (!completion.nativeReceiptId) return uncertain;
      return this.store.settle(request, { kind: "completed", nativeReceiptId: completion.nativeReceiptId,
        ...(mode ? { mode } : {}) });
    }
    return this.store.settle(request, completion);
  }
}
