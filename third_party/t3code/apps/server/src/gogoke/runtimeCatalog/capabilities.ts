import type { JsonObject, JsonValue, ModelRef, RuntimeInstanceId } from "../contracts/model.ts";
import { cloneAndFreezeJson, RuntimeCatalogError } from "./manifest.ts";
import {
  CAPABILITY_SUPPORT,
  KNOWN_CAPABILITY_NAMES,
  type CapabilityClaim,
  type CapabilityLayers,
  type CapabilityResolution,
  type ObservedCapability,
  type QualificationKeyInput,
  type QualifiedCapability,
  type QualifiedCapabilityInput,
} from "./types.ts";

const OPEN_ID_PATTERN = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;
const SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;
const U64_PATTERN = /^(0|[1-9]\d*)$/;

const KEY_FIELDS: ReadonlyArray<keyof QualificationKeyInput> = [
  "driverId",
  "adapterDigest",
  "nativeDigest",
  "platform",
  "profileRef",
  "authRevision",
  "runtimeInstanceId",
  "nativeModelId",
  "resolvedModelVersion",
  "runtimeMode",
  "isolationProfile",
  "toolProfile",
  "generation",
];

const authenticLayers = new WeakSet<object>();

const knownCapability = (name: string): boolean =>
  KNOWN_CAPABILITY_NAMES.includes(name as (typeof KNOWN_CAPABILITY_NAMES)[number]);

const requiredString = (value: unknown, path: string): string => {
  if (typeof value !== "string" || value.length === 0) {
    throw new RuntimeCatalogError("INVALID_CAPABILITY", `${path} must be a non-empty string`);
  }
  return value;
};

const snapshotExactRecord = (
  value: unknown,
  allowedFields: ReadonlySet<string>,
  path: string,
): Readonly<Record<string, JsonValue>> => {
  const snapshot = cloneAndFreezeJson(value as JsonValue, path);
  if (snapshot === null || typeof snapshot !== "object" || Array.isArray(snapshot)) {
    throw new RuntimeCatalogError("INVALID_CAPABILITY", `${path} must be a plain data object`);
  }
  const unknownField = Object.keys(snapshot).find((field) => !allowedFields.has(field));
  if (unknownField !== undefined) {
    throw new RuntimeCatalogError(
      "INVALID_CAPABILITY",
      `${path} contains unknown field ${unknownField}`,
    );
  }
  return snapshot as JsonObject;
};

const normalizeClaim = (
  claim: CapabilityClaim,
  path: string,
  extraFields: ReadonlyArray<string> = [],
): CapabilityClaim => {
  const raw = snapshotExactRecord(
    claim,
    new Set(["name", "support", "constraints", ...extraFields]),
    path,
  );
  const name = requiredString(raw.name, `${path}.name`);
  if (!CAPABILITY_SUPPORT.includes(raw.support as (typeof CAPABILITY_SUPPORT)[number])) {
    throw new RuntimeCatalogError("INVALID_CAPABILITY", `${path}.support is invalid`);
  }
  const support = knownCapability(name)
    ? (raw.support as (typeof CAPABILITY_SUPPORT)[number])
    : "unknown";
  return Object.freeze({
    name,
    support,
    ...(raw.constraints === undefined
      ? {}
      : {
          constraints: raw.constraints,
        }),
  });
};

const normalizeEvidenceRefs = (
  value: ReadonlyArray<string>,
  path: string,
): ReadonlyArray<string> => {
  if (!Array.isArray(value)) {
    throw new RuntimeCatalogError("INVALID_CAPABILITY", `${path} must be an array`);
  }
  if (value.length === 0) {
    throw new RuntimeCatalogError("INVALID_CAPABILITY", `${path} must contain evidence`);
  }
  const normalized = value.map((entry, index) => requiredString(entry, `${path}[${index}]`));
  if (new Set(normalized).size !== normalized.length) {
    throw new RuntimeCatalogError("INVALID_CAPABILITY", `${path} must not contain duplicates`);
  }
  return Object.freeze(normalized);
};

const normalizeObservation = (
  observation: ObservedCapability,
  path: string,
  extraFields: ReadonlyArray<string> = [],
): ObservedCapability => {
  const raw = snapshotExactRecord(
    observation,
    new Set(["name", "support", "constraints", "evidenceRefs", ...extraFields]),
    path,
  );
  const claim = normalizeClaim(raw as unknown as CapabilityClaim, path, [
    "evidenceRefs",
    ...extraFields,
  ]);
  return Object.freeze({
    ...claim,
    evidenceRefs: normalizeEvidenceRefs(
      raw.evidenceRefs as unknown as ReadonlyArray<string>,
      `${path}.evidenceRefs`,
    ),
  });
};

const normalizeQualificationInput = (
  input: QualificationKeyInput,
  path: string,
): Readonly<QualificationKeyInput> => {
  const raw = snapshotExactRecord(input, new Set(KEY_FIELDS), path);
  const values = Object.fromEntries(
    KEY_FIELDS.map((field) => [field, requiredString(raw[field], `${path}.${field}`)]),
  ) as unknown as QualificationKeyInput;
  if (!OPEN_ID_PATTERN.test(values.driverId) || !OPEN_ID_PATTERN.test(values.runtimeInstanceId)) {
    throw new RuntimeCatalogError(
      "INVALID_CAPABILITY",
      `${path} driverId and runtimeInstanceId must be open runtime ids`,
    );
  }
  if (!SHA256_PATTERN.test(values.adapterDigest) || !SHA256_PATTERN.test(values.nativeDigest)) {
    throw new RuntimeCatalogError(
      "INVALID_CAPABILITY",
      `${path} adapterDigest and nativeDigest must be lowercase sha256 digests`,
    );
  }
  if (!U64_PATTERN.test(values.authRevision) || !U64_PATTERN.test(values.generation)) {
    throw new RuntimeCatalogError(
      "INVALID_CAPABILITY",
      `${path} authRevision and generation must be canonical unsigned integers`,
    );
  }
  return Object.freeze({ ...values });
};

const canonicalQualificationKey = (input: Readonly<QualificationKeyInput>): string =>
  KEY_FIELDS.map(
    (field) => `${field.length}:${field}=${String(input[field]).length}:${input[field]}`,
  ).join("|");

export const makeQualificationKey = (input: QualificationKeyInput): string =>
  canonicalQualificationKey(normalizeQualificationInput(input, "qualification"));

const freezeLayers = (layers: CapabilityLayers): CapabilityLayers => {
  const frozen = Object.freeze({
    declared: Object.freeze([...layers.declared]),
    observed: Object.freeze([...layers.observed]),
    qualified: Object.freeze([...layers.qualified]),
  });
  authenticLayers.add(frozen);
  return frozen;
};

const assertAuthenticLayers = (layers: CapabilityLayers): void => {
  if (!authenticLayers.has(layers)) {
    throw new RuntimeCatalogError(
      "UNTRUSTED_CAPABILITY_LAYERS",
      "Capability layers must originate from createCapabilityLayers and registered transitions",
    );
  }
};

export const createCapabilityLayers = (
  declared: ReadonlyArray<CapabilityClaim>,
): CapabilityLayers => {
  const snapshot = cloneAndFreezeJson(declared as unknown as JsonValue, "declared");
  if (!Array.isArray(snapshot)) {
    throw new RuntimeCatalogError("INVALID_CAPABILITY", "declared must be an array");
  }
  const normalized = snapshot.map((claim, index) =>
    normalizeClaim(claim as unknown as CapabilityClaim, `declared[${index}]`),
  );
  const names = new Set<string>();
  for (const claim of normalized) {
    if (names.has(claim.name)) {
      throw new RuntimeCatalogError(
        "INVALID_CAPABILITY",
        `declared contains duplicate capability ${claim.name}`,
      );
    }
    names.add(claim.name);
  }
  return freezeLayers({
    declared: normalized,
    observed: [],
    qualified: [],
  });
};

export const withObservedCapability = (
  layers: CapabilityLayers,
  observation: ObservedCapability,
): CapabilityLayers => {
  assertAuthenticLayers(layers);
  const normalized = normalizeObservation(observation, "observation");
  return freezeLayers({
    ...layers,
    observed: [...layers.observed.filter((entry) => entry.name !== normalized.name), normalized],
  });
};

const normalizeQualification = (qualification: QualifiedCapabilityInput): QualifiedCapability => {
  const raw = snapshotExactRecord(
    qualification,
    new Set(["name", "support", "constraints", "evidenceRefs", "qualification"]),
    "qualification",
  );
  const normalized = normalizeObservation(
    raw as unknown as QualifiedCapabilityInput,
    "qualification",
    ["qualification"],
  );
  if (!knownCapability(normalized.name)) {
    throw new RuntimeCatalogError(
      "INVALID_CAPABILITY",
      `Unknown capability ${normalized.name} cannot be qualified`,
    );
  }
  const qualificationInput = normalizeQualificationInput(
    raw.qualification as unknown as QualificationKeyInput,
    "qualification.qualification",
  );
  return Object.freeze({
    ...normalized,
    qualificationKey: canonicalQualificationKey(qualificationInput),
    qualification: qualificationInput,
  });
};

export const withQualifiedCapability = (
  layers: CapabilityLayers,
  qualification: QualifiedCapabilityInput,
): CapabilityLayers => {
  assertAuthenticLayers(layers);
  const normalized = normalizeQualification(qualification);
  return freezeLayers({
    ...layers,
    qualified: [
      ...layers.qualified.filter(
        (entry) =>
          entry.name !== normalized.name || entry.qualificationKey !== normalized.qualificationKey,
      ),
      normalized,
    ],
  });
};

const verifiedQualification = (entry: QualifiedCapability): QualifiedCapability => {
  const normalized = normalizeQualification({
    name: entry.name,
    support: entry.support,
    ...(entry.constraints === undefined ? {} : { constraints: entry.constraints }),
    evidenceRefs: entry.evidenceRefs,
    qualification: entry.qualification,
  });
  if (normalized.qualificationKey !== entry.qualificationKey) {
    throw new RuntimeCatalogError(
      "INVALID_CAPABILITY",
      `Qualification key for ${entry.name} does not match its full qualification input`,
    );
  }
  return normalized;
};

const resolution = (value: CapabilityResolution): CapabilityResolution =>
  Object.freeze({ ...value, evidenceRefs: Object.freeze([...value.evidenceRefs]) });

export const resolveQualifiedCapability = (
  layers: CapabilityLayers,
  capabilityName: string,
  currentQualification: QualificationKeyInput,
): CapabilityResolution => {
  assertAuthenticLayers(layers);
  if (!knownCapability(capabilityName)) {
    return resolution({
      support: "unknown",
      layer: "none",
      reason: "NOT_QUALIFIED",
      evidenceRefs: [],
    });
  }
  const currentQualificationKey = makeQualificationKey(currentQualification);
  const qualifications = layers.qualified
    .filter((entry) => entry.name === capabilityName)
    .map(verifiedQualification);
  const matchingQualification = qualifications.find(
    (entry) => entry.qualificationKey === currentQualificationKey,
  );
  if (matchingQualification !== undefined) {
    return resolution({
      support: matchingQualification.support,
      layer: "qualified",
      reason: "QUALIFIED",
      evidenceRefs: matchingQualification.evidenceRefs,
    });
  }
  const staleQualification = qualifications[0];
  if (staleQualification === undefined) {
    return resolution({
      support: "unknown",
      layer: "none",
      reason: "NOT_QUALIFIED",
      evidenceRefs: [],
    });
  }
  return resolution({
    support: "unknown",
    layer: "none",
    reason: "STALE_QUALIFICATION",
    evidenceRefs: staleQualification.evidenceRefs,
  });
};

/** The native model id is only unique inside its runtime instance. */
export const modelIdentityKey = (
  model: Pick<ModelRef, "runtimeInstanceId" | "nativeModelId" | "resolvedVersion">,
): string =>
  `${model.runtimeInstanceId.length}:${model.runtimeInstanceId}|${model.nativeModelId.length}:${model.nativeModelId}|${model.resolvedVersion.length}:${model.resolvedVersion}`;

export const instanceScopeKey = (instanceId: RuntimeInstanceId): string =>
  `${instanceId.length}:${instanceId}`;
