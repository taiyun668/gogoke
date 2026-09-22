import { isProxy } from "node:util/types";

export class DecisionEngineError extends Error {
  override readonly name = "DecisionEngineError";
  readonly code:
    | "INVALID_INPUT"
    | "STALE_VIEW"
    | "BACKEND_PROTOCOL"
    | "COMMIT_DENIED"
    | "COMMIT_CONFLICT"
    | "COMMIT_UNKNOWN";

  constructor(
    code:
      | "INVALID_INPUT"
      | "STALE_VIEW"
      | "BACKEND_PROTOCOL"
      | "COMMIT_DENIED"
      | "COMMIT_CONFLICT"
      | "COMMIT_UNKNOWN",
  ) {
    super(code);
    this.code = code;
  }
}

type Reject = () => never;
const descriptor = Object.getOwnPropertyDescriptor;
const prototype = Object.getPrototypeOf;
const hasOwn = Object.hasOwn;
const ownKeys = Reflect.ownKeys;
const isArray = Array.isArray;
const freeze = Object.freeze;
const apply = Reflect.apply;

function passiveObject(value: unknown, reject: Reject): object {
  if (value === null || typeof value !== "object" || isProxy(value)) return reject();
  return value;
}
function ownData(value: object, key: PropertyKey, reject: Reject): unknown {
  const field = descriptor(value, key);
  if (!field || !hasOwn(field, "value")) return reject();
  return field.value;
}

/** Exact own-data records only. Never call a getter, iterator or Proxy trap. */
export function recordData(
  value: unknown, required: readonly string[], optional: readonly string[], reject: Reject,
): Readonly<Record<string, unknown>> {
  const input = passiveObject(value, reject);
  const parent = prototype(input);
  if (isArray(input) || (parent !== null && parent !== Object.prototype)) return reject();
  const keys = ownKeys(input);
  if (keys.length > required.length + optional.length) return reject();
  const result: Record<string, unknown> = Object.create(null);
  for (const key of keys) {
    if (typeof key !== "string" || (!required.includes(key) && !optional.includes(key))) return reject();
    const field = descriptor(input, key);
    if (!field || !hasOwn(field, "value") || !field.enumerable) return reject();
    result[key] = field.value;
  }
  for (const key of required) if (!hasOwn(result, key)) return reject();
  return freeze(result);
}

/** Bound before copying; dense own integer indexes, no symbols or custom methods. */
export function arrayData(value: unknown, reject: Reject): readonly unknown[] {
  const input = passiveObject(value, reject);
  if (!isArray(input) || prototype(input) !== Array.prototype) return reject();
  const length = ownData(input, "length", reject);
  if (typeof length !== "number" || !Number.isSafeInteger(length) || length < 0 || length > 256) return reject();
  const keys = ownKeys(input);
  if (keys.length !== length + 1) return reject();
  for (const key of keys) {
    if (key === "length") continue;
    if (typeof key !== "string" || !/^(0|[1-9][0-9]*)$/.test(key) || Number(key) >= length) return reject();
  }
  const result: unknown[] = [];
  for (let index = 0; index < length; index++) {
    const field = descriptor(input, String(index));
    if (!field || !hasOwn(field, "value") || !field.enumerable) return reject();
    result.push(field.value);
  }
  return freeze(result);
}

/** Capture configured collaborator data without invoking accessors/proxies. */
export function captureValue(owner: unknown, key: string): unknown {
  const reject = (): never => { throw new DecisionEngineError("INVALID_INPUT"); };
  let cursor: object | null = passiveObject(owner, reject);
  for (let depth = 0; cursor !== null && depth < 16; depth++) {
    if (isProxy(cursor)) return reject();
    const field = descriptor(cursor, key);
    if (field) {
      if (!hasOwn(field, "value")) return reject();
      return field.value;
    }
    cursor = prototype(cursor);
  }
  return reject();
}
export function captureMethod<F extends (...args: never[]) => unknown>(owner: unknown, key: string): F {
  const method = captureValue(owner, key);
  if (typeof method !== "function" || isProxy(method)) throw new DecisionEngineError("INVALID_INPUT");
  // Only callable shape is asserted here; result data are decoded separately.
  return ((...args: never[]) => apply(method, owner, args)) as F;
}
