/** D's directed side/lead seam. Native D owns intent identity, C owns its
 * inbox message, and H's original completion is the only delivery evidence. */
export type SideDeliveryDirection = "SIDE_TO_LEAD" | "LEAD_TO_SIDE";
export type SideDeliveryState = "UNKNOWN" | "STEERED" | "NEW_TURN" | "FAILED";

export interface SideDeliveryIntent {
  readonly domainId: string;
  readonly requestId: string;
  readonly sideId: string;
  readonly direction: SideDeliveryDirection;
  readonly sourceSeatId: string;
  readonly sourceSeatIncarnation: string;
  readonly sourceSessionId: string;
  readonly targetSeatId: string;
  readonly targetSeatIncarnation: string;
  readonly targetSessionId: string;
  readonly targetGeneration: string;
  readonly body: string;
  readonly messageId: string;
  readonly enqueueRequestId: string;
  readonly deliveryRequestId: string;
  readonly createdAt: string;
  readonly dispatchError: string;
  /** True only for D's first durable insertion. A replay can only observe. */
  readonly mayDispatch: boolean;
}

export interface SideDeliveryRecord {
  readonly intent: SideDeliveryIntent;
  readonly state: SideDeliveryState;
  readonly nativeReceiptId: string;
  readonly reason: string;
}

/** These methods must be installed by the native host. It derives the model
 * caller from its authenticated invocation, not from these JS arguments. */
export interface NativeSideDelivery {
  prepare(domainId: string, sideId: string, requestId: string,
    direction: SideDeliveryDirection, body: string): Promise<SideDeliveryIntent>;
  observe(domainId: string, requestId: string): Promise<SideDeliveryRecord>;
  recordError(domainId: string, requestId: string, originalError: string): Promise<void>;
  lines(domainId: string, sideId: string): Promise<readonly SideDeliveryRecord[]>;
}

/** Root provides its existing K-INBOX enqueue + H-backed steer/deliver path.
 * The callback must use the fixed C identities returned by D and preserve
 * native errors in C. This class cannot infer success from a resolved call. */
export interface SideInboxSubmit {
  submitOnce(intent: SideDeliveryIntent): Promise<void>;
}

export class SideChatDelivery {
  private readonly native: NativeSideDelivery;
  private readonly inbox: SideInboxSubmit;
  constructor(native: NativeSideDelivery, inbox: SideInboxSubmit) {
    this.native = native;
    this.inbox = inbox;
  }

  async send(domainId: string, sideId: string, requestId: string,
    direction: SideDeliveryDirection, body: string): Promise<SideDeliveryRecord> {
    if (!body.trim()) throw new Error("V37_SIDE_DELIVERY_BODY_REQUIRED");
    const intent = await this.native.prepare(domainId, sideId, requestId, direction, body);
    if (intent.domainId !== domainId || intent.sideId !== sideId || intent.requestId !== requestId ||
        intent.direction !== direction || intent.body !== body) {
      throw new Error("V37_SIDE_DELIVERY_INTENT_CORRELATION");
    }
    if (intent.mayDispatch) {
      try {
        await this.inbox.submitOnce(intent);
      } catch (error) {
        // The existing C/H operation may have written before an exception.
        // Its original journal is the source; this request is never resubmitted.
        await this.native.recordError(domainId, requestId,
          error instanceof Error ? error.message : String(error));
      }
    }
    const record = await this.native.observe(domainId, requestId);
    if (record.intent.requestId !== requestId || record.intent.sideId !== sideId ||
        record.intent.domainId !== domainId || record.intent.direction !== direction ||
        ((record.state === "STEERED" || record.state === "NEW_TURN") && !record.nativeReceiptId)) {
      throw new Error("V37_SIDE_DELIVERY_RECEIPT_CORRELATION");
    }
    return record;
  }

  read(domainId: string, sideId: string): Promise<readonly SideDeliveryRecord[]> {
    return this.native.lines(domainId, sideId);
  }
}
