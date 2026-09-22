import * as NodeUtilTypes from "node:util/types";

import type { JsonObject, JsonValue } from "../contracts/model.ts";
import {
  EVENT_STREAM_SCHEMA,
  EventRecoveryError,
  type EventBinding,
  type EventCursor,
  type EventRecoveryErrorCode,
  type EventSnapshot,
  type SequencedEvent,
} from "./types.ts";

const U64 = /^(?:0|[1-9][0-9]*)$/;
const SAFE_ID = /^[A-Za-z0-9][A-Za-z0-9._:@/-]{0,255}$/;
const MAX_U64 = 18_446_744_073_709_551_615n;

const fail = (code: EventRecoveryErrorCode, detail: string): never => {
  throw new EventRecoveryError(code, detail);
};

function passiveRecord(
  value: unknown,
  path: string,
  expectedKeys: ReadonlyArray<string>,
  code: EventRecoveryErrorCode,
): Readonly<Record<string, unknown>> {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    NodeUtilTypes.isProxy(value)
  ) {
    return fail(code, `${path} must be a non-Proxy plain object`);
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) {
    return fail(code, `${path} must be a non-Proxy plain object`);
  }
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key === "symbol")) {
    return fail(code, `${path} must not contain symbol keys`);
  }
  const names = keys as ReadonlyArray<string>;
  const extra = names.find((name) => !expectedKeys.includes(name));
  if (extra !== undefined) return fail(code, `${path}.${extra} is not allowed`);
  const missing = expectedKeys.find((name) => !names.includes(name));
  if (missing !== undefined) return fail(code, `${path}.${missing} is required`);
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const output: Record<string, unknown> = {};
  for (const name of names) {
    const descriptor = descriptors[name];
    if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
      return fail(code, `${path}.${name} must be an enumerable data property`);
    }
    output[name] = descriptor.value;
  }
  return output;
}

function canonicalId(value: unknown, path: string, code: EventRecoveryErrorCode): string {
  if (typeof value !== "string" || !SAFE_ID.test(value)) {
    return fail(code, `${path} must be a safe non-empty identifier`);
  }
  return value;
}

export function canonicalU64(
  value: unknown,
  path: string,
  code: EventRecoveryErrorCode = "INVALID_EVENT",
): string {
  if (typeof value !== "string" || !U64.test(value) || BigInt(value) > MAX_U64) {
    return fail(code, `${path} must be a canonical uint64 string`);
  }
  return value;
}

export function compareU64(left: string, right: string): number {
  const leftValue = BigInt(left);
  const rightValue = BigInt(right);
  return leftValue < rightValue ? -1 : leftValue > rightValue ? 1 : 0;
}

export function incrementU64(value: string): string | null {
  const next = BigInt(value) + 1n;
  return next > MAX_U64 ? null : next.toString();
}

export function snapshotBinding(
  value: unknown,
  path = "binding",
  code: EventRecoveryErrorCode = "INVALID_EVENT",
): EventBinding {
  const record = passiveRecord(value, path, ["domainId", "sourceEpoch", "generation"], code);
  return Object.freeze({
    domainId: canonicalId(record.domainId, `${path}.domainId`, code),
    sourceEpoch: canonicalU64(record.sourceEpoch, `${path}.sourceEpoch`, code),
    generation: canonicalU64(record.generation, `${path}.generation`, code),
  });
}

export function snapshotCursor(
  value: unknown,
  path = "cursor",
  code: EventRecoveryErrorCode = "INVALID_CURSOR",
): EventCursor {
  const record = passiveRecord(
    value,
    path,
    ["domainId", "sourceEpoch", "generation", "sequence"],
    code,
  );
  return Object.freeze({
    domainId: canonicalId(record.domainId, `${path}.domainId`, code),
    sourceEpoch: canonicalU64(record.sourceEpoch, `${path}.sourceEpoch`, code),
    generation: canonicalU64(record.generation, `${path}.generation`, code),
    sequence: canonicalU64(record.sequence, `${path}.sequence`, code),
  });
}

function snapshotJson(
  value: unknown,
  path: string,
  code: EventRecoveryErrorCode,
  ancestors = new WeakSet<object>(),
): JsonValue {
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value))) {
      return fail(code, `${path} contains an unsafe JSON number`);
    }
    return value;
  }
  if (typeof value !== "object" || NodeUtilTypes.isProxy(value)) {
    return fail(code, `${path} must be JSON data`);
  }
  if (ancestors.has(value)) return fail(code, `${path} must not contain cycles`);
  ancestors.add(value);
  try {
    if (Array.isArray(value)) {
      const descriptors = Object.getOwnPropertyDescriptors(value);
      const keys = Reflect.ownKeys(value);
      if (keys.some((key) => typeof key === "symbol")) {
        return fail(code, `${path} must not contain symbol keys`);
      }
      const extras = (keys as ReadonlyArray<string>).filter(
        (key) => key !== "length" && !/^(?:0|[1-9][0-9]*)$/.test(key),
      );
      if (extras.length > 0) return fail(code, `${path}.${extras[0]} is not valid JSON array data`);
      const output: JsonValue[] = [];
      for (let index = 0; index < value.length; index += 1) {
        const descriptor = descriptors[String(index)];
        if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
          return fail(code, `${path}[${index}] must be an enumerable data property`);
        }
        output.push(snapshotJson(descriptor.value, `${path}[${index}]`, code, ancestors));
      }
      return Object.freeze(output);
    }

    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null) {
      return fail(code, `${path} must contain only plain JSON objects`);
    }
    const keys = Reflect.ownKeys(value);
    if (keys.some((key) => typeof key === "symbol")) {
      return fail(code, `${path} must not contain symbol keys`);
    }
    const descriptors = Object.getOwnPropertyDescriptors(value);
    const output: Record<string, JsonValue> = {};
    for (const key of keys as ReadonlyArray<string>) {
      const descriptor = descriptors[key];
      if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
        return fail(code, `${path}.${key} must be an enumerable data property`);
      }
      Object.defineProperty(output, key, {
        value: snapshotJson(descriptor.value, `${path}.${key}`, code, ancestors),
        enumerable: true,
        configurable: false,
        writable: false,
      });
    }
    return Object.freeze(output) as JsonObject;
  } finally {
    ancestors.delete(value);
  }
}

export function snapshotEvent<Payload extends JsonValue>(
  value: unknown,
): Readonly<SequencedEvent<Payload>> {
  const record = passiveRecord(
    value,
    "event",
    ["schema", "eventId", "eventType", "binding", "sequence", "payload"],
    "INVALID_EVENT",
  );
  if (record.schema !== EVENT_STREAM_SCHEMA) {
    return fail("INVALID_EVENT", `event.schema must be ${EVENT_STREAM_SCHEMA}`);
  }
  return Object.freeze({
    schema: EVENT_STREAM_SCHEMA,
    eventId: canonicalId(record.eventId, "event.eventId", "INVALID_EVENT"),
    eventType: canonicalId(record.eventType, "event.eventType", "INVALID_EVENT"),
    binding: snapshotBinding(record.binding),
    sequence: canonicalU64(record.sequence, "event.sequence"),
    payload: snapshotJson(record.payload, "event.payload", "INVALID_EVENT") as Payload,
  });
}

export function snapshotRecoveryState<State extends JsonValue>(
  value: unknown,
): Readonly<EventSnapshot<State>> {
  const record = passiveRecord(value, "snapshot", ["cursor", "state"], "INVALID_SNAPSHOT");
  return Object.freeze({
    cursor: snapshotCursor(record.cursor, "snapshot.cursor", "INVALID_SNAPSHOT"),
    state: snapshotJson(record.state, "snapshot.state", "INVALID_SNAPSHOT") as State,
  });
}

export function sameBinding(left: EventBinding, right: EventBinding): boolean {
  return (
    left.domainId === right.domainId &&
    left.sourceEpoch === right.sourceEpoch &&
    left.generation === right.generation
  );
}

export function cursorFor(event: SequencedEvent): EventCursor {
  return Object.freeze({ ...event.binding, sequence: event.sequence });
}
