import * as NodeUtilTypes from "node:util/types";

import type { NativeDelegationGrantSnapshot } from "../persistence/base/nativeHostClient.ts";
import type { NativeDelegationGrantReader } from "../bootstrap/nativeStoreService.ts";
import {
  MATERIAL_SINKS,
  POLICY_ACTIONS,
  PolicyError,
  type AuthorityCeiling,
  type AuthorityGrant,
  type PolicyAuthorityPort,
  type PolicyBinding,
  type PolicyPrincipal,
} from "./types.ts";

const U64_MAX = 18_446_744_073_709_551_615n;
const U64_PATTERN = /^(?:0|[1-9][0-9]*)$/;

const invalid = (path: string, detail: string): never => {
  throw new PolicyError("INVALID_INPUT", `${path} ${detail}`);
};

const exactRecord = (
  value: unknown,
  path: string,
  requiredKeys: ReadonlyArray<string>,
): Readonly<Record<string, unknown>> => {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    NodeUtilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) {
    return invalid(path, "must be a plain object");
  }
  const names = Reflect.ownKeys(value);
  if (
    names.length !== requiredKeys.length ||
    names.some((name) => typeof name !== "string" || !requiredKeys.includes(name))
  ) {
    return invalid(path, "has missing or extra fields");
  }
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const snapshot: Record<string, unknown> = Object.create(null);
  for (const key of requiredKeys) {
    const descriptor = descriptors[key];
    if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
      return invalid(`${path}.${key}`, "must be an enumerable data field");
    }
    snapshot[key] = descriptor.value;
  }
  return Object.freeze(snapshot);
};

const canonicalText = (value: unknown, path: string): string => {
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    return invalid(path, "must be canonical non-empty text");
  }
  return value;
};

const canonicalU64 = (value: unknown, path: string): string => {
  const text = canonicalText(value, path);
  if (text.length > 20 || !U64_PATTERN.test(text) || BigInt(text) > U64_MAX) {
    return invalid(path, "must be a canonical u64 string");
  }
  return text;
};

const safeNumber = (value: unknown, path: string): number => {
  const decimal = canonicalText(value, path);
  if (decimal.length > 16 || !U64_PATTERN.test(decimal) || BigInt(decimal) > BigInt(Number.MAX_SAFE_INTEGER)) {
    return invalid(path, "must be a non-negative safe decimal string");
  }
  return Number(decimal);
};

const stringArray = (value: unknown, path: string): ReadonlyArray<string> => {
  if (!Array.isArray(value) || NodeUtilTypes.isProxy(value) || Object.getPrototypeOf(value) !== Array.prototype) {
    return invalid(path, "must be a plain array");
  }
  const lengthDescriptor = Object.getOwnPropertyDescriptor(value, "length");
  const length = lengthDescriptor?.value;
  if (!Number.isSafeInteger(length) || (length as number) < 0) {
    return invalid(path, "has an invalid length");
  }
  const ownKeys = Reflect.ownKeys(value);
  if (
    ownKeys.length !== (length as number) + 1 ||
    !ownKeys.includes("length") ||
    ownKeys.some((key) => typeof key !== "string" || (key !== "length" && !/^(?:0|[1-9][0-9]*)$/.test(key)))
  ) {
    return invalid(path, "has sparse or extra array fields");
  }
  const result: string[] = [];
  const seen = new Set<string>();
  const descriptors = Object.getOwnPropertyDescriptors(value);
  for (let index = 0; index < (length as number); index += 1) {
    const descriptor = descriptors[String(index)];
    if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
      return invalid(`${path}[${index}]`, "must be an enumerable data item");
    }
    const item = canonicalText(descriptor.value, `${path}[${index}]`);
    if (seen.has(item)) return invalid(path, "must not contain duplicates");
    seen.add(item);
    result.push(item);
  }
  return Object.freeze(result);
};

const exactEnumArray = <T extends string>(
  value: unknown,
  path: string,
  allowed: ReadonlyArray<T>,
): ReadonlyArray<T> => {
  const entries = stringArray(value, path);
  for (const entry of entries) {
    if (!allowed.includes(entry as T)) return invalid(path, `contains unsupported value ${entry}`);
  }
  return entries as ReadonlyArray<T>;
};

function authorityGrant(value: unknown, expectedGrantRef: string): AuthorityGrant {
  const raw = exactRecord(value, "grant", [
    "binding",
    "ceiling",
    "expiresAtEpochMs",
    "grantRef",
    "issuerId",
    "parentGrant",
    "policyRevision",
    "principal",
    "revision",
    "revocationHead",
    "seatId",
  ]);
  const grantRef = canonicalText(raw.grantRef, "grant.grantRef");
  if (grantRef !== expectedGrantRef) return invalid("grant.grantRef", "does not match request");

  const parentGrant =
    raw.parentGrant === null
      ? null
      : (() => {
          const parent = exactRecord(raw.parentGrant, "grant.parentGrant", ["grantRef", "revision"]);
          return Object.freeze({
            grantRef: canonicalText(parent.grantRef, "grant.parentGrant.grantRef"),
            revision: canonicalU64(parent.revision, "grant.parentGrant.revision"),
          });
        })();

  const principalRaw = exactRecord(raw.principal, "grant.principal", [
    "domainId",
    "principalId",
    "projectId",
    "role",
    "seatId",
  ]);
  const role = canonicalText(principalRaw.role, "grant.principal.role");
  if (role !== "controller" && role !== "worker" && role !== "auditor") {
    return invalid("grant.principal.role", "is unsupported");
  }
  const seatId = canonicalText(raw.seatId, "grant.seatId");
  if (canonicalText(principalRaw.seatId, "grant.principal.seatId") !== seatId) {
    return invalid("grant.seatId", "does not match principal binding");
  }
  const principal: PolicyPrincipal = Object.freeze({
    domainId: canonicalText(principalRaw.domainId, "grant.principal.domainId"),
    principalId: canonicalText(principalRaw.principalId, "grant.principal.principalId"),
    projectId: canonicalText(principalRaw.projectId, "grant.principal.projectId"),
    role,
  });

  const bindingRaw = exactRecord(raw.binding, "grant.binding", ["executionId", "generation", "sessionId"]);
  const binding: PolicyBinding = Object.freeze({
    executionId: canonicalText(bindingRaw.executionId, "grant.binding.executionId"),
    generation: canonicalU64(bindingRaw.generation, "grant.binding.generation"),
    sessionId: canonicalText(bindingRaw.sessionId, "grant.binding.sessionId"),
  });

  const ceilingRaw = exactRecord(raw.ceiling, "grant.ceiling", [
    "allowedActions",
    "allowedContinuationResponses",
    "allowedMaterialClasses",
    "allowedSinks",
    "allowedTargetDomainIds",
    "allowedTargetPrincipalIds",
    "explicitPrivateMaterialIds",
    "maxMaterialBytes",
    "maxMaterialItems",
    "maxResponseBytes",
  ]);
  const ceiling: AuthorityCeiling = Object.freeze({
    allowedActions: exactEnumArray(ceilingRaw.allowedActions, "grant.ceiling.allowedActions", POLICY_ACTIONS),
    allowedContinuationResponses: stringArray(
      ceilingRaw.allowedContinuationResponses,
      "grant.ceiling.allowedContinuationResponses",
    ),
    allowedMaterialClasses: stringArray(ceilingRaw.allowedMaterialClasses, "grant.ceiling.allowedMaterialClasses"),
    allowedSinks: exactEnumArray(ceilingRaw.allowedSinks, "grant.ceiling.allowedSinks", MATERIAL_SINKS),
    allowedTargetDomainIds: stringArray(ceilingRaw.allowedTargetDomainIds, "grant.ceiling.allowedTargetDomainIds"),
    allowedTargetPrincipalIds: stringArray(
      ceilingRaw.allowedTargetPrincipalIds,
      "grant.ceiling.allowedTargetPrincipalIds",
    ),
    explicitPrivateMaterialIds: stringArray(
      ceilingRaw.explicitPrivateMaterialIds,
      "grant.ceiling.explicitPrivateMaterialIds",
    ),
    maxMaterialBytes: safeNumber(ceilingRaw.maxMaterialBytes, "grant.ceiling.maxMaterialBytes"),
    maxMaterialItems: safeNumber(ceilingRaw.maxMaterialItems, "grant.ceiling.maxMaterialItems"),
    maxResponseBytes: safeNumber(ceilingRaw.maxResponseBytes, "grant.ceiling.maxResponseBytes"),
  });

  return Object.freeze({
    grantRef,
    revision: canonicalU64(raw.revision, "grant.revision"),
    revocationHead: canonicalU64(raw.revocationHead, "grant.revocationHead"),
    policyRevision: canonicalU64(raw.policyRevision, "grant.policyRevision"),
    seatId,
    issuerId: canonicalText(raw.issuerId, "grant.issuerId"),
    parentGrant,
    principal,
    binding,
    expiresAtEpochMs: safeNumber(raw.expiresAtEpochMs, "grant.expiresAtEpochMs"),
    ceiling,
  });
}

export class NativePolicyAuthorityPort implements PolicyAuthorityPort {
  readonly #read: (grantRef: string) => Promise<NativeDelegationGrantSnapshot>;

  constructor(store: NativeDelegationGrantReader) {
    const read = store.readCurrentDelegationGrant;
    if (typeof read !== "function") {
      throw new PolicyError("INVALID_INPUT", "native Product Authority grant read is unavailable");
    }
    this.#read = read.bind(store);
  }

  async resolveGrant(grantRef: string): Promise<AuthorityGrant | null> {
    if (typeof grantRef !== "string" || grantRef.length === 0 || grantRef !== grantRef.trim()) {
      return invalid("grantRef", "must be canonical non-empty text");
    }
    const result: unknown = await this.#read(grantRef);
    if (result === null) return null;
    return authorityGrant(result, grantRef);
  }
}
