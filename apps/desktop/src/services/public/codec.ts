import {
  DECIMAL_U64_MAX,
  PUBLIC_SCHEMA_VERSION,
  type JsonValue,
  type PublicDocument,
} from "@/types/public";

export type PublicCodecErrorCode =
  | "InvalidUtf8" | "FrameTooLarge" | "DuplicateKey" | "NonCanonicalNumber"
  | "InvalidJson" | "MissingRequiredField" | "UnknownMajorVersion"
  | "ValueOverflow" | "BoundsExceeded" | "SchemaViolation";

export class PublicCodecError extends Error {
  constructor(public readonly code: PublicCodecErrorCode, message: string) {
    super(message);
    this.name = "PublicCodecError";
  }
}

const MAX_FRAME_BYTES = 4 * 1024 * 1024;
const UINT32_MAX = 4_294_967_295n;
const I64_MIN = -9_223_372_036_854_775_808n;
const DECIMAL_U64_PATTERN = /^(0|[1-9][0-9]{0,19})$/;
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const UTC_TIMESTAMP_PATTERN = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:Z|\+00:00)$/;
const textEncoder = new TextEncoder();

type JsonObject = { [key: string]: JsonValue };

const fail = (code: PublicCodecErrorCode, message: string): never => {
  throw new PublicCodecError(code, message);
};

class StrictJsonParser {
  private index = 0;
  private objectDepth = 0;

  constructor(private readonly text: string, private readonly duplicateScanOnly = false) {}

  parse(): JsonValue {
    const value = this.value();
    this.ws();
    if (this.index !== this.text.length) fail("InvalidJson", "trailing JSON content");
    return value;
  }

  private value(field?: string): JsonValue {
    this.ws();
    const char = this.text[this.index];
    if (char === "{") return this.object();
    if (char === "[") return this.array();
    if (char === '"') return this.string();
    if (this.text.startsWith("true", this.index)) return this.literal("true", true);
    if (this.text.startsWith("false", this.index)) return this.literal("false", false);
    if (this.text.startsWith("null", this.index)) return this.literal("null", null);
    if (char === "-" || (char !== undefined && char >= "0" && char <= "9")) return this.integer(field);
    return fail("InvalidJson", `unexpected token at ${this.index}`);
  }

  private object(): JsonObject {
    this.index++;
    this.objectDepth++;
    const result: JsonObject = Object.create(null) as JsonObject;
    const keys = new Set<string>();
    this.ws();
    if (this.text[this.index] === "}") { this.index++; this.objectDepth--; return result; }
    for (;;) {
      this.ws();
      if (this.text[this.index] !== '"') fail("InvalidJson", "object key must be a string");
      const key = this.string();
      if (keys.has(key)) fail("DuplicateKey", `duplicate key: ${key}`);
      keys.add(key);
      this.ws();
      if (this.text[this.index++] !== ":") fail("InvalidJson", "missing colon");
      result[key] = this.value(this.objectDepth === 1 ? key : undefined);
      this.ws();
      const next = this.text[this.index++];
      if (next === "}") { this.objectDepth--; return result; }
      if (next !== ",") fail("InvalidJson", "missing comma");
    }
  }

  private array(): JsonValue[] {
    this.index++;
    const result: JsonValue[] = [];
    this.ws();
    if (this.text[this.index] === "]") { this.index++; return result; }
    for (;;) {
      result.push(this.value());
      this.ws();
      const next = this.text[this.index++];
      if (next === "]") return result;
      if (next !== ",") fail("InvalidJson", "missing comma");
    }
  }

  private string(): string {
    const start = this.index;
    this.index++;
    let escaped = false;
    while (this.index < this.text.length) {
      const char = this.text[this.index++];
      if (!escaped && char === '"') {
        try {
          const decoded = JSON.parse(this.text.slice(start, this.index)) as string;
          for (let offset = 0; offset < decoded.length; offset++) {
            const code = decoded.charCodeAt(offset);
            if (code >= 0xd800 && code <= 0xdbff) {
              const trailing = decoded.charCodeAt(offset + 1);
              if (!(trailing >= 0xdc00 && trailing <= 0xdfff)) fail("InvalidJson", "lone UTF-16 surrogate");
              offset++;
            } else if (code >= 0xdc00 && code <= 0xdfff) {
              fail("InvalidJson", "lone UTF-16 surrogate");
            }
          }
          return decoded;
        }
        catch { fail("InvalidJson", "invalid string escape"); }
      }
      if (!escaped && char === "\\") escaped = true;
      else escaped = false;
    }
    return fail("InvalidJson", "unterminated string");
  }

  private integer(field?: string): bigint {
    const rest = this.text.slice(this.index);
    if (this.duplicateScanOnly) {
      const jsonNumber = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/.exec(rest);
      if (!jsonNumber) return fail("InvalidJson", "invalid number");
      this.index += jsonNumber[0].length;
      return 0n;
    }
    const match = /^-?(?:0|[1-9][0-9]*)/.exec(rest);
    if (!match) return fail("InvalidJson", "invalid number");
    const token = match[0];
    const next = rest[token.length];
    if (next !== undefined && (next === "." || next === "e" || next === "E" || (next >= "0" && next <= "9"))) {
      const candidate = rest.split(/[\s,\]}]/, 1)[0];
      const numeric = Number(candidate);
      if (field === "schemaVersion" && Number.isFinite(numeric) && !Number.isInteger(numeric)) {
        fail("SchemaViolation", "schemaVersion must be a positive integer");
      }
      fail("NonCanonicalNumber", `non-canonical JSON number: ${candidate}`);
    }
    if (token === "-0") fail("NonCanonicalNumber", "negative zero is forbidden");
    this.index += token.length;
    try {
      const value = BigInt(token);
      if (value < I64_MIN || value > DECIMAL_U64_MAX) {
        return fail("NonCanonicalNumber", "JSON integer is outside the i64/u64 domain");
      }
      return value;
    } catch { return fail("NonCanonicalNumber", "JSON integer is outside the i64/u64 domain"); }
  }

  private literal<T>(token: string, value: T): T { this.index += token.length; return value; }

  private ws(): void {
    while (this.text[this.index] === " " || this.text[this.index] === "\t" || this.text[this.index] === "\r" || this.text[this.index] === "\n") this.index++;
  }
}

const assertNoDuplicateKeys = (text: string): void => {
  try {
    new StrictJsonParser(text, true).parse();
  } catch (error) {
    if (error instanceof PublicCodecError && error.code === "DuplicateKey") throw error;
  }
};

const objectValue = (value: JsonValue, name: string): JsonObject => {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail("SchemaViolation", `${name} must be an object`);
  return value as JsonObject;
};

const has = (object: JsonObject, key: string): boolean => Object.prototype.hasOwnProperty.call(object, key);
const required = (object: JsonObject, fields: readonly string[]): void => {
  for (const field of fields) if (!has(object, field)) fail("MissingRequiredField", `${field} is required`);
};
const exactKeys = (object: JsonObject, allowed: readonly string[]): void => {
  const set = new Set(allowed);
  for (const key of Object.keys(object)) if (!set.has(key)) fail("SchemaViolation", `unknown field: ${key}`);
};
const stringValue = (value: JsonValue, field: string, nonEmpty = false): string => {
  if (typeof value !== "string" || (nonEmpty && value.length === 0)) fail("SchemaViolation", `${field} must be ${nonEmpty ? "a non-empty " : "a "}string`);
  return value as string;
};
const nullableString = (value: JsonValue, field: string): string | null => {
  if (value !== null && typeof value !== "string") fail("SchemaViolation", `${field} must be a string or null`);
  return value as string | null;
};
const utcTimestamp = (value: JsonValue, field: string): string => {
  const text = stringValue(value, field);
  const match = UTC_TIMESTAMP_PATTERN.exec(text);
  if (!match) return fail("SchemaViolation", `${field} must be a valid UTC RFC3339 timestamp`);
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  const minute = Number(match[5]);
  const second = Number(match[6]);
  const leapYear = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const daysInMonth = [31, leapYear ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  if (month < 1 || month > 12 || day < 1 || day > daysInMonth[month - 1] || hour > 23 || minute > 59 || second > 59) {
    fail("SchemaViolation", `${field} must be a valid UTC RFC3339 timestamp`);
  }
  return text;
};
const nullableUtcTimestamp = (value: JsonValue, field: string): string | null => value === null ? null : utcTimestamp(value, field);
const uuid = (value: JsonValue, field: string): string => {
  const text = stringValue(value, field);
  if (!UUID_PATTERN.test(text)) fail("SchemaViolation", `${field} must be a canonical lowercase UUID`);
  return text;
};
const nullableUuid = (value: JsonValue, field: string): string | null => value === null ? null : uuid(value, field);
const enumValue = <T extends string>(value: JsonValue, field: string, allowed: readonly T[]): T => {
  const text = stringValue(value, field);
  if (!(allowed as readonly string[]).includes(text)) fail("SchemaViolation", `${field} has an invalid enum value`);
  return text as T;
};
const schemaVersion = (value: JsonValue): 1 => {
  if (value !== 1n && value !== PUBLIC_SCHEMA_VERSION) {
    if (typeof value === "bigint") {
      if (value > 1n) fail("UnknownMajorVersion", "unsupported schemaVersion");
      fail("SchemaViolation", "schemaVersion must be a positive integer");
    }
    if (typeof value === "number") {
      if (Number.isInteger(value) && value > 1) fail("UnknownMajorVersion", "unsupported schemaVersion");
      fail("SchemaViolation", "schemaVersion must be a positive integer");
    }
    fail("SchemaViolation", "schemaVersion must be a positive integer");
  }
  return PUBLIC_SCHEMA_VERSION;
};
const decimalU64 = (value: JsonValue, field: string): string => {
  const text = stringValue(value, field);
  if (!DECIMAL_U64_PATTERN.test(text)) fail("NonCanonicalNumber", `${field} must be a canonical decimal u64 string`);
  try {
    if (BigInt(text) > DECIMAL_U64_MAX) fail("ValueOverflow", `${field} overflows u64`);
  } catch { fail("ValueOverflow", `${field} overflows u64`); }
  return text;
};
const integer = (value: JsonValue, field: string): bigint => {
  if (typeof value === "bigint") return value;
  if (typeof value === "number" && Number.isSafeInteger(value) && !Object.is(value, -0)) return BigInt(value);
  return fail("SchemaViolation", `${field} must be a JSON integer number`);
};
const nullableDecimalU64 = (value: JsonValue, field: string): string | null => value === null ? null : decimalU64(value, field);
const jsonObject = (value: JsonValue, field: string): JsonObject => objectValue(value, field);
const jsonArray = (value: JsonValue, field: string): JsonValue[] => {
  if (!Array.isArray(value)) fail("SchemaViolation", `${field} must be an array`);
  return value as JsonValue[];
};
const maxUtf8 = (value: string, bytes: number, field: string): string => {
  if (textEncoder.encode(value).byteLength > bytes) fail("BoundsExceeded", `${field} exceeds UTF-8 byte limit`);
  return value;
};

const sessionLifecycle = ["active", "archived"] as const;
const continuationMode = ["native", "fixed_context", "new"] as const;
const executionState = ["created", "running", "completed", "failed", "cancelled"] as const;
const acceptanceState = ["recorded", "queued", "dispatching", "accepted", "acceptance_unknown", "rejected", "cancelled"] as const;
const capabilityState = ["supported", "unsupported", "unknown"] as const;
const humanActionState = ["pending", "resolved", "expired", "cancelled"] as const;
const eventKind = ["session_updated", "binding_updated", "delivery_updated", "execution_started", "execution_output_delta", "execution_completed", "execution_failed", "execution_cancelled", "human_action_requested", "human_action_resolved", "context_created", "delegation_result", "stream_gap", "diagnostic"] as const;
const errorCode = ["FEATURE_SEALED", "UNAUTHORIZED", "SCOPE_MISMATCH", "STALE_BINDING", "UNSUPPORTED", "CAPABILITY_UNKNOWN", "PROTOCOL_VERSION_MISMATCH", "INVALID_FRAME", "OPERATION_CONFLICT", "ACCEPTANCE_UNKNOWN", "ROOT_UNAVAILABLE", "ROOT_IN_USE", "ROOT_ID_MISMATCH", "UNSUPPORTED_ROOT", "STORE_CORRUPT", "STORE_WRITE_FAILED", "DURABILITY_UNKNOWN", "PROCESS_IDENTITY_UNKNOWN", "STOP_UNCONFIRMED", "LEGACY_ROUTE_DISABLED", "QUEUE_FULL", "RESYNC_REQUIRED", "CONFIG_CHANGED", "CANCELLED"] as const;
const observationState = ["observed", "not_observed", "unknown"] as const;
const recordedState = ["recorded", "not_recorded", "unknown"] as const;
const durabilityAckState = ["confirmed", "failed", "unknown"] as const;
const custodyState = ["retained", "released", "transferred", "unknown"] as const;
const sideEffectState = ["none", "possible", "confirmed", "unknown"] as const;

const validateNested = (object: JsonObject, name: string): JsonObject => {
  if (name === "ExecutableId") { required(object, ["path", "hash", "version", "platform"]); exactKeys(object, ["path", "hash", "version", "platform"]); for (const field of ["path", "hash", "version", "platform"]) stringValue(object[field], field); return object; }
  if (name === "CapabilityEvidence") { required(object, ["state", "reason", "evidenceRef"]); exactKeys(object, ["state", "reason", "evidenceRef"]); enumValue(object.state, "state", capabilityState); stringValue(object.reason, "reason"); nullableString(object.evidenceRef, "evidenceRef"); return object; }
  if (name === "ContextItem") { required(object, ["ref", "digest", "visibility"]); exactKeys(object, ["ref", "digest", "visibility"]); for (const field of ["ref", "digest", "visibility"]) stringValue(object[field], field, true); return object; }
  if (name === "OwnershipIssue") { required(object, ["code", "stage", "message"]); exactKeys(object, ["code", "stage", "message"]); enumValue(object.code, "code", errorCode); stringValue(object.stage, "stage"); stringValue(object.message, "message"); return object; }
  if (name === "CommandError") { required(object, ["code", "stage", "retryable", "sideEffectState", "correlationId", "message"]); exactKeys(object, ["code", "stage", "retryable", "sideEffectState", "correlationId", "message"]); enumValue(object.code, "code", errorCode); stringValue(object.stage, "stage"); if (typeof object.retryable !== "boolean") fail("SchemaViolation", "retryable must be a boolean"); enumValue(object.sideEffectState, "sideEffectState", sideEffectState); uuid(object.correlationId, "correlationId"); stringValue(object.message, "message"); return object; }
  return fail("SchemaViolation", `unsupported nested schema ${name}`);
};

const validateDocument = (value: JsonValue): PublicDocument => {
  const object = objectValue(value, "public document");
  required(object, ["schemaVersion"]);
  schemaVersion(object.schemaVersion);

  if (has(object, "title") || has(object, "lifecycle") || has(object, "privacyDomainId")) {
    required(object, ["schemaVersion", "sessionId", "scopeId", "privacyDomainId", "title", "lifecycle", "revision"]);
    exactKeys(object, ["schemaVersion", "sessionId", "scopeId", "privacyDomainId", "title", "lifecycle", "revision"]);
    uuid(object.sessionId, "sessionId"); uuid(object.scopeId, "scopeId"); uuid(object.privacyDomainId, "privacyDomainId");
    maxUtf8(stringValue(object.title, "title"), 256, "title"); enumValue(object.lifecycle, "lifecycle", sessionLifecycle); decimalU64(object.revision, "revision");
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (has(object, "driverId") || has(object, "continuationMode") || has(object, "nativeSessionId")) {
    required(object, ["schemaVersion", "bindingId", "sessionId", "driverId", "instanceId", "profileRevision", "authRevision", "domainId", "generation", "nativeSessionId", "continuationMode"]);
    exactKeys(object, ["schemaVersion", "bindingId", "sessionId", "driverId", "instanceId", "profileRevision", "authRevision", "domainId", "generation", "nativeSessionId", "continuationMode"]);
    for (const field of ["bindingId", "sessionId", "instanceId", "domainId"]) uuid(object[field], field);
    stringValue(object.driverId, "driverId", true); decimalU64(object.profileRevision, "profileRevision"); decimalU64(object.authRevision, "authRevision"); decimalU64(object.generation, "generation"); nullableString(object.nativeSessionId, "nativeSessionId"); enumValue(object.continuationMode, "continuationMode", continuationMode);
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (has(object, "createdAt") || has(object, "resultRef")) {
    required(object, ["schemaVersion", "executionId", "sessionId", "bindingId", "generation", "state", "createdAt", "completedAt", "resultRef"]);
    exactKeys(object, ["schemaVersion", "executionId", "sessionId", "bindingId", "generation", "state", "createdAt", "completedAt", "resultRef"]);
    for (const field of ["executionId", "sessionId", "bindingId"]) uuid(object[field], field);
    decimalU64(object.generation, "generation"); const state = enumValue(object.state, "state", executionState); utcTimestamp(object.createdAt, "createdAt"); const completedAt = nullableUtcTimestamp(object.completedAt, "completedAt"); nullableString(object.resultRef, "resultRef");
    if ((state === "completed" || state === "failed" || state === "cancelled") && completedAt === null) fail("SchemaViolation", "terminal execution requires completedAt");
    if ((state === "created" || state === "running") && completedAt !== null) fail("SchemaViolation", "non-terminal execution must not have completedAt");
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (has(object, "operationId") || has(object, "requestFingerprint") || has(object, "acceptanceState")) {
    required(object, ["schemaVersion", "operationId", "executionId", "sessionId", "bindingId", "generation", "requestFingerprint", "intentKind", "acceptanceState", "durableReceiptId", "nativeReceipt"]);
    exactKeys(object, ["schemaVersion", "operationId", "executionId", "sessionId", "bindingId", "generation", "requestFingerprint", "intentKind", "acceptanceState", "durableReceiptId", "nativeReceipt"]);
    for (const field of ["operationId", "executionId", "sessionId", "bindingId"]) uuid(object[field], field);
    decimalU64(object.generation, "generation"); stringValue(object.requestFingerprint, "requestFingerprint", true); stringValue(object.intentKind, "intentKind", true); enumValue(object.acceptanceState, "acceptanceState", acceptanceState); nullableUuid(object.durableReceiptId, "durableReceiptId");
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (has(object, "eventId") || has(object, "kind") || has(object, "payload")) {
    required(object, ["schemaVersion", "eventId", "sessionId", "executionId", "bindingId", "generation", "streamEpoch", "sequence", "kind", "payload"]);
    exactKeys(object, ["schemaVersion", "eventId", "sessionId", "executionId", "bindingId", "generation", "streamEpoch", "sequence", "kind", "payload"]);
    uuid(object.eventId, "eventId"); uuid(object.sessionId, "sessionId"); nullableUuid(object.executionId, "executionId"); nullableUuid(object.bindingId, "bindingId"); decimalU64(object.generation, "generation"); decimalU64(object.streamEpoch, "streamEpoch"); decimalU64(object.sequence, "sequence"); enumValue(object.kind, "kind", eventKind);
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (has(object, "executableId") || has(object, "snapshotId") || has(object, "capabilities")) {
    required(object, ["schemaVersion", "snapshotId", "instanceId", "executableId", "mode", "profileRevision", "authRevision", "observedAt", "expiresAt", "evidenceSource", "capabilities"]);
    exactKeys(object, ["schemaVersion", "snapshotId", "instanceId", "executableId", "mode", "profileRevision", "authRevision", "observedAt", "expiresAt", "evidenceSource", "capabilities"]);
    uuid(object.snapshotId, "snapshotId"); uuid(object.instanceId, "instanceId"); validateNested(objectValue(object.executableId, "executableId"), "ExecutableId"); stringValue(object.mode, "mode", true); decimalU64(object.profileRevision, "profileRevision"); decimalU64(object.authRevision, "authRevision"); utcTimestamp(object.observedAt, "observedAt"); utcTimestamp(object.expiresAt, "expiresAt"); stringValue(object.evidenceSource, "evidenceSource", true);
    const capabilities = jsonObject(object.capabilities, "capabilities"); for (const [key, entry] of Object.entries(capabilities)) { stringValue(key, "capability name", true); validateNested(objectValue(entry, `capabilities.${key}`), "CapabilityEvidence"); }
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (has(object, "packageId") || has(object, "items") || has(object, "recipient")) {
    required(object, ["schemaVersion", "packageId", "recipient", "scopeId", "domainId", "taskId", "sourceVersion", "items", "permissionCeiling", "expiresAt"]);
    exactKeys(object, ["schemaVersion", "packageId", "recipient", "scopeId", "domainId", "taskId", "sourceVersion", "items", "permissionCeiling", "expiresAt"]);
    uuid(object.packageId, "packageId"); stringValue(object.recipient, "recipient", true); uuid(object.scopeId, "scopeId"); uuid(object.domainId, "domainId"); stringValue(object.taskId, "taskId", true); stringValue(object.sourceVersion, "sourceVersion", true); const items = jsonArray(object.items, "items"); if (items.length > 128) fail("BoundsExceeded", "items exceeds maxItems"); for (const item of items) validateNested(objectValue(item, "items[]"), "ContextItem"); jsonObject(object.permissionCeiling, "permissionCeiling"); utcTimestamp(object.expiresAt, "expiresAt");
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (has(object, "requestId") || has(object, "allowedAnswers") || has(object, "continuationId")) {
    required(object, ["schemaVersion", "requestId", "executionId", "bindingId", "generation", "continuationId", "domainId", "expiresAt", "allowedAnswers", "permissionCeiling", "state"]);
    exactKeys(object, ["schemaVersion", "requestId", "executionId", "bindingId", "generation", "continuationId", "domainId", "expiresAt", "allowedAnswers", "permissionCeiling", "state"]);
    uuid(object.requestId, "requestId"); uuid(object.executionId, "executionId"); uuid(object.bindingId, "bindingId"); decimalU64(object.generation, "generation"); stringValue(object.continuationId, "continuationId", true); uuid(object.domainId, "domainId"); utcTimestamp(object.expiresAt, "expiresAt"); const answers = jsonArray(object.allowedAnswers, "allowedAnswers"); if (answers.length === 0) fail("SchemaViolation", "allowedAnswers must not be empty"); for (const answer of answers) stringValue(answer, "allowedAnswers[]", true); jsonObject(object.permissionCeiling, "permissionCeiling"); enumValue(object.state, "state", humanActionState);
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (has(object, "hostId") || has(object, "pid") || has(object, "processHandleRef")) {
    required(object, ["schemaVersion", "hostId", "instanceId", "processHandleRef", "pid", "startedAtTicks", "launchEpoch", "imageDigest", "containmentId"]);
    exactKeys(object, ["schemaVersion", "hostId", "instanceId", "processHandleRef", "pid", "startedAtTicks", "launchEpoch", "imageDigest", "containmentId"]);
    uuid(object.hostId, "hostId"); uuid(object.instanceId, "instanceId"); stringValue(object.processHandleRef, "processHandleRef", true); const pid = integer(object.pid, "pid"); if (pid < 0n || pid > UINT32_MAX) fail("ValueOverflow", "pid is outside u32"); nullableDecimalU64(object.startedAtTicks, "startedAtTicks"); decimalU64(object.launchEpoch, "launchEpoch"); stringValue(object.imageDigest, "imageDigest", true); nullableString(object.containmentId, "containmentId");
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION, pid: Number(pid) } as unknown as PublicDocument;
  }
  if (has(object, "custodyState") || has(object, "observation") || has(object, "errors")) {
    required(object, ["schemaVersion", "observation", "inMemoryState", "persistedState", "durabilityAck", "custodyState", "errors"]);
    exactKeys(object, ["schemaVersion", "observation", "inMemoryState", "persistedState", "durabilityAck", "custodyState", "errors"]);
    enumValue(object.observation, "observation", observationState); enumValue(object.inMemoryState, "inMemoryState", recordedState); enumValue(object.persistedState, "persistedState", recordedState); enumValue(object.durabilityAck, "durabilityAck", durabilityAckState); enumValue(object.custodyState, "custodyState", custodyState); const errors = jsonArray(object.errors, "errors"); for (const entry of errors) validateNested(objectValue(entry, "errors[]"), "OwnershipIssue");
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (object.ok === true) {
    required(object, ["schemaVersion", "ok", "value", "receipt"]); exactKeys(object, ["schemaVersion", "ok", "value", "receipt"]); if (object.ok !== true) fail("SchemaViolation", "ok must be true");
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  if (object.ok === false) {
    required(object, ["schemaVersion", "ok", "error"]); exactKeys(object, ["schemaVersion", "ok", "error"]); if (object.ok !== false) fail("SchemaViolation", "ok must be false"); validateNested(objectValue(object.error, "error"), "CommandError");
    return { ...object, schemaVersion: PUBLIC_SCHEMA_VERSION } as unknown as PublicDocument;
  }
  return fail("MissingRequiredField", "unrecognized public document shape");
};

export const decodePublicJson = (bytes: Uint8Array): PublicDocument => {
  if (bytes.byteLength > MAX_FRAME_BYTES) fail("FrameTooLarge", "public frame exceeds 4 MiB");
  let text = "";
  try { text = new TextDecoder("utf-8", { fatal: true }).decode(bytes); }
  catch { fail("InvalidUtf8", "frame is not UTF-8"); }
  assertNoDuplicateKeys(text);
  return validateDocument(new StrictJsonParser(text).parse());
};

const compareUtf8 = (left: string, right: string): number => {
  const a = textEncoder.encode(left); const b = textEncoder.encode(right); const length = Math.min(a.length, b.length);
  for (let index = 0; index < length; index++) if (a[index] !== b[index]) return a[index] - b[index];
  return a.length - b.length;
};

const writeCanonical = (value: JsonValue): string => {
  if (value === null) return "null";
  if (typeof value === "string") return JSON.stringify(value);
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "bigint") {
    if (value < I64_MIN || value > DECIMAL_U64_MAX) return fail("NonCanonicalNumber", "JSON integer is outside the i64/u64 domain");
    return value.toString(10);
  }
  if (typeof value === "number") {
    if (!Number.isSafeInteger(value) || Object.is(value, -0)) return fail("NonCanonicalNumber", "JSON numbers must be safe integers");
    return String(value);
  }
  if (Array.isArray(value)) return `[${value.map(writeCanonical).join(",")}]`;
  const object = value as JsonObject;
  const entries = Object.keys(object).sort(compareUtf8).map((key) => `${JSON.stringify(key)}:${writeCanonical(object[key])}`);
  return `{${entries.join(",")}}`;
};

export const encodePublicJson = (document: PublicDocument): Uint8Array => {
  const normalized = validateDocument(document as unknown as JsonValue);
  const bytes = textEncoder.encode(writeCanonical(normalized as unknown as JsonValue));
  if (bytes.byteLength > MAX_FRAME_BYTES) fail("FrameTooLarge", "encoded public frame exceeds 4 MiB");
  return bytes;
};
