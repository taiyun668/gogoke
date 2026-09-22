import * as NodeCrypto from "node:crypto";
import * as NodeUtilTypes from "node:util/types";

import {
  canonicalJson,
  decodePublicObject,
  encodePublicObject,
  parseStrictJsonBytes,
  type DecodedPublicObject,
  type JsonObject,
  type JsonValue,
  type PublicObjectType,
} from "../../contracts/index.ts";

const U64_MAX = 18_446_744_073_709_551_615n;
const U64_PATTERN = /^(?:0|[1-9]\d*)$/;

export interface AtomicRecordInput {
  readonly domainId: string;
  readonly object: { readonly canonicalBytes: Uint8Array };
  readonly nativeIdentity?:
    | {
        readonly runtimeInstanceId: string;
        readonly nativeId: string;
      }
    | undefined;
  readonly event: {
    readonly eventId: string;
    readonly streamId: string;
    readonly expectedPreviousCounter: string | null;
    readonly counter: string;
    readonly eventType: string;
    readonly occurredAt: string;
    readonly canonicalBytes: Uint8Array;
  };
  readonly receipt: {
    readonly receiptId: string;
    readonly operationId: string;
    readonly receiptType: string;
    readonly recordedAt: string;
    readonly canonicalBytes: Uint8Array;
  };
}

interface AtomicRecordSnapshot {
  readonly domainId: string;
  readonly object: { readonly canonicalBytes: Uint8Array };
  readonly nativeIdentity:
    | { readonly runtimeInstanceId: string; readonly nativeId: string }
    | undefined;
  readonly event: {
    readonly eventId: string;
    readonly streamId: string;
    readonly expectedPreviousCounter: string | null;
    readonly counter: string;
    readonly eventType: string;
    readonly occurredAt: string;
    readonly canonicalBytes: Uint8Array;
  };
  readonly receipt: {
    readonly receiptId: string;
    readonly operationId: string;
    readonly receiptType: string;
    readonly recordedAt: string;
    readonly canonicalBytes: Uint8Array;
  };
}

interface PreparedAtomicRecord {
  readonly domainId: string;
  readonly objectType: PublicObjectType;
  readonly objectId: string;
  readonly objectVersion: string;
  readonly objectBytes: Uint8Array;
  readonly objectHash: string;
  readonly nativeIdentity:
    | { readonly runtimeInstanceId: string; readonly nativeId: string }
    | undefined;
  readonly eventId: string;
  readonly streamId: string;
  readonly expectedPreviousCounter: string | null;
  readonly counter: string;
  readonly eventType: string;
  readonly occurredAt: string;
  readonly eventBytes: Uint8Array;
  readonly eventHash: string;
  readonly receiptId: string;
  readonly operationId: string;
  readonly receiptType: string;
  readonly recordedAt: string;
  readonly receiptBytes: Uint8Array;
  readonly receiptHash: string;
  readonly operationFingerprint: string;
}

export interface SqliteDurability {
  readonly foreignKeys: number | boolean;
  readonly journalMode: string;
  readonly synchronous: number | string;
}

export type AtomicSqlValue = null | string | number | bigint | Uint8Array;

export interface ExistingAtomicReceipt {
  readonly domainId: string;
  readonly receiptId: string;
  readonly operationId: string;
  readonly eventId: string;
  readonly objectType: PublicObjectType;
  readonly objectId: string;
  readonly objectVersion: string;
  readonly objectHash: string;
  readonly eventHash: string;
  readonly receiptHash: string;
  readonly operationFingerprint: string;
}

export interface AtomicSqliteTransaction {
  /** Reads PRAGMAs from the same connection that owns the transaction. */
  readonly readDurability: () => Promise<SqliteDurability>;
  readonly queryReceiptByOperation: (
    domainId: string,
    operationId: string,
  ) => Promise<ExistingAtomicReceipt | null>;
  readonly execute: (
    statement: string,
    parameters: ReadonlyArray<AtomicSqlValue>,
  ) => Promise<{ readonly changes: number | bigint }>;
}

export interface AtomicSqliteDriver {
  /**
   * Trusted integration boundary, not a TypeScript proof. The future shared
   * production adapter must retain native-pinned DB custody internally, issue
   * BEGIN IMMEDIATE on that exact connection, COMMIT once, and ROLLBACK on
   * callback failure. This preparatory core provides no production adapter.
   */
  readonly withNativePinnedImmediateTransaction: <T>(
    operation: (transaction: AtomicSqliteTransaction) => Promise<T>,
  ) => Promise<T>;
}

export type AtomicRecordErrorCode =
  | "INVALID_RECORD"
  | "NON_CANONICAL_JSON"
  | "DURABILITY_CONTRACT_FAILED"
  | "COUNTER_CONFLICT"
  | "WRITE_CONFLICT"
  | "OPERATION_CONFLICT"
  | "ATOMIC_WRITE_FAILED"
  | "COMMIT_UNKNOWN";

export type AtomicRecordStage =
  | "snapshot"
  | "prepare"
  | "durability"
  | "operation-query"
  | "stream-counter"
  | "object"
  | "native-identity"
  | "event"
  | "receipt"
  | "commit";

export class AtomicRecordError extends Error {
  override readonly name = "AtomicRecordError";
  readonly code: AtomicRecordErrorCode;
  readonly stage: AtomicRecordStage;
  override readonly cause: unknown;

  constructor(
    code: AtomicRecordErrorCode,
    stage: AtomicRecordStage,
    detail: string,
    cause?: unknown,
  ) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.stage = stage;
    this.cause = cause;
  }
}

/** Adapter signal for a COMMIT whose outcome cannot be established locally. */
export class AtomicCommitUnknownError extends Error {
  override readonly name = "AtomicCommitUnknownError";
}

const invalidSnapshot = (path: string, detail: string): never => {
  throw new AtomicRecordError("INVALID_RECORD", "snapshot", `${path} ${detail}`);
};

function readPassiveRecord(
  value: unknown,
  path: string,
  required: ReadonlyArray<string>,
  optional: ReadonlyArray<string> = [],
): Readonly<Record<string, unknown>> {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    NodeUtilTypes.isProxy(value)
  ) {
    return invalidSnapshot(path, "must be a non-Proxy plain object");
  }
  if (Object.getPrototypeOf(value) !== Object.prototype) {
    return invalidSnapshot(path, "must have Object.prototype");
  }
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key === "symbol")) {
    return invalidSnapshot(path, "must not contain symbol keys");
  }
  const names = keys as ReadonlyArray<string>;
  const allowed = new Set([...required, ...optional]);
  const extra = names.find((name) => !allowed.has(name));
  if (extra !== undefined) return invalidSnapshot(`${path}.${extra}`, "is not allowed");
  const missing = required.find((name) => !names.includes(name));
  if (missing !== undefined) return invalidSnapshot(`${path}.${missing}`, "is required");

  const descriptors = Object.getOwnPropertyDescriptors(value);
  const snapshot: Record<string, unknown> = {};
  for (const name of names) {
    const descriptor = descriptors[name];
    if (descriptor === undefined || !("value" in descriptor) || descriptor.enumerable !== true) {
      return invalidSnapshot(`${path}.${name}`, "must be an enumerable data property");
    }
    snapshot[name] = descriptor.value;
  }
  return Object.freeze(snapshot);
}

function snapshotBytes(value: unknown, path: string): Uint8Array {
  if (
    typeof value !== "object" ||
    value === null ||
    NodeUtilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Uint8Array.prototype
  ) {
    return invalidSnapshot(path, "must be a non-Proxy Uint8Array");
  }
  const bytes = value as Uint8Array;
  // Inspect every own key and descriptor before touching buffer, byteLength,
  // byteOffset, length, iteration, or an indexed value. Canonical Uint8Array
  // indices are the only admitted own string properties.
  const keys = Reflect.ownKeys(bytes);
  const descriptors = Object.getOwnPropertyDescriptors(bytes);
  for (const key of keys) {
    if (typeof key === "symbol") return invalidSnapshot(path, "must not contain symbol keys");
    if (!/^(?:0|[1-9]\d*)$/u.test(key)) {
      return invalidSnapshot(`${path}.${key}`, "is not an admitted byte index");
    }
    const descriptor = descriptors[key];
    if (descriptor === undefined || !("value" in descriptor) || descriptor.enumerable !== true) {
      return invalidSnapshot(`${path}.${key}`, "must be an enumerable byte data property");
    }
  }

  const byteLength = bytes.byteLength;
  if (
    keys.length !== byteLength ||
    keys.some((key) => typeof key !== "string" || Number(key) >= byteLength)
  ) {
    return invalidSnapshot(path, "has an incomplete or out-of-range byte index set");
  }
  if (bytes.buffer instanceof SharedArrayBuffer) {
    return invalidSnapshot(path, "must not be backed by SharedArrayBuffer");
  }
  // This is the sole read of caller-owned bytes. The private clone is never
  // returned by writeAtomicRecord, so caller mutation cannot race validation.
  return Uint8Array.from(bytes);
}

const snapshotString = (value: unknown, path: string): string => {
  if (typeof value !== "string") return invalidSnapshot(path, "must be a string");
  return value;
};

function snapshotAtomicInput(input: AtomicRecordInput): AtomicRecordSnapshot {
  const root = readPassiveRecord(
    input,
    "input",
    ["domainId", "object", "event", "receipt"],
    ["nativeIdentity"],
  );
  const object = readPassiveRecord(root.object, "input.object", ["canonicalBytes"]);
  const event = readPassiveRecord(root.event, "input.event", [
    "eventId",
    "streamId",
    "expectedPreviousCounter",
    "counter",
    "eventType",
    "occurredAt",
    "canonicalBytes",
  ]);
  const receipt = readPassiveRecord(root.receipt, "input.receipt", [
    "receiptId",
    "operationId",
    "receiptType",
    "recordedAt",
    "canonicalBytes",
  ]);
  const nativeRecord =
    root.nativeIdentity === undefined
      ? undefined
      : readPassiveRecord(root.nativeIdentity, "input.nativeIdentity", [
          "runtimeInstanceId",
          "nativeId",
        ]);
  const expectedPreviousCounter = event.expectedPreviousCounter;
  if (expectedPreviousCounter !== null && typeof expectedPreviousCounter !== "string") {
    return invalidSnapshot("input.event.expectedPreviousCounter", "must be a string or null");
  }

  return Object.freeze({
    domainId: snapshotString(root.domainId, "input.domainId"),
    object: Object.freeze({
      canonicalBytes: snapshotBytes(object.canonicalBytes, "input.object.canonicalBytes"),
    }),
    nativeIdentity:
      nativeRecord === undefined
        ? undefined
        : Object.freeze({
            runtimeInstanceId: snapshotString(
              nativeRecord.runtimeInstanceId,
              "input.nativeIdentity.runtimeInstanceId",
            ),
            nativeId: snapshotString(nativeRecord.nativeId, "input.nativeIdentity.nativeId"),
          }),
    event: Object.freeze({
      eventId: snapshotString(event.eventId, "input.event.eventId"),
      streamId: snapshotString(event.streamId, "input.event.streamId"),
      expectedPreviousCounter,
      counter: snapshotString(event.counter, "input.event.counter"),
      eventType: snapshotString(event.eventType, "input.event.eventType"),
      occurredAt: snapshotString(event.occurredAt, "input.event.occurredAt"),
      canonicalBytes: snapshotBytes(event.canonicalBytes, "input.event.canonicalBytes"),
    }),
    receipt: Object.freeze({
      receiptId: snapshotString(receipt.receiptId, "input.receipt.receiptId"),
      operationId: snapshotString(receipt.operationId, "input.receipt.operationId"),
      receiptType: snapshotString(receipt.receiptType, "input.receipt.receiptType"),
      recordedAt: snapshotString(receipt.recordedAt, "input.receipt.recordedAt"),
      canonicalBytes: snapshotBytes(receipt.canonicalBytes, "input.receipt.canonicalBytes"),
    }),
  });
}

const requireText = (value: string, path: string): string => {
  if (value.length === 0 || value.trim() !== value || value.includes("\0")) {
    throw new AtomicRecordError("INVALID_RECORD", "prepare", `${path} is not a canonical ID`);
  }
  return value;
};

const requireTimestamp = (value: string, path: string): string => {
  requireText(value, path);
  if (
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/.test(value) ||
    Number.isNaN(Date.parse(value))
  ) {
    throw new AtomicRecordError("INVALID_RECORD", "prepare", `${path} is not a UTC timestamp`);
  }
  return value;
};

const decodeCounter = (value: string, path: string): bigint => {
  if (!U64_PATTERN.test(value)) {
    throw new AtomicRecordError(
      "INVALID_RECORD",
      "prepare",
      `${path} is not a canonical uint64 string`,
    );
  }
  const decoded = BigInt(value);
  if (decoded > U64_MAX) {
    throw new AtomicRecordError("INVALID_RECORD", "prepare", `${path} exceeds uint64`);
  }
  return decoded;
};

const sha256 = (bytes: Uint8Array): string =>
  `sha256:${NodeCrypto.createHash("sha256").update(bytes).digest("hex")}`;

const sameBytes = (left: Uint8Array, right: Uint8Array): boolean =>
  left.byteLength === right.byteLength && left.every((byte, index) => byte === right[index]);

const requireCanonicalJson = (bytes: Uint8Array, path: string): Uint8Array => {
  try {
    const value = parseStrictJsonBytes(bytes);
    const canonical = new TextEncoder().encode(canonicalJson(value));
    if (sameBytes(bytes, canonical)) return bytes;
    throw new AtomicRecordError(
      "NON_CANONICAL_JSON",
      "prepare",
      `${path} must already be strict canonical JSON`,
    );
  } catch (error) {
    if (error instanceof AtomicRecordError) throw error;
    throw new AtomicRecordError("INVALID_RECORD", "prepare", `${path} is not strict JSON`, error);
  }
};

interface ObjectRowBinding {
  readonly idFields: ReadonlyArray<string>;
  readonly versionFields: ReadonlyArray<string>;
  readonly domainField?: "domainId";
}

/** Every one of the fixed S1-01-C public objects has one explicit row rule. */
const OBJECT_ROW_BINDINGS: Readonly<Record<PublicObjectType, ObjectRowBinding>> = Object.freeze({
  RuntimeDriver: { idFields: ["driverId"], versionFields: ["adapterVersion"] },
  RuntimeInstance: { idFields: ["instanceId"], versionFields: ["authRevision"] },
  ModelRef: {
    idFields: ["runtimeInstanceId", "nativeModelId"],
    versionFields: ["resolvedVersion", "capabilityRevision"],
  },
  RoleSpec: { idFields: ["roleId"], versionFields: ["revision"] },
  Seat: { idFields: ["seatId"], versionFields: ["lifecycle"], domainField: "domainId" },
  ExecutionRecipe: { idFields: ["recipeId"], versionFields: ["revision"] },
  NativeBinding: {
    idFields: ["bindingId"],
    versionFields: ["generation"],
    domainField: "domainId",
  },
  ContextObject: {
    idFields: ["contextId"],
    versionFields: ["version", "accessPolicyRevision"],
    domainField: "domainId",
  },
  ContextManifest: {
    idFields: ["manifestId"],
    versionFields: ["bindingGeneration", "policyRevision", "manifestHash"],
    domainField: "domainId",
  },
  ExposureReceipt: { idFields: ["receiptId"], versionFields: ["generation"] },
  SessionLineage: { idFields: ["sessionId"], versionFields: ["sourceEpoch"] },
  DecisionRecord: {
    idFields: ["decisionId"],
    versionFields: ["stateViewHash", "candidateHash", "questionVersion", "state"],
  },
  OutcomeRecord: { idFields: ["outcomeId"], versionFields: ["revision"] },
  CalibrationProfile: {
    idFields: ["profileId"],
    versionFields: ["modelVersion", "questionViewVersion", "datasetSplitHash", "rubricHash"],
  },
  DreamRun: {
    idFields: ["runId"],
    versionFields: ["snapshotHash", "datasetSplitHash", "preemptionState"],
    domainField: "domainId",
  },
  DreamProposal: {
    idFields: ["proposalId"],
    versionFields: ["basePolicyRevision", "candidateHash", "state"],
  },
});

for (const binding of Object.values(OBJECT_ROW_BINDINGS)) {
  Object.freeze(binding.idFields);
  Object.freeze(binding.versionFields);
  Object.freeze(binding);
}

export const OBJECT_ROW_BINDING_TYPES = Object.freeze(
  Object.keys(OBJECT_ROW_BINDINGS) as ReadonlyArray<PublicObjectType>,
);

const canonicalFieldTuple = (
  object: JsonObject,
  fields: ReadonlyArray<string>,
  objectType: PublicObjectType,
): string =>
  canonicalJson(
    fields.map((field) => {
      const value = object[field];
      if (value === undefined) {
        throw new AtomicRecordError(
          "INVALID_RECORD",
          "prepare",
          `${objectType}.${field} is missing from the decoded authority object`,
        );
      }
      return value;
    }) as ReadonlyArray<JsonValue>,
  );

function deriveObjectRow(
  decoded: DecodedPublicObject,
  domainId: string,
): { readonly objectId: string; readonly objectVersion: string } {
  const objectType = decoded.value.objectType;
  const object = decoded.value.object as unknown as JsonObject;
  const binding = OBJECT_ROW_BINDINGS[objectType];
  if (binding.domainField !== undefined && object[binding.domainField] !== domainId) {
    throw new AtomicRecordError(
      "INVALID_RECORD",
      "prepare",
      `${objectType}.${binding.domainField} does not match the durable row domain`,
    );
  }
  return {
    objectId: canonicalFieldTuple(object, binding.idFields, objectType),
    objectVersion: canonicalFieldTuple(object, binding.versionFields, objectType),
  };
}

function prepareAtomicRecord(input: AtomicRecordSnapshot): PreparedAtomicRecord {
  let objectDecoded: DecodedPublicObject;
  let objectCanonical: Uint8Array;
  try {
    objectDecoded = decodePublicObject(input.object.canonicalBytes);
    objectCanonical = encodePublicObject(objectDecoded);
  } catch (error) {
    throw new AtomicRecordError(
      "INVALID_RECORD",
      "prepare",
      "object.canonicalBytes is not a valid public object contract",
      error,
    );
  }
  if (!sameBytes(input.object.canonicalBytes, objectCanonical)) {
    throw new AtomicRecordError(
      "NON_CANONICAL_JSON",
      "prepare",
      "object.canonicalBytes must already be canonical contract bytes",
    );
  }
  const domainId = requireText(input.domainId, "domainId");
  const row = deriveObjectRow(objectDecoded, domainId);
  const expected = input.event.expectedPreviousCounter;
  const counter = decodeCounter(input.event.counter, "event.counter");
  if (expected === null) {
    if (counter !== 0n) {
      throw new AtomicRecordError(
        "INVALID_RECORD",
        "prepare",
        "the first stream event must use counter 0",
      );
    }
  } else if (counter !== decodeCounter(expected, "event.expectedPreviousCounter") + 1n) {
    throw new AtomicRecordError(
      "INVALID_RECORD",
      "prepare",
      "event.counter must immediately follow expectedPreviousCounter",
    );
  }

  const objectBytes = input.object.canonicalBytes;
  const eventBytes = requireCanonicalJson(input.event.canonicalBytes, "event.canonicalBytes");
  const receiptBytes = requireCanonicalJson(input.receipt.canonicalBytes, "receipt.canonicalBytes");
  const nativeIdentity = input.nativeIdentity
    ? Object.freeze({
        runtimeInstanceId: requireText(
          input.nativeIdentity.runtimeInstanceId,
          "nativeIdentity.runtimeInstanceId",
        ),
        nativeId: requireText(input.nativeIdentity.nativeId, "nativeIdentity.nativeId"),
      })
    : undefined;
  const object = objectDecoded.value.object as unknown as JsonObject;
  if (objectDecoded.value.objectType === "NativeBinding") {
    if (
      nativeIdentity === undefined ||
      object.domainId !== domainId ||
      object.instanceId !== nativeIdentity.runtimeInstanceId ||
      object.nativeIdentity !== nativeIdentity.nativeId ||
      row.objectId !== canonicalJson([object.bindingId] as ReadonlyArray<JsonValue>) ||
      row.objectVersion !== canonicalJson([object.generation] as ReadonlyArray<JsonValue>)
    ) {
      throw new AtomicRecordError(
        "INVALID_RECORD",
        "prepare",
        "NativeBinding index must match domain, instance, native ID, binding ID, and generation",
      );
    }
  } else if (nativeIdentity !== undefined) {
    throw new AtomicRecordError(
      "INVALID_RECORD",
      "prepare",
      "nativeIdentity index is only valid for a NativeBinding object",
    );
  }

  const objectHash = sha256(objectBytes);
  const eventHash = sha256(eventBytes);
  const receiptHash = sha256(receiptBytes);
  const operationFingerprint = sha256(
    new TextEncoder().encode(
      canonicalJson({
        domainId,
        event: {
          counter: input.event.counter,
          eventHash,
          eventId: input.event.eventId,
          eventType: input.event.eventType,
          expectedPreviousCounter: input.event.expectedPreviousCounter,
          occurredAt: input.event.occurredAt,
          streamId: input.event.streamId,
        },
        nativeIdentity: nativeIdentity ?? null,
        object: {
          objectHash,
          objectId: row.objectId,
          objectType: objectDecoded.value.objectType,
          objectVersion: row.objectVersion,
        },
        receipt: {
          receiptHash,
          receiptId: input.receipt.receiptId,
          receiptType: input.receipt.receiptType,
          recordedAt: input.receipt.recordedAt,
        },
      }),
    ),
  );
  return Object.freeze({
    domainId,
    objectType: objectDecoded.value.objectType,
    objectId: row.objectId,
    objectVersion: row.objectVersion,
    objectBytes,
    objectHash,
    nativeIdentity,
    eventId: requireText(input.event.eventId, "event.eventId"),
    streamId: requireText(input.event.streamId, "event.streamId"),
    expectedPreviousCounter: expected,
    counter: input.event.counter,
    eventType: requireText(input.event.eventType, "event.eventType"),
    occurredAt: requireTimestamp(input.event.occurredAt, "event.occurredAt"),
    eventBytes,
    eventHash,
    receiptId: requireText(input.receipt.receiptId, "receipt.receiptId"),
    operationId: requireText(input.receipt.operationId, "receipt.operationId"),
    receiptType: requireText(input.receipt.receiptType, "receipt.receiptType"),
    recordedAt: requireTimestamp(input.receipt.recordedAt, "receipt.recordedAt"),
    receiptBytes,
    receiptHash,
    operationFingerprint,
  });
}

export function assertDurabilityContract(settings: SqliteDurability): void {
  const journalMode = settings.journalMode.toLowerCase();
  const synchronous =
    typeof settings.synchronous === "string"
      ? settings.synchronous.toLowerCase()
      : settings.synchronous;
  if (settings.foreignKeys !== 1 && settings.foreignKeys !== true) {
    throw new AtomicRecordError(
      "DURABILITY_CONTRACT_FAILED",
      "durability",
      "PRAGMA foreign_keys must be ON on the transaction connection",
    );
  }
  if (journalMode !== "wal") {
    throw new AtomicRecordError(
      "DURABILITY_CONTRACT_FAILED",
      "durability",
      `PRAGMA journal_mode must be WAL, observed ${settings.journalMode}`,
    );
  }
  if (synchronous !== 2 && synchronous !== "full") {
    throw new AtomicRecordError(
      "DURABILITY_CONTRACT_FAILED",
      "durability",
      `PRAGMA synchronous must be FULL, observed ${settings.synchronous}`,
    );
  }
}

const INSERT_STREAM_HEAD = `
  INSERT INTO gogoke_stream_heads (domain_id, stream_id, counter)
  VALUES (?, ?, ?)
  ON CONFLICT DO NOTHING
`;
const ADVANCE_STREAM_HEAD = `
  UPDATE gogoke_stream_heads
  SET counter = ?
  WHERE domain_id = ? AND stream_id = ? AND counter = ?
`;
const INSERT_OBJECT = `
  INSERT INTO gogoke_objects (
    domain_id, object_type, object_id, object_version,
    canonical_json, content_hash, created_at
  ) VALUES (?, ?, ?, ?, ?, ?, ?)
  ON CONFLICT DO NOTHING
`;
const INSERT_NATIVE_IDENTITY = `
  INSERT INTO gogoke_native_identities (
    domain_id, runtime_instance_id, native_id,
    object_type, object_id, object_version
  ) VALUES (?, ?, ?, ?, ?, ?)
  ON CONFLICT DO NOTHING
`;
const INSERT_EVENT = `
  INSERT INTO gogoke_events (
    domain_id, event_id, stream_id, stream_counter, event_type, occurred_at,
    object_type, object_id, object_version, canonical_json, content_hash
  ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  ON CONFLICT DO NOTHING
`;
const INSERT_RECEIPT = `
  INSERT INTO gogoke_receipts (
    domain_id, receipt_id, operation_id, event_id,
    object_type, object_id, object_version, receipt_type, recorded_at,
    operation_fingerprint, canonical_json, content_hash
  ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  ON CONFLICT DO NOTHING
`;

const exactlyOne = (changes: number | bigint): boolean => changes === 1 || changes === 1n;

const requireInserted = (
  result: { readonly changes: number | bigint },
  stage: AtomicRecordStage,
): void => {
  if (!exactlyOne(result.changes)) {
    throw new AtomicRecordError(
      "WRITE_CONFLICT",
      stage,
      `${stage} conflicts with an existing durable identity`,
    );
  }
};

export interface AtomicRecordReceipt {
  readonly disposition: "COMMITTED" | "RECONCILED";
  readonly domainId: string;
  readonly receiptId: string;
  readonly operationId: string;
  readonly eventId: string;
  readonly objectHash: string;
  readonly eventHash: string;
  readonly receiptHash: string;
  readonly operationFingerprint: string;
}

const matchesExistingOperation = (
  existing: ExistingAtomicReceipt,
  record: PreparedAtomicRecord,
): boolean =>
  existing.domainId === record.domainId &&
  existing.receiptId === record.receiptId &&
  existing.operationId === record.operationId &&
  existing.eventId === record.eventId &&
  existing.objectType === record.objectType &&
  existing.objectId === record.objectId &&
  existing.objectVersion === record.objectVersion &&
  existing.objectHash === record.objectHash &&
  existing.eventHash === record.eventHash &&
  existing.receiptHash === record.receiptHash &&
  existing.operationFingerprint === record.operationFingerprint;

const receiptFromRecord = (
  record: PreparedAtomicRecord,
  disposition: AtomicRecordReceipt["disposition"],
): AtomicRecordReceipt => ({
  disposition,
  domainId: record.domainId,
  receiptId: record.receiptId,
  operationId: record.operationId,
  eventId: record.eventId,
  objectHash: record.objectHash,
  eventHash: record.eventHash,
  receiptHash: record.receiptHash,
  operationFingerprint: record.operationFingerprint,
});

/**
 * One bounded write path. The driver supplies the already-owned T3 SQLite
 * connection; this module never chooses a path, opens a database, or schedules
 * work. Receipt insertion is deliberately the final statement.
 */
export async function writeAtomicRecord(
  driver: AtomicSqliteDriver,
  input: AtomicRecordInput,
): Promise<AtomicRecordReceipt> {
  const record = prepareAtomicRecord(snapshotAtomicInput(input));
  let stage: AtomicRecordStage = "durability";
  try {
    return await driver.withNativePinnedImmediateTransaction(async (transaction) => {
      assertDurabilityContract(await transaction.readDurability());

      stage = "operation-query";
      const existing = await transaction.queryReceiptByOperation(
        record.domainId,
        record.operationId,
      );
      if (existing !== null) {
        if (!matchesExistingOperation(existing, record)) {
          throw new AtomicRecordError(
            "OPERATION_CONFLICT",
            stage,
            "operation identity already exists with different durable fingerprints",
          );
        }
        return receiptFromRecord(record, "RECONCILED");
      }

      stage = "stream-counter";
      const counterResult =
        record.expectedPreviousCounter === null
          ? await transaction.execute(INSERT_STREAM_HEAD, [
              record.domainId,
              record.streamId,
              record.counter,
            ])
          : await transaction.execute(ADVANCE_STREAM_HEAD, [
              record.counter,
              record.domainId,
              record.streamId,
              record.expectedPreviousCounter,
            ]);
      if (!exactlyOne(counterResult.changes)) {
        throw new AtomicRecordError(
          "COUNTER_CONFLICT",
          stage,
          "stream head no longer matches expectedPreviousCounter",
        );
      }

      stage = "object";
      requireInserted(
        await transaction.execute(INSERT_OBJECT, [
          record.domainId,
          record.objectType,
          record.objectId,
          record.objectVersion,
          record.objectBytes,
          record.objectHash,
          record.occurredAt,
        ]),
        stage,
      );

      if (record.nativeIdentity !== undefined) {
        stage = "native-identity";
        requireInserted(
          await transaction.execute(INSERT_NATIVE_IDENTITY, [
            record.domainId,
            record.nativeIdentity.runtimeInstanceId,
            record.nativeIdentity.nativeId,
            record.objectType,
            record.objectId,
            record.objectVersion,
          ]),
          stage,
        );
      }

      stage = "event";
      requireInserted(
        await transaction.execute(INSERT_EVENT, [
          record.domainId,
          record.eventId,
          record.streamId,
          record.counter,
          record.eventType,
          record.occurredAt,
          record.objectType,
          record.objectId,
          record.objectVersion,
          record.eventBytes,
          record.eventHash,
        ]),
        stage,
      );

      stage = "receipt";
      requireInserted(
        await transaction.execute(INSERT_RECEIPT, [
          record.domainId,
          record.receiptId,
          record.operationId,
          record.eventId,
          record.objectType,
          record.objectId,
          record.objectVersion,
          record.receiptType,
          record.recordedAt,
          record.operationFingerprint,
          record.receiptBytes,
          record.receiptHash,
        ]),
        stage,
      );

      stage = "commit";
      return receiptFromRecord(record, "COMMITTED");
    });
  } catch (error) {
    if (error instanceof AtomicRecordError) throw error;
    if (error instanceof AtomicCommitUnknownError) {
      throw new AtomicRecordError(
        "COMMIT_UNKNOWN",
        "commit",
        "commit outcome is unknown; reconcile the original operation identity before any retry",
        error,
      );
    }
    throw new AtomicRecordError(
      "ATOMIC_WRITE_FAILED",
      stage,
      `${stage} failed; the transaction must be rolled back without normalization or retry`,
      error,
    );
  }
}
