import {
  MAX_FRAME_BYTES,
  type EventBinding,
  type OutputDeliveryEvent,
  type TerminalEvidence,
  type TerminalStatus,
} from "./types.ts";
import { canonicalU64, incrementU64, snapshotBinding } from "./validation.ts";

export interface NativeFinish {
  readonly exitCode: number | null;
  readonly completionObserved: boolean;
  readonly cancelled?: boolean;
  readonly nativeError?: string;
  readonly toolFailure?: string;
}

export interface NativeFinishResult {
  readonly partial: OutputDeliveryEvent | null;
  readonly terminal: TerminalEvidence;
}

function appendBytes(left: Uint8Array, right: Uint8Array): Uint8Array {
  const joined = new Uint8Array(left.byteLength + right.byteLength);
  joined.set(left, 0);
  joined.set(right, left.byteLength);
  return joined;
}

function cleanDetail(value: string | undefined, field: string): string | null {
  if (value === undefined) return null;
  if (typeof value !== "string") throw new Error(`${field} must be a string`);
  return value;
}

function terminalStatus(input: {
  readonly exitCode: number | null;
  readonly completionObserved: boolean;
  readonly cancelled: boolean;
  readonly partial: Uint8Array | null;
  readonly nativeError: string | null;
  readonly toolFailure: string | null;
}): TerminalStatus {
  if (input.cancelled) return "cancelled";
  if (input.nativeError !== null || input.toolFailure !== null) return "failed";
  if (input.exitCode !== null && input.exitCode !== 0) return "failed";
  if (input.exitCode === 0 && input.completionObserved && input.partial === null) {
    return "completed";
  }
  return "unknown";
}

/**
 * Incremental LF framing for native output. EOF publishes remaining bytes as a
 * partial event and repeats those bytes in terminal evidence, so an error or
 * exit cannot erase content already observed.
 */
export class NativeFrameAccumulator {
  readonly #binding: EventBinding;
  #buffer: Uint8Array = new Uint8Array(0);
  #finished = false;
  #nextSequence: string;

  constructor(binding: EventBinding, firstSequence: string) {
    this.#binding = snapshotBinding(binding);
    this.#nextSequence = canonicalU64(firstSequence, "firstSequence");
  }

  push(bytes: Uint8Array): ReadonlyArray<OutputDeliveryEvent> {
    if (this.#finished) throw new Error("native frame accumulator is already finished");
    if (!(bytes instanceof Uint8Array)) throw new Error("native bytes must be Uint8Array");
    const buffer = appendBytes(this.#buffer, Uint8Array.from(bytes));
    const frames: OutputDeliveryEvent[] = [];
    let nextSequence = this.#nextSequence;
    let start = 0;
    for (let index = 0; index < buffer.byteLength; index += 1) {
      if (buffer[index] !== 0x0a) continue;
      let end = index;
      if (end > start && buffer[end - 1] === 0x0d) end -= 1;
      const frame = buffer.slice(start, end);
      if (frame.byteLength > MAX_FRAME_BYTES) throw new Error("native frame exceeds 4 MiB");
      const sequence = nextSequence;
      const following = incrementU64(sequence);
      if (following === null) throw new Error("native event sequence exhausted uint64");
      nextSequence = following;
      frames.push(Object.freeze({ kind: "output", ...this.#binding, sequence, bytes: frame }));
      start = index + 1;
    }
    const remainder = buffer.slice(start);
    if (remainder.byteLength > MAX_FRAME_BYTES) throw new Error("native frame exceeds 4 MiB");
    this.#buffer = remainder;
    this.#nextSequence = nextSequence;
    return Object.freeze(frames);
  }

  finish(input: NativeFinish): NativeFinishResult {
    if (this.#finished) throw new Error("native frame accumulator is already finished");
    if (input.exitCode !== null && !Number.isSafeInteger(input.exitCode)) {
      throw new Error("finish.exitCode must be a safe integer or null");
    }
    if (typeof input.completionObserved !== "boolean") {
      throw new Error("finish.completionObserved must be boolean");
    }
    const cancelled = input.cancelled ?? false;
    if (typeof cancelled !== "boolean") throw new Error("finish.cancelled must be boolean");
    const nativeError = cleanDetail(input.nativeError, "finish.nativeError");
    const toolFailure = cleanDetail(input.toolFailure, "finish.toolFailure");
    const partialBytes = this.#buffer.byteLength === 0 ? null : Uint8Array.from(this.#buffer);
    const partialSequence = partialBytes === null ? null : this.#takeSequence();
    const partial =
      partialBytes === null || partialSequence === null
        ? null
        : Object.freeze({
            kind: "partial" as const,
            ...this.#binding,
            sequence: partialSequence,
            bytes: partialBytes,
          });
    const sequence = this.#nextSequence;
    const terminalInput = {
      exitCode: input.exitCode,
      completionObserved: input.completionObserved,
      cancelled,
      partial: partialBytes,
      nativeError,
      toolFailure,
    };
    const terminal = Object.freeze({
      kind: "terminal" as const,
      ...this.#binding,
      sequence,
      status: terminalStatus(terminalInput),
      eof: true as const,
      exitCode: input.exitCode,
      completionObserved: input.completionObserved,
      cancelled,
      partialFrame: partialBytes === null ? null : Uint8Array.from(partialBytes),
      nativeError,
      toolFailure,
    });
    this.#buffer = new Uint8Array(0);
    this.#finished = true;
    return Object.freeze({ partial, terminal });
  }

  #takeSequence(): string {
    const current = this.#nextSequence;
    const next = incrementU64(current);
    if (next === null) throw new Error("native event sequence exhausted uint64");
    this.#nextSequence = next;
    return current;
  }
}
