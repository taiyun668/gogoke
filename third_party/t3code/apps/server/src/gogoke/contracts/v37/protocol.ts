import type { JsonObject, JsonValue } from "../model.ts";
import { canonicalJson, parseStrictJsonBytes } from "../strictJson.ts";
import { isV37Operation, type V37Family, type V37Operation } from "./catalog.ts";

export const V37_SCHEMA = "gogoke.37.operations.v1" as const;
export const V37_U64_MAX = 18_446_744_073_709_551_615n;
const idPattern = /^[A-Za-z][A-Za-z0-9_-]{0,127}$/u;
const decimalPattern = /^(?:0|[1-9][0-9]*)$/u;

export type V37Status = "APPLIED" | "REPLAYED" | "DENIED" | "STALE" | "CONFLICT" |
  "UNSUPPORTED" | "UNKNOWN" | "FAILED";

/** Caller identity is deliberately absent from wire requests. The native host supplies it. */
export interface V37Request {
  readonly schema: typeof V37_SCHEMA;
  readonly family: V37Family;
  readonly operation: V37Operation;
  readonly requestId: string;
  readonly targetId: string;
  readonly domainId: string;
  readonly expectedRevision: string;
  readonly payload: JsonObject;
}

export interface V37Receipt {
  readonly schema: typeof V37_SCHEMA;
  readonly family: V37Family;
  readonly operation: V37Operation;
  readonly requestId: string;
  readonly targetId: string;
  readonly status: V37Status;
  readonly previousRevision: string;
  readonly revision: string;
  readonly result: JsonObject;
}

export class V37ProtocolError extends Error {
  override readonly name = "V37ProtocolError";
  readonly code: string;
  constructor(code: string, detail: string) { super(`${code}: ${detail}`); this.code = code; }
}

function object(value: JsonValue, path: string): JsonObject {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    throw new V37ProtocolError("INVALID_FIELD", `${path} must be an object`);
  }
  return value as JsonObject;
}

function exact(value: JsonObject, names: readonly string[]): void {
  const keys = Object.keys(value);
  if (keys.length !== names.length || keys.some((key) => !names.includes(key))) {
    throw new V37ProtocolError("INVALID_FIELDS", `expected ${names.join(",")}`);
  }
}

function id(value: JsonValue, path: string): string {
  if (typeof value !== "string" || !idPattern.test(value)) {
    throw new V37ProtocolError("INVALID_FIELD", `${path} must be a canonical ID`);
  }
  return value;
}

function revision(value: JsonValue, path: string): string {
  if (typeof value !== "string" || !decimalPattern.test(value) || BigInt(value) > V37_U64_MAX) {
    throw new V37ProtocolError("INVALID_REVISION", `${path} must be a uint64 decimal string`);
  }
  return value;
}

function pair(value: JsonObject): { family: V37Family; operation: V37Operation } {
  if (typeof value.family !== "string" || typeof value.operation !== "string" ||
      !isV37Operation(value.family, value.operation)) {
    throw new V37ProtocolError("UNKNOWN_OPERATION", "operation is outside the frozen set");
  }
  return { family: value.family, operation: value.operation as V37Operation };
}

function schema(value: JsonObject): void {
  if (value.schema !== V37_SCHEMA) {
    throw new V37ProtocolError("UNKNOWN_SCHEMA", "only the frozen v1 schema is executable");
  }
}

export function decodeV37Request(bytes: Uint8Array): V37Request {
  const value = object(parseStrictJsonBytes(bytes), "request");
  exact(value, ["schema", "family", "operation", "requestId", "targetId", "domainId", "expectedRevision", "payload"]);
  schema(value);
  const operation = pair(value);
  return Object.freeze({ schema: V37_SCHEMA, ...operation,
    requestId: id(value.requestId!, "requestId"), targetId: id(value.targetId!, "targetId"),
    domainId: id(value.domainId!, "domainId"),
    expectedRevision: revision(value.expectedRevision!, "expectedRevision"),
    payload: object(value.payload!, "payload"),
  });
}

export function encodeV37Request(request: V37Request): Uint8Array {
  const bytes = new TextEncoder().encode(canonicalJson(request as unknown as JsonValue));
  decodeV37Request(bytes);
  return bytes;
}

export function decodeV37Receipt(bytes: Uint8Array): V37Receipt {
  const value = object(parseStrictJsonBytes(bytes), "receipt");
  exact(value, ["schema", "family", "operation", "requestId", "targetId", "status", "previousRevision", "revision", "result"]);
  schema(value);
  const operation = pair(value);
  const statuses: readonly string[] = ["APPLIED", "REPLAYED", "DENIED", "STALE", "CONFLICT", "UNSUPPORTED", "UNKNOWN", "FAILED"];
  if (typeof value.status !== "string" || !statuses.includes(value.status)) {
    throw new V37ProtocolError("INVALID_STATUS", "status is outside the closed set");
  }
  return Object.freeze({ schema: V37_SCHEMA, ...operation,
    requestId: id(value.requestId!, "requestId"), targetId: id(value.targetId!, "targetId"),
    status: value.status as V37Status,
    previousRevision: revision(value.previousRevision!, "previousRevision"),
    revision: revision(value.revision!, "revision"),
    result: object(value.result!, "result"),
  });
}

export function encodeV37Receipt(receipt: V37Receipt): Uint8Array {
  const bytes = new TextEncoder().encode(canonicalJson(receipt as unknown as JsonValue));
  decodeV37Receipt(bytes);
  return bytes;
}

export interface V37TrustedCaller {
  readonly principalId: string;
  readonly seatId: string;
  readonly domainId: string;
  readonly role: "user" | "lead" | "seat" | "host";
  readonly policyRevision: string;
  readonly revocationHead: string;
}

/** A real port must obtain this context from the native issuer on every call. */
export interface V37Port {
  execute(requestBytes: Uint8Array): Promise<Uint8Array>;
}

/** Product default until H supplies native issuer, atomic store, and OS permission enforcement. */
export class V37UnwiredPort implements V37Port {
  async execute(_bytes: Uint8Array): Promise<Uint8Array> {
    throw new V37ProtocolError("UNWIRED", "trusted native operation is unavailable");
  }
}
