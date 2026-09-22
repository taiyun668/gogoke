import * as NodeUtilTypes from "node:util/types";

import type { JsonObject, JsonValue, RuntimeDriverId } from "../contracts/model.ts";
import {
  ADAPTER_MANIFEST_SCHEMA,
  CAPABILITY_SUPPORT,
  KNOWN_CAPABILITY_NAMES,
  type AdapterManifestV1,
  type RuntimeCatalogHostContext,
} from "./types.ts";

const OPEN_ID_PATTERN = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;
const SEMVER_PATTERN = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;

export class RuntimeCatalogError extends Error {
  readonly code:
    | "DUPLICATE_REGISTRATION"
    | "ADMISSION_NOT_GRANTED"
    | "EFFECT_NOT_GRANTED"
    | "HOST_API_INCOMPATIBLE"
    | "INVALID_CAPABILITY"
    | "INVALID_INSTANCE_CONFIG"
    | "INVALID_MANIFEST"
    | "MISSING_HOST_SERVICE"
    | "PLATFORM_UNSUPPORTED"
    | "SWITCH_NOT_AVAILABLE"
    | "UNTRUSTED_CAPABILITY_LAYERS"
    | "UNTRUSTED_INSTANCE_CONFIG"
    | "UNTRUSTED_LEASE_STATE"
    | "UNTRUSTED_RESOLVED_INSTANCE";

  constructor(
    code:
      | "DUPLICATE_REGISTRATION"
      | "ADMISSION_NOT_GRANTED"
      | "EFFECT_NOT_GRANTED"
      | "HOST_API_INCOMPATIBLE"
      | "INVALID_CAPABILITY"
      | "INVALID_INSTANCE_CONFIG"
      | "INVALID_MANIFEST"
      | "MISSING_HOST_SERVICE"
      | "PLATFORM_UNSUPPORTED"
      | "SWITCH_NOT_AVAILABLE"
      | "UNTRUSTED_CAPABILITY_LAYERS"
      | "UNTRUSTED_INSTANCE_CONFIG"
      | "UNTRUSTED_LEASE_STATE"
      | "UNTRUSTED_RESOLVED_INSTANCE",
    message: string,
  ) {
    super(message);
    this.code = code;
    this.name = "RuntimeCatalogError";
  }
}

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const expectObject = (value: unknown, path: string): Record<string, unknown> => {
  if (!isObject(value))
    throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must be an object`);
  return value;
};

const expectString = (value: unknown, path: string): string => {
  if (typeof value !== "string" || value.length === 0) {
    throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must be a non-empty string`);
  }
  return value;
};

const expectStrings = (value: unknown, path: string): ReadonlyArray<string> => {
  if (
    !Array.isArray(value) ||
    value.some((entry) => typeof entry !== "string" || entry.length === 0)
  ) {
    throw new RuntimeCatalogError(
      "INVALID_MANIFEST",
      `${path} must be an array of non-empty strings`,
    );
  }
  return value;
};

const expectOpenId = (value: unknown, path: string): RuntimeDriverId => {
  const id = expectString(value, path);
  if (!OPEN_ID_PATTERN.test(id)) {
    throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must be an open runtime id`);
  }
  return id as RuntimeDriverId;
};

const expectSemver = (value: unknown, path: string): string => {
  const version = expectString(value, path);
  if (!SEMVER_PATTERN.test(version)) {
    throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must be semantic version x.y.z`);
  }
  return version;
};

const semverTuple = (version: string): readonly [string, string, string] => {
  const match = SEMVER_PATTERN.exec(version);
  if (match === null) {
    throw new RuntimeCatalogError("INVALID_MANIFEST", `Invalid semantic version ${version}`);
  }
  return [match[1]!, match[2]!, match[3]!];
};

const compareDecimalIdentifiers = (left: string, right: string): number => {
  if (left.length !== right.length) return left.length < right.length ? -1 : 1;
  if (left === right) return 0;
  return left < right ? -1 : 1;
};

const compareSemver = (left: string, right: string): number => {
  const a = semverTuple(left);
  const b = semverTuple(right);
  for (let index = 0; index < a.length; index += 1) {
    const componentOrder = compareDecimalIdentifiers(a[index]!, b[index]!);
    if (componentOrder !== 0) return componentOrder;
  }
  return 0;
};

const cloneJson = (value: unknown, path: string, ancestors: WeakSet<object>): JsonValue => {
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must contain finite JSON numbers`);
    }
    return value;
  }
  if (typeof value !== "object") {
    throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must be JSON data`);
  }
  if (NodeUtilTypes.isProxy(value)) {
    throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must not be a Proxy`);
  }
  if (ancestors.has(value)) {
    throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must not contain a cycle`);
  }
  ancestors.add(value);
  try {
    if (Array.isArray(value)) {
      if (Object.getPrototypeOf(value) !== Array.prototype) {
        throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must be a plain array`);
      }
      const descriptors = Object.getOwnPropertyDescriptors(value);
      const ownKeys = Reflect.ownKeys(descriptors);
      if (ownKeys.some((key) => typeof key === "symbol")) {
        throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must not contain symbol keys`);
      }
      const lengthDescriptor = Reflect.get(descriptors, "length") as PropertyDescriptor | undefined;
      if (lengthDescriptor === undefined || !("value" in lengthDescriptor)) {
        throw new RuntimeCatalogError("INVALID_MANIFEST", `${path}.length must be a data property`);
      }
      const length = lengthDescriptor.value as number;
      const cloned: JsonValue[] = [];
      const allowedKeys = new Set<string>(["length"]);
      for (let index = 0; index < length; index += 1) {
        allowedKeys.add(String(index));
        const descriptor = descriptors[String(index)];
        if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
          throw new RuntimeCatalogError(
            "INVALID_MANIFEST",
            `${path}[${index}] must be an enumerable data property`,
          );
        }
        cloned.push(cloneJson(descriptor.value, `${path}[${index}]`, ancestors));
      }
      const unexpected = ownKeys.find((key) => typeof key === "string" && !allowedKeys.has(key));
      if (unexpected !== undefined) {
        throw new RuntimeCatalogError(
          "INVALID_MANIFEST",
          `${path} contains unexpected property ${String(unexpected)}`,
        );
      }
      return cloned;
    }
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null) {
      throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must be a plain JSON object`);
    }
    const descriptors = Object.getOwnPropertyDescriptors(value);
    const ownKeys = Reflect.ownKeys(descriptors);
    if (ownKeys.some((key) => typeof key === "symbol")) {
      throw new RuntimeCatalogError("INVALID_MANIFEST", `${path} must not contain symbol keys`);
    }
    const cloned: Record<string, JsonValue> = {};
    for (const key of ownKeys as string[]) {
      const descriptor = descriptors[key]!;
      if (!("value" in descriptor) || !descriptor.enumerable) {
        throw new RuntimeCatalogError(
          "INVALID_MANIFEST",
          `${path}.${key} must be an enumerable data property`,
        );
      }
      Object.defineProperty(cloned, key, {
        value: cloneJson(descriptor.value, `${path}.${key}`, ancestors),
        enumerable: true,
        configurable: true,
        writable: true,
      });
    }
    return cloned;
  } finally {
    ancestors.delete(value);
  }
};

const freezeJson = <T>(value: T): T => {
  if (value !== null && typeof value === "object") {
    for (const child of Object.values(value)) freezeJson(child);
    Object.freeze(value);
  }
  return value;
};

export const cloneAndFreezeJson = <T extends JsonValue>(value: T, path = "value"): T =>
  freezeJson(cloneJson(value, path, new WeakSet())) as T;

export function parseAdapterManifest(input: unknown): AdapterManifestV1 {
  const raw = expectObject(cloneAndFreezeJson(input as JsonValue, "manifest"), "manifest");
  if (raw.schema !== ADAPTER_MANIFEST_SCHEMA) {
    throw new RuntimeCatalogError(
      "INVALID_MANIFEST",
      `Unsupported adapter manifest schema ${String(raw.schema)}`,
    );
  }

  const hostApiRange = expectObject(raw.hostApiRange, "manifest.hostApiRange");
  const minInclusive = expectSemver(
    hostApiRange.minInclusive,
    "manifest.hostApiRange.minInclusive",
  );
  const maxExclusive = expectSemver(
    hostApiRange.maxExclusive,
    "manifest.hostApiRange.maxExclusive",
  );
  if (compareSemver(minInclusive, maxExclusive) >= 0) {
    throw new RuntimeCatalogError(
      "INVALID_MANIFEST",
      "manifest.hostApiRange must have minInclusive < maxExclusive",
    );
  }

  if (!Array.isArray(raw.declaredCapabilities)) {
    throw new RuntimeCatalogError(
      "INVALID_MANIFEST",
      "manifest.declaredCapabilities must be an array",
    );
  }
  const capabilityNames = new Set<string>();
  const declaredCapabilities = raw.declaredCapabilities.map((entry, index) => {
    const capability = expectObject(entry, `manifest.declaredCapabilities[${index}]`);
    const support = expectString(
      capability.support,
      `manifest.declaredCapabilities[${index}].support`,
    );
    if (!CAPABILITY_SUPPORT.includes(support as (typeof CAPABILITY_SUPPORT)[number])) {
      throw new RuntimeCatalogError(
        "INVALID_MANIFEST",
        `manifest.declaredCapabilities[${index}].support is invalid`,
      );
    }
    const name = expectString(capability.name, `manifest.declaredCapabilities[${index}].name`);
    if (capabilityNames.has(name)) {
      throw new RuntimeCatalogError("INVALID_MANIFEST", `Duplicate capability ${name}`);
    }
    capabilityNames.add(name);
    const constraints = capability.constraints;
    const known = KNOWN_CAPABILITY_NAMES.includes(name as (typeof KNOWN_CAPABILITY_NAMES)[number]);
    return {
      name,
      support: known ? (support as (typeof CAPABILITY_SUPPORT)[number]) : "unknown",
      ...(constraints === undefined
        ? {}
        : {
            constraints: cloneAndFreezeJson(
              constraints as JsonValue,
              `manifest.declaredCapabilities[${index}].constraints`,
            ),
          }),
    };
  });

  const source = expectObject(raw.source, "manifest.source");
  if (source.kind !== "bundled") {
    throw new RuntimeCatalogError(
      "INVALID_MANIFEST",
      "Only build-bundled adapter declarations are supported in manifest v1",
    );
  }
  const license = expectObject(raw.license, "manifest.license");
  const artifactDigest = expectString(raw.artifactDigest, "manifest.artifactDigest");
  if (!SHA256_PATTERN.test(artifactDigest)) {
    throw new RuntimeCatalogError(
      "INVALID_MANIFEST",
      "manifest.artifactDigest must be a lowercase sha256 digest",
    );
  }

  return freezeJson({
    schema: ADAPTER_MANIFEST_SCHEMA,
    packageId: expectString(raw.packageId, "manifest.packageId"),
    driverId: expectOpenId(raw.driverId, "manifest.driverId"),
    adapterVersion: expectSemver(raw.adapterVersion, "manifest.adapterVersion"),
    artifactDigest,
    hostApiRange: { minInclusive, maxExclusive },
    nativeProtocol: expectString(raw.nativeProtocol, "manifest.nativeProtocol"),
    platforms: [...expectStrings(raw.platforms, "manifest.platforms")],
    configSchemaRef: expectString(raw.configSchemaRef, "manifest.configSchemaRef"),
    declaredCapabilities,
    requiredHostServices: [
      ...expectStrings(raw.requiredHostServices, "manifest.requiredHostServices"),
    ],
    requestedEffects: [...expectStrings(raw.requestedEffects, "manifest.requestedEffects")],
    source: {
      kind: "bundled",
      provenanceRef: expectString(source.provenanceRef, "manifest.source.provenanceRef"),
    },
    license: {
      spdxId: expectString(license.spdxId, "manifest.license.spdxId"),
      noticeRef: expectString(license.noticeRef, "manifest.license.noticeRef"),
    },
    admissionRef: expectString(raw.admissionRef, "manifest.admissionRef"),
  });
}

export function assertHostCompatibility(
  manifest: AdapterManifestV1,
  context: RuntimeCatalogHostContext,
): void {
  const {
    hostApiVersion,
    hostPlatform,
    availableHostServices,
    allowedEffects,
    admittedAdmissionRefs,
  } = context;
  expectSemver(hostApiVersion, "hostApiVersion");
  if (
    compareSemver(hostApiVersion, manifest.hostApiRange.minInclusive) < 0 ||
    compareSemver(hostApiVersion, manifest.hostApiRange.maxExclusive) >= 0
  ) {
    throw new RuntimeCatalogError(
      "HOST_API_INCOMPATIBLE",
      `${manifest.driverId}@${manifest.adapterVersion} does not support host API ${hostApiVersion}`,
    );
  }
  const missing = manifest.requiredHostServices.find(
    (service) => !availableHostServices.has(service),
  );
  if (missing !== undefined) {
    throw new RuntimeCatalogError(
      "MISSING_HOST_SERVICE",
      `${manifest.driverId}@${manifest.adapterVersion} requires unknown host service ${missing}`,
    );
  }
  if (!manifest.platforms.includes(hostPlatform)) {
    throw new RuntimeCatalogError(
      "PLATFORM_UNSUPPORTED",
      `${manifest.driverId}@${manifest.adapterVersion} does not support ${hostPlatform}`,
    );
  }
  const deniedEffect = manifest.requestedEffects.find((effect) => !allowedEffects.has(effect));
  if (deniedEffect !== undefined) {
    throw new RuntimeCatalogError(
      "EFFECT_NOT_GRANTED",
      `${manifest.driverId}@${manifest.adapterVersion} is not allowed effect ${deniedEffect}`,
    );
  }
  if (!admittedAdmissionRefs.has(manifest.admissionRef)) {
    throw new RuntimeCatalogError(
      "ADMISSION_NOT_GRANTED",
      `${manifest.driverId}@${manifest.adapterVersion} has no matching admission`,
    );
  }
}

export const adapterIdentity = (manifest: AdapterManifestV1) =>
  Object.freeze({
    driverId: manifest.driverId,
    adapterVersion: manifest.adapterVersion,
    artifactDigest: manifest.artifactDigest,
  });

export const manifestRegistrationKey = (
  driverId: RuntimeDriverId,
  adapterVersion: string,
): string => `${driverId}\0${adapterVersion}`;

export const jsonObject = (entries: Readonly<Record<string, JsonValue>>): JsonObject => entries;
