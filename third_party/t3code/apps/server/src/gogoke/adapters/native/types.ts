import * as crypto from "node:crypto";

/**
 * Provider-neutral native adapter contracts.
 *
 * This directory intentionally contains ports and facts only.  It does not
 * import a provider SDK, spawn a process, open a socket, or own product
 * persistence.  A Controller supplies a managed process/SDK port and a
 * provider-specific parser at the edge.
 */

export const NATIVE_FRAME_MAX_BYTES = 4 * 1024 * 1024;
export const NATIVE_ADAPTER_SOURCE_SCHEMA = "gogoke.native-adapter-source.v1" as const;
export const NATIVE_ADAPTER_CONTRACT_VERSION = "1" as const;

const SHA256_DIGEST = /^sha256:[0-9a-f]{64}$/;
const SOURCE_REVISION = /^[0-9a-f]{40}$/;
const CANONICAL_U64 = /^(?:0|[1-9][0-9]*)$/;
const SAFE_NAME = /^[A-Za-z0-9._:@/-]+$/;
const RELATIVE_PATH = /^(?![\\/])(?!(?:[A-Za-z]:))[A-Za-z0-9._@+:-]+(?:[\\/][A-Za-z0-9._@+:-]+)*$/;
const SECRET_NAME =
  /(?:token|secret|password|passwd|credential|api[_-]?key|cookie|authorization|auth)/i;
const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
const DEFAULT_ENVIRONMENT_NAMES = new Set(["CI", "LANG", "LC_ALL", "PATH", "TERM", "TZ"]);
const U64_MAX = 18_446_744_073_709_551_615n;

export type NativeBytes = Uint8Array;

export type ByteInput = Readonly<Uint8Array> | ReadonlyArray<number>;

export type NativeAdapterErrorCode =
  | "INVALID_SOURCE_CONTRACT"
  | "INVALID_LAUNCH"
  | "INVALID_FRAME"
  | "FRAME_TOO_LARGE"
  | "CANCELLED"
  | "DEADLINE_EXCEEDED"
  | "PROCESS_IDENTITY_UNKNOWN"
  | "PORT_NOT_READY"
  | "PORT_CLOSED"
  | "PORT_PROTOCOL_ERROR"
  | "PARSER_ERROR"
  | "INVALID_DESCRIPTOR"
  | "UNKNOWN_DRIVER"
  | "DRIVER_VERSION_MISMATCH"
  | "SESSION_STATE_ERROR";

export class NativeAdapterError extends Error {
  override readonly name = "NativeAdapterError";
  readonly code: NativeAdapterErrorCode;
  override readonly cause: unknown;

  constructor(code: NativeAdapterErrorCode, detail: string, cause?: unknown) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.cause = cause;
  }
}

function fail(code: NativeAdapterErrorCode, path: string, detail: string): never {
  throw new NativeAdapterError(code, `${path} ${detail}`);
}

function nonEmpty(value: unknown, path: string): string {
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    return fail("INVALID_SOURCE_CONTRACT", path, "must be a canonical non-empty string");
  }
  return value;
}

function safeName(value: unknown, path: string): string {
  const name = nonEmpty(value, path);
  if (!SAFE_NAME.test(name))
    return fail("INVALID_SOURCE_CONTRACT", path, "contains unsafe characters");
  return name;
}

function relativePath(value: unknown, path: string): string {
  const candidate = nonEmpty(value, path);
  if (!RELATIVE_PATH.test(candidate) || candidate.split(/[\\/]/u).includes("..")) {
    return fail(
      "INVALID_SOURCE_CONTRACT",
      path,
      "must be a relative path without parent traversal",
    );
  }
  return candidate;
}

function digest(value: unknown, path: string): string {
  const candidate = nonEmpty(value, path);
  if (!SHA256_DIGEST.test(candidate))
    return fail("INVALID_SOURCE_CONTRACT", path, "must be sha256:<64 lowercase hex>");
  return candidate;
}

function spdxId(value: unknown, path: string): string {
  const candidate = nonEmpty(value, path);
  if (!SAFE_NAME.test(candidate))
    return fail("INVALID_SOURCE_CONTRACT", path, "must be a canonical SPDX identifier");
  return candidate;
}

function revision(value: unknown, path: string): string {
  const candidate = nonEmpty(value, path);
  if (!SOURCE_REVISION.test(candidate))
    return fail(
      "INVALID_SOURCE_CONTRACT",
      path,
      "must be a 40-character lowercase commit revision",
    );
  return candidate;
}

function freezeArray<T>(values: ReadonlyArray<T>): ReadonlyArray<T> {
  return Object.freeze([...values]);
}

function freezeRecord<T extends object>(value: T): Readonly<T> {
  return Object.freeze(value);
}

export function snapshotPassiveRecord(
  value: unknown,
  path: string,
  expectedKeys?: ReadonlyArray<string>,
  code: NativeAdapterErrorCode = "INVALID_SOURCE_CONTRACT",
): Record<string, unknown> {
  try {
    if (
      typeof value !== "object" ||
      value === null ||
      Array.isArray(value) ||
      Object.getPrototypeOf(value) !== Object.prototype
    ) {
      return fail(code, path, "must be a non-Proxy plain object");
    }
    const keys = Reflect.ownKeys(value);
    if (keys.some((key) => typeof key === "symbol")) {
      return fail(code, path, "must not contain symbol keys");
    }
    const names = keys as ReadonlyArray<string>;
    if (expectedKeys !== undefined) {
      const extra = names.find((name) => !expectedKeys.includes(name));
      if (extra !== undefined) return fail(code, `${path}.${extra}`, "is not allowed");
      const missing = expectedKeys.find((name) => !names.includes(name));
      if (missing !== undefined) return fail(code, `${path}.${missing}`, "is required");
    }
    const descriptors = Object.getOwnPropertyDescriptors(value);
    const snapshot: Record<string, unknown> = {};
    for (const name of names) {
      const descriptor = descriptors[name];
      if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
        return fail(code, `${path}.${name}`, "must be an enumerable data property");
      }
      snapshot[name] = descriptor.value;
    }
    return snapshot;
  } catch (error) {
    if (error instanceof NativeAdapterError) throw error;
    return fail(code, path, "could not be inspected safely");
  }
}

export function snapshotPassiveArray(
  value: unknown,
  path: string,
  code: NativeAdapterErrorCode = "INVALID_SOURCE_CONTRACT",
): ReadonlyArray<unknown> {
  try {
    if (!Array.isArray(value)) return fail(code, path, "must be an array");
    const keys = Reflect.ownKeys(value);
    if (keys.some((key) => typeof key === "symbol")) {
      return fail(code, path, "must not contain symbol keys");
    }
    const names = keys as ReadonlyArray<string>;
    const unexpected = names.find((name) => name !== "length" && !/^\d+$/u.test(name));
    if (unexpected !== undefined) return fail(code, `${path}.${unexpected}`, "is not allowed");
    const lengthDescriptor = Object.getOwnPropertyDescriptor(value, "length");
    if (lengthDescriptor === undefined || !("value" in lengthDescriptor)) {
      return fail(code, path, "must have a data length");
    }
    const length = lengthDescriptor.value;
    if (!Number.isSafeInteger(length) || length < 0)
      return fail(code, path, "has an invalid length");
    const output: unknown[] = [];
    for (let index = 0; index < length; index += 1) {
      const name = String(index);
      const descriptor = Object.getOwnPropertyDescriptor(value, name);
      if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
        return fail(code, `${path}[${index}]`, "must be an enumerable data property");
      }
      output.push(descriptor.value);
    }
    return Object.freeze(output);
  } catch (error) {
    if (error instanceof NativeAdapterError) throw error;
    return fail(code, path, "could not be inspected safely");
  }
}

export interface AdapterSourceFile {
  readonly path: string;
  readonly digest: string;
  readonly role: "parser" | "fixture" | "bundle" | "notice";
}

export interface AdapterDependency {
  readonly name: string;
  readonly version: string;
  readonly digest: string;
  readonly spdxId: string;
  readonly noticeRef: string;
  readonly noticeDigest: string;
}

export interface AdapterLicense {
  readonly spdxId: string;
  readonly noticeRef: string;
  readonly digest: string;
}

/** Bytes are injected by a Controller-owned fixture; this boundary never reads a path. */
export interface SourceMaterialInput {
  readonly name: string;
  readonly bytes: ByteInput;
}

/** Normalized material is immutable plain data, not a live filesystem handle. */
export interface SourceMaterial {
  readonly name: string;
  readonly bytes: ReadonlyArray<number>;
}

export interface NativeAdapterSourceContract {
  readonly schema: typeof NATIVE_ADAPTER_SOURCE_SCHEMA;
  readonly adapterId: string;
  readonly adapterVersion: string;
  readonly source: {
    readonly repository: string;
    readonly revision: string;
    readonly paths: ReadonlyArray<string>;
    readonly digest: string;
  };
  readonly artifact: {
    readonly entrypoint: string;
    readonly digest: string;
  };
  readonly license: AdapterLicense;
  readonly nodeBundle: {
    readonly nodeVersion: string;
    readonly packageManager: string;
    readonly lockfileDigest: string;
    readonly bundleDigest: string;
    /** Production Node packages are not distributed by this contract. */
    readonly distributed: false;
  };
  readonly dependencies: ReadonlyArray<AdapterDependency>;
  readonly assets: ReadonlyArray<AdapterSourceFile>;
  readonly isolation: {
    readonly profile: "managed-private-root";
    readonly inheritedEnvironment: "allowlist";
    readonly network: "disabled";
    readonly credentials: "none";
    readonly userHome: "not-inherited";
  };
  readonly materials: ReadonlyArray<SourceMaterial>;
}

export type NativeAdapterSourceContractInput = Omit<NativeAdapterSourceContract, "materials">;

function copyTypedArrayBytes(value: Uint8Array, path: string): ReadonlyArray<number> {
  try {
    const keys = Reflect.ownKeys(value);
    if (keys.some((key) => typeof key === "symbol")) {
      return fail("INVALID_SOURCE_CONTRACT", path, "must not contain symbol keys");
    }
    const names = keys as ReadonlyArray<string>;
    const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
    const byteLengthGetter = Object.getOwnPropertyDescriptor(
      typedArrayPrototype,
      "byteLength",
    )?.get;
    if (byteLengthGetter === undefined)
      return fail("INVALID_SOURCE_CONTRACT", path, "has no byte length descriptor");
    const byteLength = byteLengthGetter.call(value);
    if (!Number.isSafeInteger(byteLength) || byteLength < 0) {
      return fail("INVALID_SOURCE_CONTRACT", path, "has an invalid byte length");
    }
    const bytes: number[] = [];
    for (const name of names) {
      const index = Number(name);
      if (
        !/^\d+$/u.test(name) ||
        !Number.isSafeInteger(index) ||
        index < 0 ||
        index >= byteLength
      ) {
        return fail("INVALID_SOURCE_CONTRACT", `${path}.${name}`, "is not an indexed byte");
      }
    }
    for (let index = 0; index < byteLength; index += 1) {
      const name = String(index);
      const descriptor = Object.getOwnPropertyDescriptor(value, name);
      if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
        return fail(
          "INVALID_SOURCE_CONTRACT",
          `${path}[${index}]`,
          "must be an enumerable data property",
        );
      }
      const byte = descriptor.value;
      if (typeof byte !== "number" || !Number.isInteger(byte) || byte < 0 || byte > 255) {
        return fail(
          "INVALID_SOURCE_CONTRACT",
          `${path}[${index}]`,
          "must be an integer byte from 0 to 255",
        );
      }
      bytes.push(byte);
    }
    return Object.freeze(bytes);
  } catch (error) {
    if (error instanceof NativeAdapterError) throw error;
    return fail("INVALID_SOURCE_CONTRACT", path, "could not copy typed-array bytes safely");
  }
}

function normalizeMaterialBytes(value: unknown, path: string): ReadonlyArray<number> {
  try {
    const values =
      value instanceof Uint8Array
        ? copyTypedArrayBytes(value, path)
        : snapshotPassiveArray(value, path, "INVALID_SOURCE_CONTRACT");
    const bytes = values.map((item, index) => {
      if (!Number.isInteger(item) || (item as number) < 0 || (item as number) > 255) {
        return fail(
          "INVALID_SOURCE_CONTRACT",
          `${path}[${index}]`,
          "must be an integer byte from 0 to 255",
        );
      }
      return item as number;
    });
    return Object.freeze(bytes);
  } catch (error) {
    if (error instanceof NativeAdapterError) throw error;
    return fail("INVALID_SOURCE_CONTRACT", path, "could not be copied safely");
  }
}

function materialKey(kind: string, name: string): string {
  return `${kind}:${name}`;
}

function sha256Bytes(bytes: ReadonlyArray<number>): string {
  return `sha256:${crypto.createHash("sha256").update(Uint8Array.from(bytes)).digest("hex")}`;
}

export const computeMaterialDigest = sha256Bytes;

export function computeSourceLedgerDigest(
  sourcePaths: ReadonlyArray<string>,
  materials: ReadonlyArray<SourceMaterial>,
): string {
  const byName = new Map(materials.map((material) => [material.name, material]));
  const hash = crypto.createHash("sha256");
  for (const path of [...sourcePaths].sort()) {
    const material = byName.get(materialKey("source", path));
    if (material === undefined) {
      return fail(
        "INVALID_SOURCE_CONTRACT",
        `source.materials.${materialKey("source", path)}`,
        "is missing",
      );
    }
    hash.update(new TextEncoder().encode(material.name));
    hash.update(Uint8Array.of(0));
    hash.update(Uint8Array.from(material.bytes));
    hash.update(Uint8Array.of(0));
  }
  return `sha256:${hash.digest("hex")}`;
}

function normalizeSourceMaterials(
  raw: ReadonlyArray<SourceMaterialInput>,
): ReadonlyArray<SourceMaterial> {
  const entries = snapshotPassiveArray(raw, "source.materials", "INVALID_SOURCE_CONTRACT");
  const names = new Set<string>();
  const normalized = entries.map((entry, index) => {
    const record = snapshotPassiveRecord(
      entry,
      `source.materials[${index}]`,
      ["name", "bytes"],
      "INVALID_SOURCE_CONTRACT",
    );
    const name = safeName(record.name, `source.materials[${index}].name`);
    if (names.has(name))
      return fail("INVALID_SOURCE_CONTRACT", `source.materials[${index}].name`, "is duplicated");
    names.add(name);
    return freezeRecord({
      name,
      bytes: normalizeMaterialBytes(record.bytes, `source.materials[${index}].bytes`),
    });
  });
  return freezeArray(normalized);
}

/**
 * Validate and freeze the source ledger before a descriptor can be attached.
 * The ledger is deliberately explicit: source, lock/runtime, assets,
 * dependencies, license notices and isolation are all retained together.
 */
export function validateSourceContract(
  raw: NativeAdapterSourceContractInput,
  rawMaterials: ReadonlyArray<SourceMaterialInput>,
): Readonly<NativeAdapterSourceContract> {
  const sourceRecord = snapshotPassiveRecord(raw, "source", [
    "schema",
    "adapterId",
    "adapterVersion",
    "source",
    "artifact",
    "license",
    "nodeBundle",
    "dependencies",
    "assets",
    "isolation",
  ]);
  if (sourceRecord.schema !== NATIVE_ADAPTER_SOURCE_SCHEMA) {
    return fail("INVALID_SOURCE_CONTRACT", "source.schema", "is unsupported");
  }
  const adapterId = safeName(sourceRecord.adapterId, "source.adapterId");
  const adapterVersion = safeName(sourceRecord.adapterVersion, "source.adapterVersion");
  const sourceInfo = snapshotPassiveRecord(sourceRecord.source, "source.source", [
    "repository",
    "revision",
    "paths",
    "digest",
  ]);
  const repository = nonEmpty(sourceInfo.repository, "source.source.repository");
  const sourceRevision = revision(sourceInfo.revision, "source.source.revision");
  const sourceDigest = digest(sourceInfo.digest, "source.source.digest");
  const sourcePaths = snapshotPassiveArray(
    sourceInfo.paths,
    "source.source.paths",
    "INVALID_SOURCE_CONTRACT",
  );
  if (sourcePaths.length === 0) {
    return fail("INVALID_SOURCE_CONTRACT", "source.source.paths", "must contain at least one path");
  }
  const paths = freezeArray(
    sourcePaths.map((path, index) => relativePath(path, `source.source.paths[${index}]`)),
  );
  const artifactInfo = snapshotPassiveRecord(sourceRecord.artifact, "source.artifact", [
    "entrypoint",
    "digest",
  ]);
  const artifactEntrypoint = relativePath(artifactInfo.entrypoint, "source.artifact.entrypoint");
  const artifactDigest = digest(artifactInfo.digest, "source.artifact.digest");
  const licenseInfo = snapshotPassiveRecord(sourceRecord.license, "source.license", [
    "spdxId",
    "noticeRef",
    "digest",
  ]);
  const normalizedLicense = freezeRecord({
    spdxId: spdxId(licenseInfo.spdxId, "source.license.spdxId"),
    noticeRef: relativePath(licenseInfo.noticeRef, "source.license.noticeRef"),
    digest: digest(licenseInfo.digest, "source.license.digest"),
  });
  const nodeBundle = snapshotPassiveRecord(sourceRecord.nodeBundle, "source.nodeBundle", [
    "nodeVersion",
    "packageManager",
    "lockfileDigest",
    "bundleDigest",
    "distributed",
  ]);
  const nodeVersion = nonEmpty(nodeBundle.nodeVersion, "source.nodeBundle.nodeVersion");
  const packageManager = nonEmpty(nodeBundle.packageManager, "source.nodeBundle.packageManager");
  const lockfileDigest = digest(nodeBundle.lockfileDigest, "source.nodeBundle.lockfileDigest");
  const bundleDigest = digest(nodeBundle.bundleDigest, "source.nodeBundle.bundleDigest");
  if (nodeBundle.distributed !== false) {
    return fail("INVALID_SOURCE_CONTRACT", "source.nodeBundle.distributed", "must remain false");
  }
  const dependencies = snapshotPassiveArray(
    sourceRecord.dependencies,
    "source.dependencies",
    "INVALID_SOURCE_CONTRACT",
  );
  const normalizedDependencies = freezeArray(
    dependencies.map((dependency, index) => {
      const dependencyRecord = snapshotPassiveRecord(dependency, `source.dependencies[${index}]`, [
        "name",
        "version",
        "digest",
        "spdxId",
        "noticeRef",
        "noticeDigest",
      ]);
      return freezeRecord({
        name: safeName(dependencyRecord.name, `source.dependencies[${index}].name`),
        version: nonEmpty(dependencyRecord.version, `source.dependencies[${index}].version`),
        digest: digest(dependencyRecord.digest, `source.dependencies[${index}].digest`),
        spdxId: spdxId(dependencyRecord.spdxId, `source.dependencies[${index}].spdxId`),
        noticeRef: relativePath(
          dependencyRecord.noticeRef,
          `source.dependencies[${index}].noticeRef`,
        ),
        noticeDigest: digest(
          dependencyRecord.noticeDigest,
          `source.dependencies[${index}].noticeDigest`,
        ),
      });
    }),
  );
  const assets = snapshotPassiveArray(
    sourceRecord.assets,
    "source.assets",
    "INVALID_SOURCE_CONTRACT",
  );
  const normalizedAssets = freezeArray(
    assets.map((asset, index) => {
      const assetRecord = snapshotPassiveRecord(asset, `source.assets[${index}]`, [
        "path",
        "digest",
        "role",
      ]);
      if (
        assetRecord.role !== "parser" &&
        assetRecord.role !== "fixture" &&
        assetRecord.role !== "bundle" &&
        assetRecord.role !== "notice"
      ) {
        return fail("INVALID_SOURCE_CONTRACT", `source.assets[${index}].role`, "is unsupported");
      }
      return freezeRecord({
        path: relativePath(assetRecord.path, `source.assets[${index}].path`),
        digest: digest(assetRecord.digest, `source.assets[${index}].digest`),
        role: assetRecord.role as "parser" | "fixture" | "bundle" | "notice",
      });
    }),
  );
  const isolation = snapshotPassiveRecord(sourceRecord.isolation, "source.isolation", [
    "profile",
    "inheritedEnvironment",
    "network",
    "credentials",
    "userHome",
  ]);
  if (
    isolation.profile !== "managed-private-root" ||
    isolation.inheritedEnvironment !== "allowlist" ||
    isolation.network !== "disabled" ||
    isolation.credentials !== "none" ||
    isolation.userHome !== "not-inherited"
  ) {
    return fail(
      "INVALID_SOURCE_CONTRACT",
      "source.isolation",
      "does not satisfy the managed fake-only boundary",
    );
  }
  const materials = normalizeSourceMaterials(rawMaterials);
  const materialByName = new Map(materials.map((material) => [material.name, material]));
  const expectedNames = new Set<string>();
  const expectMaterial = (name: string): SourceMaterial => {
    if (expectedNames.has(name))
      return fail("INVALID_SOURCE_CONTRACT", `source.materials.${name}`, "is duplicated");
    expectedNames.add(name);
    const material = materialByName.get(name);
    if (material === undefined)
      return fail("INVALID_SOURCE_CONTRACT", `source.materials.${name}`, "is missing");
    return material;
  };
  for (const path of paths) expectMaterial(materialKey("source", path));
  if (
    sha256Bytes(expectMaterial(materialKey("artifact", artifactEntrypoint)).bytes) !==
    artifactDigest
  ) {
    return fail(
      "INVALID_SOURCE_CONTRACT",
      "source.artifact.digest",
      "does not match injected material bytes",
    );
  }
  if (
    sha256Bytes(expectMaterial(materialKey("license", normalizedLicense.noticeRef)).bytes) !==
    normalizedLicense.digest
  ) {
    return fail(
      "INVALID_SOURCE_CONTRACT",
      "source.license.digest",
      "does not match injected material bytes",
    );
  }
  const lockfile = expectMaterial(materialKey("nodeBundle", "lockfile"));
  if (sha256Bytes(lockfile.bytes) !== lockfileDigest) {
    return fail(
      "INVALID_SOURCE_CONTRACT",
      "source.nodeBundle.lockfileDigest",
      "does not match injected material bytes",
    );
  }
  const bundle = expectMaterial(materialKey("nodeBundle", "bundle"));
  if (sha256Bytes(bundle.bytes) !== bundleDigest) {
    return fail(
      "INVALID_SOURCE_CONTRACT",
      "source.nodeBundle.bundleDigest",
      "does not match injected material bytes",
    );
  }
  for (const [index, dependency] of dependencies.entries()) {
    const record = snapshotPassiveRecord(dependency, `source.dependencies[${index}]`, [
      "name",
      "version",
      "digest",
      "spdxId",
      "noticeRef",
      "noticeDigest",
    ]);
    const dependencyName = safeName(record.name, `source.dependencies[${index}].name`);
    const dependencyVersion = nonEmpty(record.version, `source.dependencies[${index}].version`);
    const material = expectMaterial(
      materialKey("dependency", `${dependencyName}@${dependencyVersion}`),
    );
    if (sha256Bytes(material.bytes) !== record.digest) {
      return fail(
        "INVALID_SOURCE_CONTRACT",
        `source.dependencies[${index}].digest`,
        "does not match injected material bytes",
      );
    }
    const noticeRef = relativePath(record.noticeRef, `source.dependencies[${index}].noticeRef`);
    const noticeMaterial = expectMaterial(
      materialKey("dependencyNotice", `${dependencyName}@${dependencyVersion}:${noticeRef}`),
    );
    if (sha256Bytes(noticeMaterial.bytes) !== record.noticeDigest) {
      return fail(
        "INVALID_SOURCE_CONTRACT",
        `source.dependencies[${index}].noticeDigest`,
        "does not match injected material bytes",
      );
    }
  }
  for (const [index, asset] of assets.entries()) {
    const record = snapshotPassiveRecord(asset, `source.assets[${index}]`, [
      "path",
      "digest",
      "role",
    ]);
    const assetPath = relativePath(record.path, `source.assets[${index}].path`);
    const material = expectMaterial(materialKey("asset", assetPath));
    if (sha256Bytes(material.bytes) !== record.digest) {
      return fail(
        "INVALID_SOURCE_CONTRACT",
        `source.assets[${index}].digest`,
        "does not match injected material bytes",
      );
    }
  }
  if (expectedNames.size !== materialByName.size) {
    const extra = [...materialByName.keys()].find((name) => !expectedNames.has(name));
    return fail("INVALID_SOURCE_CONTRACT", `source.materials.${extra ?? "unknown"}`, "is unbound");
  }
  const sourceDigestFromMaterials = computeSourceLedgerDigest(paths, materials);
  if (sourceDigestFromMaterials !== sourceDigest) {
    return fail(
      "INVALID_SOURCE_CONTRACT",
      "source.source.digest",
      "does not match injected source material bytes",
    );
  }
  return freezeRecord({
    schema: NATIVE_ADAPTER_SOURCE_SCHEMA,
    adapterId,
    adapterVersion,
    source: freezeRecord({ repository, revision: sourceRevision, paths, digest: sourceDigest }),
    artifact: freezeRecord({ entrypoint: artifactEntrypoint, digest: artifactDigest }),
    license: normalizedLicense,
    nodeBundle: freezeRecord({
      nodeVersion,
      packageManager,
      lockfileDigest,
      bundleDigest,
      distributed: false,
    }),
    dependencies: normalizedDependencies,
    assets: normalizedAssets,
    isolation: freezeRecord({
      profile: "managed-private-root",
      inheritedEnvironment: "allowlist",
      network: "disabled",
      credentials: "none",
      userHome: "not-inherited",
    }),
    materials,
  });
}

export interface NativeAdapterBinding {
  readonly runtimeInstanceId: string;
  readonly profileId: string;
  readonly authRevision: string;
  readonly generation: string;
  readonly scope: "public" | "legacy";
}

export function validateBinding(raw: NativeAdapterBinding): Readonly<NativeAdapterBinding> {
  const binding = snapshotPassiveRecord(
    raw,
    "binding",
    ["runtimeInstanceId", "profileId", "authRevision", "generation", "scope"],
    "INVALID_LAUNCH",
  );
  for (const field of ["runtimeInstanceId", "profileId"] as const) {
    const value = binding[field];
    if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
      return fail("INVALID_LAUNCH", `binding.${field}`, "must be canonical and non-empty");
    }
  }
  for (const field of ["authRevision", "generation"] as const) {
    if (
      typeof binding[field] !== "string" ||
      !CANONICAL_U64.test(binding[field]) ||
      BigInt(binding[field]) > U64_MAX
    ) {
      return fail("INVALID_LAUNCH", `binding.${field}`, "must be a canonical u64 string");
    }
  }
  if (binding.scope !== "public" && binding.scope !== "legacy") {
    return fail("INVALID_LAUNCH", "binding.scope", "must be public or legacy");
  }
  return freezeRecord({
    runtimeInstanceId: binding.runtimeInstanceId as string,
    profileId: binding.profileId as string,
    authRevision: binding.authRevision as string,
    generation: binding.generation as string,
    scope: binding.scope as "public" | "legacy",
  });
}

export interface NativeLaunchSpec {
  readonly adapterId: string;
  readonly adapterVersion: string;
  readonly binding: NativeAdapterBinding;
  readonly custodyRef: string;
  /** A fixture/managed reference, not an executable path resolved by this port. */
  readonly executableRef: string;
  readonly cwdRef: string;
  readonly environment: Readonly<Record<string, string>>;
}

/**
 * Build an environment from explicit input only.  No process environment is
 * read or inherited.  Secret-looking names are intentionally dropped before
 * their values are observed; callers may pass a narrower allowlist for a
 * fixture or managed runtime.
 */
export function sanitizeEnvironment(
  raw: Readonly<Record<string, string>>,
  allowedNames: ReadonlySet<string> = DEFAULT_ENVIRONMENT_NAMES,
): Readonly<Record<string, string>> {
  try {
    if (
      typeof raw !== "object" ||
      raw === null ||
      Array.isArray(raw) ||
      Object.getPrototypeOf(raw) !== Object.prototype
    ) {
      return fail("INVALID_LAUNCH", "launch.environment", "must be a non-Proxy plain object");
    }
    const keys = Reflect.ownKeys(raw);
    if (keys.some((key) => typeof key === "symbol")) {
      return fail("INVALID_LAUNCH", "launch.environment", "must not contain symbol keys");
    }
    const names = keys as ReadonlyArray<string>;
    const descriptors = Object.getOwnPropertyDescriptors(raw);
    const environment: Record<string, string> = {};
    for (const name of names) {
      // Do this before touching the descriptor value: secret accessors are never invoked.
      if (SECRET_NAME.test(name)) continue;
      if (!ENV_NAME.test(name) || !allowedNames.has(name)) {
        return fail(
          "INVALID_LAUNCH",
          `launch.environment.${name}`,
          "is not in the explicit allowlist",
        );
      }
      const descriptor = descriptors[name];
      if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
        return fail(
          "INVALID_LAUNCH",
          `launch.environment.${name}`,
          "must be an enumerable data property",
        );
      }
      const value = descriptor.value;
      if (typeof value !== "string" || value.includes("\0")) {
        return fail("INVALID_LAUNCH", `launch.environment.${name}`, "must be a NUL-free string");
      }
      environment[name] = value;
    }
    return freezeRecord(environment);
  } catch (error) {
    if (error instanceof NativeAdapterError) throw error;
    return fail("INVALID_LAUNCH", "launch.environment", "could not be inspected safely");
  }
}

export function validateLaunchSpec(
  raw: NativeLaunchSpec,
  allowedEnvironmentNames?: ReadonlySet<string>,
): Readonly<NativeLaunchSpec> {
  const launch = snapshotPassiveRecord(
    raw,
    "launch",
    [
      "adapterId",
      "adapterVersion",
      "binding",
      "custodyRef",
      "executableRef",
      "cwdRef",
      "environment",
    ],
    "INVALID_LAUNCH",
  );
  const adapterId = safeName(launch.adapterId, "launch.adapterId");
  const adapterVersion = safeName(launch.adapterVersion, "launch.adapterVersion");
  const binding = validateBinding(launch.binding as NativeAdapterBinding);
  const custodyRef = launch.custodyRef;
  if (
    typeof custodyRef !== "string" ||
    custodyRef.length === 0 ||
    custodyRef !== custodyRef.trim() ||
    custodyRef.includes("\0")
  ) {
    return fail("INVALID_LAUNCH", "launch.custodyRef", "must be canonical and non-empty");
  }
  const executableRef = relativePath(launch.executableRef, "launch.executableRef");
  const cwdRef = relativePath(launch.cwdRef, "launch.cwdRef");
  const environment = sanitizeEnvironment(
    launch.environment as Readonly<Record<string, string>>,
    allowedEnvironmentNames,
  );
  return freezeRecord({
    adapterId,
    adapterVersion,
    binding,
    custodyRef,
    executableRef,
    cwdRef,
    environment,
  });
}

export interface NativeOperationContext {
  readonly signal: AbortSignal;
  readonly deadlineAt: number;
  readonly binding: Readonly<NativeAdapterBinding>;
  readonly custodyRef: string;
}

export interface ProcessIdentity {
  readonly processId: number;
  readonly creationTime: string;
}

export function validateProcessIdentity(raw: ProcessIdentity): Readonly<ProcessIdentity> {
  const identity = snapshotPassiveRecord(
    raw,
    "processIdentity",
    ["processId", "creationTime"],
    "PROCESS_IDENTITY_UNKNOWN",
  );
  if (
    !Number.isSafeInteger(identity.processId) ||
    (identity.processId as number) <= 0 ||
    typeof identity.creationTime !== "string" ||
    !CANONICAL_U64.test(identity.creationTime as string) ||
    BigInt(identity.creationTime as string) > U64_MAX
  ) {
    throw new NativeAdapterError(
      "PROCESS_IDENTITY_UNKNOWN",
      "process identity must contain a positive pid and canonical creation time",
    );
  }
  return freezeRecord({
    processId: identity.processId as number,
    creationTime: identity.creationTime as string,
  });
}

export interface NativeDataFrame {
  readonly kind: "data";
  readonly bytes: NativeBytes;
}

export interface NativeEofFrame {
  readonly kind: "eof";
  readonly reason: string;
}

export interface NativeErrorFrame {
  readonly kind: "error";
  readonly code: string;
  readonly message: string;
  readonly fatal: boolean;
}

export type NativeFrame = NativeDataFrame | NativeEofFrame | NativeErrorFrame;

export function dataFrame(
  bytes: ByteInput,
  maxBytes = NATIVE_FRAME_MAX_BYTES,
): Readonly<NativeDataFrame> {
  return freezeRecord({ kind: "data", bytes: cloneBytes(bytes, "frame.bytes", maxBytes) });
}

export function eofFrame(reason = "peer closed"): Readonly<NativeEofFrame> {
  if (typeof reason !== "string" || reason.length === 0 || reason.includes("\0")) {
    throw new NativeAdapterError(
      "INVALID_FRAME",
      "frame.eof.reason must be a non-empty NUL-free string",
    );
  }
  return freezeRecord({ kind: "eof", reason });
}

export function errorFrame(
  code: string,
  message: string,
  fatal = true,
): Readonly<NativeErrorFrame> {
  if (
    typeof code !== "string" ||
    code.length === 0 ||
    typeof message !== "string" ||
    message.length === 0 ||
    message.includes("\0") ||
    typeof fatal !== "boolean"
  ) {
    throw new NativeAdapterError("INVALID_FRAME", "frame.error requires non-empty strings");
  }
  return freezeRecord({ kind: "error", code, message, fatal });
}

export function cloneFrame(
  frame: NativeFrame,
  maxBytes = NATIVE_FRAME_MAX_BYTES,
): Readonly<NativeFrame> {
  const record = snapshotPassiveRecord(frame, "frame", undefined, "INVALID_FRAME");
  if (record.kind === "data") {
    const data = snapshotPassiveRecord(record, "frame", ["kind", "bytes"], "INVALID_FRAME");
    return dataFrame(data.bytes as ByteInput, maxBytes);
  }
  if (record.kind === "eof") {
    const eof = snapshotPassiveRecord(record, "frame", ["kind", "reason"], "INVALID_FRAME");
    return eofFrame(eof.reason as string);
  }
  if (record.kind === "error") {
    const error = snapshotPassiveRecord(
      record,
      "frame",
      ["kind", "code", "message", "fatal"],
      "INVALID_FRAME",
    );
    return errorFrame(error.code as string, error.message as string, error.fatal as boolean);
  }
  throw new NativeAdapterError("INVALID_FRAME", "frame.kind is unsupported");
}

export interface NativeCloseReceipt {
  readonly status: "closed" | "already-closed";
  readonly reason: string;
  readonly binding: Readonly<NativeAdapterBinding>;
  readonly custodyRef: string;
  readonly processIdentity: ProcessIdentity | null;
}

export function cloneBytes(
  input: ByteInput,
  path = "bytes",
  maxBytes = NATIVE_FRAME_MAX_BYTES,
): NativeBytes {
  try {
    let bytes: Uint8Array;
    if (input instanceof Uint8Array) {
      bytes = new Uint8Array(input);
    } else if (Array.isArray(input)) {
      const values = snapshotPassiveArray(input, path, "INVALID_FRAME");
      for (const [index, value] of values.entries()) {
        if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > 255) {
          return fail(
            "INVALID_FRAME",
            `${path}[${index}]`,
            "must be an integer byte from 0 to 255",
          );
        }
      }
      bytes = Uint8Array.from(values as ReadonlyArray<number>);
    } else {
      return fail("INVALID_FRAME", path, "must be a Uint8Array or byte array");
    }
    if (bytes.byteLength > maxBytes) {
      throw new NativeAdapterError("FRAME_TOO_LARGE", `${path} exceeds ${maxBytes} bytes`);
    }
    return bytes;
  } catch (error) {
    if (error instanceof NativeAdapterError) throw error;
    return fail("INVALID_FRAME", path, "could not be copied safely");
  }
}

export function bytesEqual(left: ByteInput, right: ByteInput): boolean {
  const a = cloneBytes(left);
  const b = cloneBytes(right);
  if (a.byteLength !== b.byteLength) return false;
  for (let index = 0; index < a.byteLength; index += 1) {
    if (a[index] !== b[index]) return false;
  }
  return true;
}

export function assertOperationUsable(context: NativeOperationContext): void {
  if (
    typeof context.custodyRef !== "string" ||
    context.custodyRef.length === 0 ||
    context.custodyRef !== context.custodyRef.trim() ||
    context.custodyRef.includes("\0")
  ) {
    throw new NativeAdapterError("INVALID_LAUNCH", "operation custody reference is invalid");
  }
  if (context.signal.aborted) {
    throw new NativeAdapterError("CANCELLED", "operation signal is aborted", context.signal.reason);
  }
  if (!Number.isFinite(context.deadlineAt) || context.deadlineAt <= 0) {
    throw new NativeAdapterError("DEADLINE_EXCEEDED", "operation deadline is invalid");
  }
  if (Date.now() > context.deadlineAt) {
    throw new NativeAdapterError("DEADLINE_EXCEEDED", "operation deadline has elapsed");
  }
}

export function bindingKey(binding: NativeAdapterBinding): string {
  const value = validateBinding(binding);
  return JSON.stringify([
    value.runtimeInstanceId,
    value.profileId,
    value.authRevision,
    value.generation,
    value.scope,
  ]);
}

export function assertBindingMatches(
  expected: NativeAdapterBinding,
  actual: NativeAdapterBinding,
): void {
  if (bindingKey(expected) !== bindingKey(actual)) {
    throw new NativeAdapterError(
      "INVALID_LAUNCH",
      "operation binding does not match the admitted runtime instance",
    );
  }
}

/** Race an adapter operation against its explicit cancellation/deadline. */
export function awaitOperation<T>(
  operation: Promise<T>,
  context: NativeOperationContext,
): Promise<T> {
  assertOperationUsable(context);
  return new Promise<T>((resolve, reject) => {
    let settled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const cleanup = (): void => {
      if (timer !== undefined) clearTimeout(timer);
      context.signal.removeEventListener("abort", onAbort);
    };
    const finish = (fn: () => void): void => {
      if (settled) return;
      settled = true;
      cleanup();
      fn();
    };
    const onAbort = (): void =>
      finish(() =>
        reject(
          new NativeAdapterError("CANCELLED", "operation signal is aborted", context.signal.reason),
        ),
      );
    const remaining = context.deadlineAt - Date.now();
    if (remaining <= 0) {
      finish(() =>
        reject(new NativeAdapterError("DEADLINE_EXCEEDED", "operation deadline has elapsed")),
      );
      return;
    }
    context.signal.addEventListener("abort", onAbort, { once: true });
    timer = setTimeout(
      () =>
        finish(() =>
          reject(new NativeAdapterError("DEADLINE_EXCEEDED", "operation deadline has elapsed")),
        ),
      remaining,
    );
    void operation.then(
      (value) =>
        finish(() => {
          try {
            assertOperationUsable(context);
            resolve(value);
          } catch (error) {
            reject(error);
          }
        }),
      (error) => finish(() => reject(error)),
    );
  });
}
