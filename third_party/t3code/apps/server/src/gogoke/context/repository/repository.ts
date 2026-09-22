import * as NodeUtilTypes from "node:util/types";

import type { ContextObject, JsonObject } from "../../contracts/model.ts";

const ID = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,255}$/u;
const U64 = /^(?:0|[1-9]\d*)$/u;
const SHA256 = /^sha256:[0-9a-f]{64}$/u;
const VERSION_REF = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,255}@(0|[1-9]\d*)$/u;

export interface ContextAccessPolicy {
  readonly visibility: "OWNER_PRIVATE" | "DOMAIN_GRANTED";
  readonly readGrantRefs: ReadonlyArray<string>;
}

export interface ContextPromotionEvidence {
  readonly sourceVersionRef: string;
  readonly sourceGrantRef: string;
  readonly targetGrantRef: string;
  readonly provenanceRefs: ReadonlyArray<string>;
}

export interface CommitContextVersionRequest {
  readonly operationId: string;
  readonly object: ContextObject;
  readonly access: ContextAccessPolicy;
  readonly promotion?: ContextPromotionEvidence;
}

export interface ContextCommitReceipt {
  readonly operationId: string;
  readonly contextId: string;
  readonly version: string;
  readonly disposition: "COMMITTED" | "RECONCILED";
  readonly invalidatedVersionRefs: ReadonlyArray<string>;
}

export interface ContextStorePort {
  /** Rust revalidates and applies the complete version/status graph in one transaction. */
  commitContextVersion(request: CommitContextVersionRequest): Promise<ContextCommitReceipt>;
}

export class ContextRepositoryError extends Error {
  override readonly name = "ContextRepositoryError";
}

const invalid = (path: string, detail: string): never => {
  throw new ContextRepositoryError(`${path} ${detail}`);
};

function passiveRecord(value: unknown, path: string): Readonly<Record<string, unknown>> {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    NodeUtilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) {
    return invalid(path, "must be a non-Proxy plain object");
  }
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key === "symbol")) return invalid(path, "must not have symbols");
  const result: Record<string, unknown> = Object.create(null);
  for (const key of keys as string[]) {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (descriptor === undefined || !Object.hasOwn(descriptor, "value") || !descriptor.enumerable) {
      return invalid(`${path}.${key}`, "must be an enumerable data property");
    }
    result[key] = descriptor.value;
  }
  return Object.freeze(result);
}

const text = (value: unknown, path: string): string => {
  if (typeof value !== "string" || !ID.test(value)) return invalid(path, "is not canonical");
  return value;
};

const u64 = (value: unknown, path: string): string => {
  if (
    typeof value !== "string" ||
    !U64.test(value) ||
    value.length > 20 ||
    BigInt(value) > 18_446_744_073_709_551_615n
  ) {
    return invalid(path, "must be a canonical uint64 string");
  }
  return value;
};

function strings(value: unknown, path: string): ReadonlyArray<string> {
  const entries = passiveArray(value, path);
  const result = entries.map((entry, index) => text(entry, `${path}[${index}]`));
  if (new Set(result).size !== result.length) return invalid(path, "must not contain duplicates");
  return Object.freeze(result);
}

function versionReferences(value: unknown, path: string): ReadonlyArray<string> {
  const entries = passiveArray(value, path);
  const result = entries.map((entry, index) => versionReference(entry, `${path}[${index}]`));
  if (new Set(result).size !== result.length) return invalid(path, "must not contain duplicates");
  return Object.freeze(result);
}

/** Internal descriptor construction must not consult inherited get/set fields. */
function defineDataProperty(target: object, key: string, value: unknown): void {
  const descriptor: PropertyDescriptor = Object.create(null);
  descriptor.value = value;
  descriptor.enumerable = true;
  Object.defineProperty(target, key, descriptor);
}

function passiveArray(value: unknown, path: string): ReadonlyArray<unknown> {
  if (
    !Array.isArray(value) ||
    NodeUtilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Array.prototype
  ) {
    return invalid(path, "must be a non-Proxy plain array");
  }
  const keys = Reflect.ownKeys(value);
  const allowed = new Set<string>(["length"]);
  const result: unknown[] = [];
  for (let index = 0; index < value.length; index += 1) {
    const key = String(index);
    allowed.add(key);
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (descriptor === undefined || !Object.hasOwn(descriptor, "value") || !descriptor.enumerable) {
      return invalid(`${path}[${index}]`, "must be an enumerable data property");
    }
    defineDataProperty(result, key, descriptor.value);
  }
  if (keys.some((key) => typeof key !== "string" || !allowed.has(key))) {
    return invalid(path, "has extra properties");
  }
  return Object.freeze(result);
}

const versionReference = (value: unknown, path: string): string => {
  if (typeof value !== "string" || !VERSION_REF.test(value)) {
    return invalid(path, "must be a canonical context version reference");
  }
  u64(value.slice(value.lastIndexOf("@") + 1), `${path}.version`);
  return value;
};

function source(value: unknown): JsonObject {
  const record = passiveRecord(value, "object.sourceRef");
  const ref = text(record.ref, "object.sourceRef.ref");
  if (typeof record.hash !== "string" || !SHA256.test(record.hash)) {
    return invalid("object.sourceRef.hash", "must be a lowercase sha256 digest");
  }
  if (Reflect.ownKeys(record).length !== 2) return invalid("object.sourceRef", "has extra fields");
  return Object.freeze({ ref, hash: record.hash });
}

function authority(value: unknown): JsonObject {
  const record = passiveRecord(value, "object.sourceAuthority");
  if (Reflect.ownKeys(record).length !== 2) {
    return invalid("object.sourceAuthority", "has extra fields");
  }
  return Object.freeze({
    kind: text(record.kind, "object.sourceAuthority.kind"),
    ref: text(record.ref, "object.sourceAuthority.ref"),
  });
}

function snapshotObject(value: unknown): ContextObject {
  const raw = passiveRecord(value, "object");
  const expected = [
    "contextId",
    "version",
    "scope",
    "domainId",
    "kind",
    "contentHash",
    "sourceRef",
    "sourceAuthority",
    "derivedFrom",
    "validity",
    "supersedes",
    "accessPolicyRevision",
  ];
  if (Reflect.ownKeys(raw).length !== expected.length || expected.some((key) => !Object.hasOwn(raw, key))) {
    return invalid("object", "must contain the exact ContextObject fields");
  }
  if (typeof raw.contentHash !== "string" || !SHA256.test(raw.contentHash)) {
    return invalid("object.contentHash", "must be a lowercase sha256 digest");
  }
  if (!(["GLOBAL", "PROJECT", "SESSION"] as const).includes(raw.scope as never)) {
    return invalid("object.scope", "is invalid");
  }
  if (
    !(["ACTIVE", "SUPERSEDED", "CONFLICTED", "STALE", "REVOKED", "ARCHIVED"] as const).includes(
      raw.validity as never,
    )
  ) {
    return invalid("object.validity", "is invalid");
  }
  return Object.freeze({
    contextId: text(raw.contextId, "object.contextId"),
    version: u64(raw.version, "object.version") as ContextObject["version"],
    scope: raw.scope as ContextObject["scope"],
    domainId: text(raw.domainId, "object.domainId"),
    kind: text(raw.kind, "object.kind"),
    contentHash: raw.contentHash,
    sourceRef: source(raw.sourceRef),
    sourceAuthority: authority(raw.sourceAuthority),
    derivedFrom: versionReferences(raw.derivedFrom, "object.derivedFrom"),
    validity: raw.validity as ContextObject["validity"],
    supersedes: versionReferences(raw.supersedes, "object.supersedes"),
    accessPolicyRevision: u64(
      raw.accessPolicyRevision,
      "object.accessPolicyRevision",
    ) as ContextObject["accessPolicyRevision"],
  });
}

function versionRef(object: ContextObject): string {
  return `${object.contextId}@${object.version}`;
}

function snapshotRequest(value: unknown): CommitContextVersionRequest {
  const raw = passiveRecord(value, "request");
  const allowed = new Set(["operationId", "object", "access", "promotion"]);
  if (Reflect.ownKeys(raw).some((key) => typeof key !== "string" || !allowed.has(key))) {
    return invalid("request", "has extra fields");
  }
  const object = snapshotObject(raw.object);
  if (object.validity !== "ACTIVE") return invalid("object.validity", "must begin ACTIVE");
  if (object.supersedes.includes(versionRef(object))) {
    return invalid("object.supersedes", "must not supersede itself");
  }
  const accessRaw = passiveRecord(raw.access, "access");
  if (Reflect.ownKeys(accessRaw).length !== 2) return invalid("access", "has extra fields");
  if (accessRaw.visibility !== "OWNER_PRIVATE" && accessRaw.visibility !== "DOMAIN_GRANTED") {
    return invalid("access.visibility", "is invalid");
  }
  const access = Object.freeze({
    visibility: accessRaw.visibility,
    readGrantRefs: strings(accessRaw.readGrantRefs, "access.readGrantRefs"),
  });
  let promotion: ContextPromotionEvidence | undefined;
  if (raw.promotion !== undefined) {
    const promotionRaw = passiveRecord(raw.promotion, "promotion");
    if (Reflect.ownKeys(promotionRaw).length !== 4) return invalid("promotion", "has extra fields");
    promotion = Object.freeze({
      sourceVersionRef: versionReference(
        promotionRaw.sourceVersionRef,
        "promotion.sourceVersionRef",
      ),
      sourceGrantRef: text(promotionRaw.sourceGrantRef, "promotion.sourceGrantRef"),
      targetGrantRef: text(promotionRaw.targetGrantRef, "promotion.targetGrantRef"),
      provenanceRefs: strings(promotionRaw.provenanceRefs, "promotion.provenanceRefs"),
    });
  }
  if (object.scope === "GLOBAL") {
    if (promotion === undefined) return invalid("promotion", "is required for GLOBAL context");
    if (!object.derivedFrom.includes(promotion.sourceVersionRef)) {
      return invalid("object.derivedFrom", "must include the promoted source version");
    }
    if (promotion.provenanceRefs.length === 0) {
      return invalid("promotion.provenanceRefs", "must not be empty");
    }
  } else if (promotion !== undefined) {
    return invalid("promotion", "is only valid for GLOBAL context");
  }
  return Object.freeze({
    operationId: text(raw.operationId, "request.operationId"),
    object,
    access,
    ...(promotion === undefined ? {} : { promotion }),
  });
}

export class ContextRepository {
  readonly #commit: ContextStorePort["commitContextVersion"];

  constructor(port: ContextStorePort) {
    const raw = passiveRecord(port, "port");
    if (Reflect.ownKeys(raw).length !== 1 || typeof raw.commitContextVersion !== "function") {
      throw new ContextRepositoryError("port must expose only commitContextVersion");
    }
    this.#commit = raw.commitContextVersion.bind(port) as ContextStorePort["commitContextVersion"];
  }

  commit(input: CommitContextVersionRequest): Promise<ContextCommitReceipt> {
    let request: CommitContextVersionRequest;
    try {
      request = snapshotRequest(input);
    } catch (error) {
      return Promise.reject(error);
    }
    return Promise.resolve().then(() => this.#commit(request));
  }
}

export const contextVersionRef = versionRef;
