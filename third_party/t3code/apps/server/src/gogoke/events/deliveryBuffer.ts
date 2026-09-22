import {
  MAX_OUTPUT_BYTES,
  MAX_OUTPUT_EVENTS,
  type DeliveryItem,
  type EventBinding,
  type EventCursor,
  type OutputDeliveryEvent,
  type OutputGap,
  type OutputGapReason,
  type TerminalEvidence,
} from "./types.ts";
import { compareU64, sameBinding, snapshotBinding, snapshotCursor } from "./validation.ts";

export type BufferOfferResult = "accepted" | "backpressure" | "closed";

function cursorFrom(value: EventCursor): EventCursor {
  return snapshotCursor({
    domainId: value.domainId,
    sourceEpoch: value.sourceEpoch,
    generation: value.generation,
    sequence: value.sequence,
  });
}

function copyOutput(event: OutputDeliveryEvent): OutputDeliveryEvent {
  const cursor = cursorFrom(event);
  if (event.kind !== "output" && event.kind !== "partial") {
    throw new Error("output.kind must be output or partial");
  }
  if (!(event.bytes instanceof Uint8Array)) throw new Error("output.bytes must be Uint8Array");
  return Object.freeze({ kind: event.kind, ...cursor, bytes: Uint8Array.from(event.bytes) });
}

function copyTerminal(event: TerminalEvidence): TerminalEvidence {
  const cursor = cursorFrom(event);
  if (event.kind !== "terminal") throw new Error("terminal.kind must be terminal");
  if (
    event.status !== "completed" &&
    event.status !== "failed" &&
    event.status !== "cancelled" &&
    event.status !== "unknown"
  ) {
    throw new Error("terminal.status is invalid");
  }
  if (event.eof !== true) throw new Error("terminal.eof must be true");
  if (event.exitCode !== null && !Number.isSafeInteger(event.exitCode)) {
    throw new Error("terminal.exitCode must be a safe integer or null");
  }
  if (typeof event.completionObserved !== "boolean" || typeof event.cancelled !== "boolean") {
    throw new Error("terminal completion and cancellation flags must be boolean");
  }
  if (event.partialFrame !== null && !(event.partialFrame instanceof Uint8Array)) {
    throw new Error("terminal.partialFrame must be Uint8Array or null");
  }
  for (const [field, value] of [
    ["nativeError", event.nativeError],
    ["toolFailure", event.toolFailure],
  ] as const) {
    if (value !== null && typeof value !== "string") {
      throw new Error(`terminal.${field} must be a string or null`);
    }
  }
  const derivedStatus = event.cancelled
    ? "cancelled"
    : event.nativeError !== null ||
        event.toolFailure !== null ||
        (event.exitCode !== null && event.exitCode !== 0)
      ? "failed"
      : event.exitCode === 0 && event.completionObserved && event.partialFrame === null
        ? "completed"
        : "unknown";
  if (event.status !== derivedStatus) {
    throw new Error(`terminal.status must be ${derivedStatus} for its evidence`);
  }
  return Object.freeze({
    kind: "terminal",
    ...cursor,
    status: event.status,
    eof: true,
    exitCode: event.exitCode,
    completionObserved: event.completionObserved,
    cancelled: event.cancelled,
    partialFrame: event.partialFrame === null ? null : Uint8Array.from(event.partialFrame),
    nativeError: event.nativeError,
    toolFailure: event.toolFailure,
  });
}

/**
 * Subscriber-local acceleration buffer. Callers may offer only events that are
 * already durable. Output is bounded; terminal evidence occupies a separate
 * reserved lane and therefore survives a slow consumer and output overflow.
 */
export class BoundedEventDeliveryBuffer {
  readonly #binding: EventBinding;
  readonly #outputs: OutputDeliveryEvent[] = [];
  #outputBytes = 0;
  #gap: OutputGap | null = null;
  #gapRevision = 0n;
  #deliveredGapRevision = 0n;
  #terminal: TerminalEvidence | null = null;
  #terminalDelivered = false;
  #resyncRequired = false;
  #closed = false;

  constructor(binding: EventBinding) {
    this.#binding = snapshotBinding(binding);
  }

  offerPersistedOutput(event: OutputDeliveryEvent): BufferOfferResult {
    if (this.#closed) return "closed";
    const output = copyOutput(event);
    this.#assertBinding(output);
    if (this.#resyncRequired) {
      this.#recordGap(output, "resync-pending");
      return "backpressure";
    }

    const countOverflow = this.#outputs.length >= MAX_OUTPUT_EVENTS;
    const byteOverflow = this.#outputBytes + output.bytes.byteLength > MAX_OUTPUT_BYTES;
    if (countOverflow || byteOverflow) {
      this.#resyncRequired = true;
      this.#recordGap(output, countOverflow ? "event-limit" : "byte-limit");
      return "backpressure";
    }
    this.#outputs.push(output);
    this.#outputBytes += output.bytes.byteLength;
    return "accepted";
  }

  offerPersistedTerminal(event: TerminalEvidence): BufferOfferResult {
    if (this.#closed) return "closed";
    const terminal = copyTerminal(event);
    this.#assertBinding(terminal);
    this.#terminal = terminal;
    this.#closed = true;
    return "accepted";
  }

  drain(): ReadonlyArray<DeliveryItem> {
    const items: DeliveryItem[] = this.#outputs.splice(0);
    this.#outputBytes = 0;
    if (this.#gap !== null && this.#gapRevision > this.#deliveredGapRevision) {
      items.push(this.#gap);
      this.#deliveredGapRevision = this.#gapRevision;
    }
    if (this.#terminal !== null && !this.#terminalDelivered) {
      items.push(this.#terminal);
      this.#terminalDelivered = true;
    }
    return Object.freeze(items);
  }

  /** Clears a gap only after the caller has installed a durable exact-binding snapshot. */
  resumeAfterDurableResync(cursor: EventCursor): void {
    if (this.#closed) throw new Error("a terminal buffer cannot be resumed");
    const snapshot = cursorFrom(cursor);
    this.#assertBinding(snapshot);
    if (this.#gap === null) throw new Error("no delivery gap requires resync");
    if (compareU64(snapshot.sequence, this.#gap.toSequence) < 0) {
      throw new Error("durable resync cursor does not cover the delivery gap");
    }
    this.#outputs.length = 0;
    this.#outputBytes = 0;
    this.#gap = null;
    this.#deliveredGapRevision = 0n;
    this.#resyncRequired = false;
  }

  get binding(): EventBinding {
    return this.#binding;
  }

  get bufferedOutputEvents(): number {
    return this.#outputs.length;
  }

  get bufferedOutputBytes(): number {
    return this.#outputBytes;
  }

  get resyncRequired(): boolean {
    return this.#resyncRequired;
  }

  #assertBinding(value: EventBinding): void {
    if (!sameBinding(value, this.#binding)) {
      throw new Error("delivery item belongs to a different domain, epoch, or generation");
    }
  }

  #recordGap(event: OutputDeliveryEvent, reason: OutputGapReason): void {
    const revision = (this.#gapRevision + 1n).toString();
    if (this.#gap === null) {
      this.#gap = Object.freeze({
        kind: "gap",
        revision,
        semantics: "cumulative-replacement",
        binding: this.#binding,
        fromSequence: event.sequence,
        toSequence: event.sequence,
        droppedEvents: 1,
        droppedBytes: event.bytes.byteLength,
        reason,
        resyncRequired: true,
      });
    } else {
      this.#gap = Object.freeze({
        ...this.#gap,
        revision,
        fromSequence:
          compareU64(event.sequence, this.#gap.fromSequence) < 0
            ? event.sequence
            : this.#gap.fromSequence,
        toSequence:
          compareU64(event.sequence, this.#gap.toSequence) > 0
            ? event.sequence
            : this.#gap.toSequence,
        droppedEvents: this.#gap.droppedEvents + 1,
        droppedBytes: this.#gap.droppedBytes + event.bytes.byteLength,
      });
    }
    this.#gapRevision += 1n;
  }
}
