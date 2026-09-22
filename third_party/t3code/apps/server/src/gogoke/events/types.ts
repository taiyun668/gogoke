import type { JsonValue } from "../contracts/model.ts";

export const EVENT_STREAM_SCHEMA = "gogoke.event-stream.v1" as const;

export const MAX_OUTPUT_EVENTS = 1_024;
export const MAX_OUTPUT_BYTES = 8 * 1_024 * 1_024;
export const MAX_FRAME_BYTES = 4 * 1_024 * 1_024;

export interface EventBinding {
  readonly domainId: string;
  readonly sourceEpoch: string;
  readonly generation: string;
}

export interface EventCursor extends EventBinding {
  readonly sequence: string;
}

export interface SequencedEvent<Payload extends JsonValue = JsonValue> {
  readonly schema: typeof EVENT_STREAM_SCHEMA;
  readonly eventId: string;
  readonly eventType: string;
  readonly binding: EventBinding;
  readonly sequence: string;
  readonly payload: Payload;
}

export interface EventSnapshot<State extends JsonValue = JsonValue> {
  readonly cursor: EventCursor;
  readonly state: State;
}

/**
 * The events package asks the existing host authority which binding is current.
 * It never infers authority from an incoming event and has no fallback authority.
 */
export interface EventBindingAuthority {
  currentBinding(domainId: string): Promise<EventBinding | null>;
  isKnownEventType(binding: Readonly<EventBinding>, eventType: string): Promise<boolean>;
}

export type EventPortWrite = "committed" | "conflict" | "stale-authority";

/**
 * Implementations bridge to the product's existing durable store. Both writes
 * must atomically compare the expected cursor and the supplied host binding.
 * This package deliberately provides no production store or authority.
 */
export interface EventProjectionPort<
  State extends JsonValue = JsonValue,
  Payload extends JsonValue = JsonValue,
> {
  readCursor(domainId: string): Promise<EventCursor | null>;
  eventIdAt(cursor: EventCursor): Promise<string | null>;
  append(
    event: Readonly<SequencedEvent<Payload>>,
    expectedCursor: Readonly<EventCursor> | null,
    expectedBinding: Readonly<EventBinding>,
  ): Promise<EventPortWrite>;
  replaceSnapshot(
    snapshot: Readonly<EventSnapshot<State>>,
    expectedCursor: Readonly<EventCursor> | null,
    expectedBinding: Readonly<EventBinding>,
  ): Promise<EventPortWrite>;
}

/** The source must return a durable snapshot for exactly the requested binding. */
export interface EventSnapshotSource<State extends JsonValue = JsonValue> {
  loadSnapshot(binding: Readonly<EventBinding>): Promise<EventSnapshot<State>>;
}

export type EventIngestResult =
  | { readonly status: "applied"; readonly cursor: EventCursor; readonly resynced: boolean }
  | { readonly status: "duplicate"; readonly cursor: EventCursor }
  | { readonly status: "late"; readonly cursor: EventCursor }
  | {
      readonly status: "out-of-order";
      readonly cursor: EventCursor;
      readonly expectedSequence: string;
      readonly receivedSequence: string;
    }
  | { readonly status: "unknown"; readonly reason: "DOMAIN" | "EVENT_TYPE" }
  | { readonly status: "stale"; readonly current: EventBinding }
  | { readonly status: "conflict"; readonly operation: "append" | "resync" };

export type EventRecoveryErrorCode =
  | "INVALID_EVENT"
  | "INVALID_AUTHORITY_BINDING"
  | "INVALID_CURSOR"
  | "INVALID_PORT_RESULT"
  | "INVALID_SNAPSHOT"
  | "SNAPSHOT_REGRESSION";

export class EventRecoveryError extends Error {
  override readonly name = "EventRecoveryError";
  readonly code: EventRecoveryErrorCode;
  override readonly cause: unknown;

  constructor(code: EventRecoveryErrorCode, detail: string, cause?: unknown) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.cause = cause;
  }
}

export interface OutputDeliveryEvent extends EventCursor {
  readonly kind: "output" | "partial";
  readonly bytes: Uint8Array;
}

export type OutputGapReason = "event-limit" | "byte-limit" | "resync-pending";

export interface OutputGap {
  readonly kind: "gap";
  /** Monotonic cumulative replacement revision; a later revision supersedes an earlier one. */
  readonly revision: string;
  readonly semantics: "cumulative-replacement";
  readonly binding: EventBinding;
  readonly fromSequence: string;
  readonly toSequence: string;
  readonly droppedEvents: number;
  readonly droppedBytes: number;
  readonly reason: OutputGapReason;
  readonly resyncRequired: true;
}

export type TerminalStatus = "completed" | "failed" | "cancelled" | "unknown";

export interface TerminalEvidence extends EventCursor {
  readonly kind: "terminal";
  readonly status: TerminalStatus;
  readonly eof: true;
  readonly exitCode: number | null;
  readonly completionObserved: boolean;
  readonly cancelled: boolean;
  readonly partialFrame: Uint8Array | null;
  readonly nativeError: string | null;
  readonly toolFailure: string | null;
}

export type DeliveryItem = OutputDeliveryEvent | OutputGap | TerminalEvidence;
