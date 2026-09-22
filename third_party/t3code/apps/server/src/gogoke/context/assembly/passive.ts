import * as Crypto from "node:crypto";
import * as UtilTypes from "node:util/types";
import type { JsonValue } from "../../contracts/model.ts";
import { ContextAssemblyError, type AssemblyErrorCode } from "./model.ts";

export function fail(code: AssemblyErrorCode): never {
  throw new ContextAssemblyError(code);
}

export function ownRecord(
  value: unknown, required: readonly string[], optional: readonly string[] = [],
  code: AssemblyErrorCode = "AUTHORITY_PROTOCOL_ERROR",
): Readonly<Record<string, unknown>> {
  if (typeof value !== "object" || value === null || UtilTypes.isProxy(value) || Array.isArray(value)) {
    return fail(code);
  }
  const proto = Object.getPrototypeOf(value);
  if (proto !== Object.prototype && proto !== null) return fail(code);
  const keys = Reflect.ownKeys(value);
  if (keys.some(key => typeof key !== "string" || (!required.includes(key) && !optional.includes(key))) ||
      required.some(key => !keys.includes(key))) return fail(code);
  const result: Record<string, unknown> = Object.create(null);
  for (const key of keys as string[]) {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (!descriptor || !Object.hasOwn(descriptor, "value") || !descriptor.enumerable) return fail(code);
    result[key] = descriptor.value;
  }
  return Object.freeze(result);
}

export function ownArray(value: unknown, code: AssemblyErrorCode = "AUTHORITY_PROTOCOL_ERROR"):
ReadonlyArray<unknown> {
  if (UtilTypes.isProxy(value) || !Array.isArray(value) || Object.getPrototypeOf(value) !== Array.prototype) {
    return fail(code);
  }
  const length = value.length;
  const keys = Reflect.ownKeys(value);
  if (keys.some(key => typeof key !== "string" || (key !== "length" &&
      (!/^(?:0|[1-9][0-9]*)$/.test(key) || Number(key) >= length)))) return fail(code);
  const result: unknown[] = [];
  for (let index = 0; index < length; index++) {
    const descriptor = Object.getOwnPropertyDescriptor(value, String(index));
    if (!descriptor || !Object.hasOwn(descriptor, "value") || !descriptor.enumerable) return fail(code);
    const own: PropertyDescriptor = Object.create(null);
    own.value = descriptor.value;
    own.enumerable = true;
    Object.defineProperty(result, String(index), own);
  }
  return Object.freeze(result);
}

export function identifier(value: unknown, code: AssemblyErrorCode = "AUTHORITY_PROTOCOL_ERROR"): string {
  if (typeof value !== "string" || !/^[A-Za-z0-9][A-Za-z0-9._:@/-]{0,255}$/.test(value)) return fail(code);
  return value;
}

export function u64(value: unknown, code: AssemblyErrorCode = "AUTHORITY_PROTOCOL_ERROR"): string {
  if (typeof value !== "string" || value.length > 20 || !/^(?:0|[1-9][0-9]*)$/.test(value) ||
      BigInt(value) > 18_446_744_073_709_551_615n) return fail(code);
  return value;
}

export function count(value: unknown, code: AssemblyErrorCode = "AUTHORITY_PROTOCOL_ERROR"): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) return fail(code);
  return value;
}

export function digest(value: unknown): string {
  if (typeof value !== "string" || !/^sha256:[0-9a-f]{64}$/.test(value)) return fail("AUTHORITY_PROTOCOL_ERROR");
  return value;
}

/** Bounded passive JSON, used only for metadata; no getters/toJSON/coercion. */
export function jsonData(value: unknown): JsonValue {
  const ancestors = new WeakSet<object>();
  let nodes = 0;
  function visit(input: unknown, depth: number): JsonValue {
    if (++nodes > 100_000 || depth > 64) return fail("AUTHORITY_PROTOCOL_ERROR");
    if (input === null || typeof input === "string" || typeof input === "boolean") return input;
    if (typeof input === "number") {
      if (!Number.isFinite(input) || (Number.isInteger(input) && !Number.isSafeInteger(input))) {
        return fail("AUTHORITY_PROTOCOL_ERROR");
      }
      return input;
    }
    if (typeof input !== "object" || UtilTypes.isProxy(input) || ancestors.has(input)) {
      return fail("AUTHORITY_PROTOCOL_ERROR");
    }
    ancestors.add(input);
    try {
      if (Array.isArray(input)) return Object.freeze(ownArray(input).map(item => visit(item, depth + 1)));
      const proto = Object.getPrototypeOf(input);
      if (proto !== Object.prototype && proto !== null) return fail("AUTHORITY_PROTOCOL_ERROR");
      const keys = Reflect.ownKeys(input);
      if (keys.some(key => typeof key !== "string")) return fail("AUTHORITY_PROTOCOL_ERROR");
      const raw = ownRecord(input, keys as string[]);
      const output: Record<string, JsonValue> = Object.create(null);
      for (const key of keys as string[]) output[key] = visit(raw[key], depth + 1);
      return Object.freeze(output);
    } finally { ancestors.delete(input); }
  }
  return visit(value, 0);
}

/** This module's deterministic metadata encoding; never JSON.stringify an object. */
export function canonical(value: unknown): string {
  const passive = jsonData(value);
  function encode(item: JsonValue): string {
    if (item === null || typeof item !== "object") return JSON.stringify(item);
    if (Array.isArray(item)) return `[${item.map(encode).join(",")}]`;
    const object = item as Readonly<Record<string, JsonValue>>;
    return `{${Object.keys(object).sort().map(key => `${JSON.stringify(key)}:${encode(object[key]!)}`).join(",")}}`;
  }
  return encode(passive);
}

export function hashText(text: string): string {
  return `sha256:${Crypto.createHash("sha256").update(text, "utf8").digest("hex")}`;
}

export function hashData(value: unknown): string { return hashText(canonical(value)); }

/** Capture plain host-owned methods once; no mutable/global fallback authority. */
export function captureMethods<T extends object>(port: T, names: readonly (keyof T & string)[]): T {
  const raw = ownRecord(port, names, [], "INVALID_INPUT");
  const result: Record<string, unknown> = Object.create(null);
  for (const name of names) {
    const method = raw[name];
    if (typeof method !== "function" || UtilTypes.isProxy(method)) return fail("INVALID_INPUT");
    result[name] = (...args: unknown[]) => Reflect.apply(method, port, args);
  }
  return Object.freeze(result) as T;
}
