import {
  OBJECT_DEFINITIONS,
  type DecodedPublicObject,
  type FieldRule,
  type JsonObject,
  type JsonValue,
  type PublicObjectMap,
  type PublicObjectType,
  type U64String,
} from "./model.ts";
import { canonicalJson, ContractCodecError, parseStrictJsonBytes } from "./strictJson.ts";

const U64_MAX = 18_446_744_073_709_551_615n;
const U64_PATTERN = /^(?:0|[1-9]\d*)$/;
const OPEN_ID_PATTERN = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;
const SCHEMA_PATTERN = /^gogoke\.s1-r4\.objects\.v(\d+)(?:\.(\d+))?$/;
const ENVELOPE_FIELDS = new Set(["schema", "objectType", "object"]);
const validatedDecodedRecords = new WeakSet<object>();

const isJsonObject = (value: JsonValue): value is JsonObject =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const invalidField = (path: string, expected: string): never => {
  throw new ContractCodecError("INVALID_FIELD", `${path} must be ${expected}`);
};

export function decodeU64(value: JsonValue, path: string): U64String {
  if (typeof value !== "string") {
    throw new ContractCodecError("U64_NOT_STRING", `${path} must be a lossless decimal string`);
  }
  if (!U64_PATTERN.test(value)) invalidField(path, "a canonical unsigned decimal string");
  if (BigInt(value) > U64_MAX) {
    throw new ContractCodecError("U64_OVERFLOW", `${path} exceeds uint64`);
  }
  return value as U64String;
}

function validateString(value: JsonValue, path: string): string {
  if (typeof value !== "string") {
    throw new ContractCodecError("INVALID_FIELD", `${path} must be a non-empty string`);
  }
  if (value.length === 0) {
    throw new ContractCodecError("INVALID_FIELD", `${path} must be a non-empty string`);
  }
  return value;
}

function validateRule(value: JsonValue, rule: FieldRule, path: string): JsonValue {
  if (typeof rule !== "string") {
    const allowed = rule[1];
    const candidate = validateString(value, path);
    if (!allowed.includes(candidate)) invalidField(path, `one of ${allowed.join(", ")}`);
    return candidate;
  }
  switch (rule) {
    case "boolean":
      if (typeof value !== "boolean") invalidField(path, "a boolean");
      return value;
    case "driverId":
    case "instanceId": {
      const candidate = validateString(value, path);
      if (!OPEN_ID_PATTERN.test(candidate)) {
        invalidField(
          path,
          "an open identifier starting with a letter and using letters/digits/_/-",
        );
      }
      return candidate;
    }
    case "json":
      return value;
    case "jsonArray":
      if (!Array.isArray(value)) invalidField(path, "an array");
      return value;
    case "jsonObject":
      if (!isJsonObject(value)) invalidField(path, "an object");
      return value;
    case "string":
      return validateString(value, path);
    case "stringArray":
      if (
        !Array.isArray(value) ||
        value.some((item) => typeof item !== "string" || item.length === 0)
      ) {
        invalidField(path, "an array of non-empty strings");
      }
      return value;
    case "u64":
      return decodeU64(value, path);
  }
}

function decodeKnownObject<K extends PublicObjectType>(
  objectType: K,
  rawObject: JsonObject,
): { readonly object: PublicObjectMap[K]; readonly unknown: JsonObject } {
  const definition = OBJECT_DEFINITIONS[objectType];
  const known: Record<string, JsonValue> = Object.create(null) as Record<string, JsonValue>;
  const unknown: Record<string, JsonValue> = Object.create(null) as Record<string, JsonValue>;
  for (const [field, rule] of Object.entries(definition)) {
    if (!Object.hasOwn(rawObject, field)) invalidField(`object.${field}`, "present");
    known[field] = validateRule(rawObject[field]!, rule, `object.${field}`);
  }
  for (const [field, value] of Object.entries(rawObject)) {
    if (!Object.hasOwn(definition, field)) unknown[field] = value;
  }
  return { object: known as unknown as PublicObjectMap[K], unknown };
}

export function decodePublicObject(bytes: Uint8Array): DecodedPublicObject {
  const raw = parseStrictJsonBytes(bytes);
  if (!isJsonObject(raw)) {
    throw new ContractCodecError("INVALID_ENVELOPE", "Contract envelope must be an object");
  }
  if (typeof raw.schema !== "string") {
    throw new ContractCodecError("INVALID_ENVELOPE", "schema must be a string");
  }
  const schemaMatch = SCHEMA_PATTERN.exec(raw.schema);
  if (schemaMatch === null) {
    throw new ContractCodecError("UNKNOWN_SCHEMA", `Unsupported schema ${raw.schema}`);
  }
  if (schemaMatch[1] !== "1") {
    throw new ContractCodecError("UNKNOWN_MAJOR", `Unsupported contract major ${schemaMatch[1]}`);
  }
  if (typeof raw.objectType !== "string" || !Object.hasOwn(OBJECT_DEFINITIONS, raw.objectType)) {
    throw new ContractCodecError(
      "UNKNOWN_OBJECT_TYPE",
      `Unsupported object type ${String(raw.objectType)}`,
    );
  }
  const rawObject = raw.object;
  if (rawObject === undefined || !isJsonObject(rawObject)) {
    throw new ContractCodecError("INVALID_ENVELOPE", "object must be an object");
  }

  const objectType = raw.objectType as PublicObjectType;
  const decoded = decodeKnownObject(objectType, rawObject);
  const unknownEnvelope: Record<string, JsonValue> = Object.create(null) as Record<
    string,
    JsonValue
  >;
  for (const [field, value] of Object.entries(raw)) {
    if (!ENVELOPE_FIELDS.has(field)) unknownEnvelope[field] = value;
  }
  const result = cloneAndFreezeJson({
    value: { schema: raw.schema, objectType, object: decoded.object },
    unknownFields: { envelope: unknownEnvelope, object: decoded.unknown },
  } as unknown as JsonValue) as unknown as DecodedPublicObject;
  validatedDecodedRecords.add(result);
  return result;
}

export function encodePublicObject(decoded: DecodedPublicObject): Uint8Array {
  const object = {
    ...decoded.unknownFields.object,
    ...(decoded.value.object as unknown as JsonObject),
  };
  const envelope: JsonObject = {
    ...decoded.unknownFields.envelope,
    schema: decoded.value.schema,
    objectType: decoded.value.objectType,
    object,
  };
  // Validate caller-constructed values through the same byte boundary before returning them.
  const bytes = new TextEncoder().encode(canonicalJson(envelope));
  decodePublicObject(bytes);
  return bytes;
}

function cloneAndFreezeJson(value: JsonValue): JsonValue {
  if (Array.isArray(value)) {
    return Object.freeze(value.map((item) => cloneAndFreezeJson(item)));
  }
  if (isJsonObject(value)) {
    const clone: Record<string, JsonValue> = {};
    for (const [key, item] of Object.entries(value)) {
      Object.defineProperty(clone, key, {
        value: cloneAndFreezeJson(item),
        enumerable: true,
        configurable: false,
        writable: false,
      });
    }
    return Object.freeze(clone);
  }
  return value;
}

/**
 * Returns a defensive, deeply frozen copy of only v1-known fields.
 * Extension fields never participate in authority decisions, and callers
 * cannot mutate the decoded record after validation to forge authority.
 */
export function authorityProjection<K extends PublicObjectType>(
  decoded: DecodedPublicObject<K>,
): PublicObjectMap[K] {
  if (!validatedDecodedRecords.has(decoded as object)) {
    throw new ContractCodecError(
      "INVALID_ENVELOPE",
      "Authority projection requires a decoder-originated record",
    );
  }
  return cloneAndFreezeJson(
    decoded.value.object as unknown as JsonValue,
  ) as unknown as PublicObjectMap[K];
}
