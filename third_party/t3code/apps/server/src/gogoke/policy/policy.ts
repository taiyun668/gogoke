import * as NodeCrypto from "node:crypto";

import { cloneAndFreezeJson } from "../runtimeCatalog/manifest.ts";
import {
  MATERIAL_SINKS,
  POLICY_ACTIONS,
  PolicyError,
  type AuthorityCeiling,
  type AuthorityGrant,
  type AuthorizedMaterialExposure,
  type AuthorizedTaskPackage,
  type ContinuationCommand,
  type ContinuationReceipt,
  type ContinuationResolution,
  type ContinuationState,
  type ContinuationStopReceipt,
  type DelegationRequest,
  type MaterialExposureRequest,
  type MaterialSink,
  type PendingContinuationQuery,
  type PolicyAction,
  type PolicyAdapters,
  type PolicyBinding,
  type PolicyPrincipal,
  type RevalidatedTaskPackage,
  type SelectedMaterial,
  type TaskMaterial,
  type TrustedContinuation,
} from "./types.ts";

const U64_PATTERN = /^(?:0|[1-9]\d*)$/;
const U64_MAX = 18_446_744_073_709_551_615n;
const textEncoder = new TextEncoder();
const SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;

interface ContinuationEntry {
  readonly record: TrustedContinuation;
  state: ContinuationState;
  receipt: ContinuationReceipt | null;
  commitFingerprint: string | null;
  stopReceipt: ContinuationStopReceipt | null;
  stopFingerprint: string | null;
}

const invalid = (detail: string): never => {
  throw new PolicyError("INVALID_INPUT", detail);
};

const snapshot = <T>(value: T, path: string): T => {
  try {
    return cloneAndFreezeJson(value as never, path) as T;
  } catch (error) {
    throw new PolicyError(
      "INVALID_INPUT",
      `${path} must be passive JSON: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
};

const exactRecord = (
  value: unknown,
  path: string,
  requiredKeys: ReadonlyArray<string>,
): Readonly<Record<string, unknown>> => {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return invalid(`${path} must be an object`);
  }
  const record = value as Readonly<Record<string, unknown>>;
  const keys = Object.keys(record);
  const extra = keys.find((key) => !requiredKeys.includes(key));
  if (extra !== undefined) return invalid(`${path}.${extra} is not allowed`);
  const missing = requiredKeys.find((key) => !keys.includes(key));
  if (missing !== undefined) return invalid(`${path}.${missing} is required`);
  return record;
};

const canonicalString = (value: unknown, path: string): string => {
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    return invalid(`${path} must be canonical and non-empty`);
  }
  return value;
};

const canonicalU64 = (value: unknown, path: string): string => {
  const candidate = canonicalString(value, path);
  if (!U64_PATTERN.test(candidate) || BigInt(candidate) > U64_MAX) {
    return invalid(`${path} must be a canonical u64 string`);
  }
  return candidate;
};

const epoch = (value: unknown, path: string): number => {
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    return invalid(`${path} must be a non-negative safe integer`);
  }
  return value as number;
};

const boundedCount = (value: unknown, path: string): number => {
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    return invalid(`${path} must be a non-negative safe integer`);
  }
  return value as number;
};

const stringArray = (value: unknown, path: string): ReadonlyArray<string> => {
  if (!Array.isArray(value)) return invalid(`${path} must be an array`);
  const seen = new Set<string>();
  return Object.freeze(
    value.map((item, index) => {
      const text = canonicalString(item, `${path}[${index}]`);
      if (seen.has(text)) return invalid(`${path} must not contain duplicates`);
      seen.add(text);
      return text;
    }),
  );
};

const enumArray = <T extends string>(
  value: unknown,
  path: string,
  allowed: ReadonlyArray<T>,
): ReadonlyArray<T> => {
  const values = stringArray(value, path);
  for (const item of values) {
    if (!allowed.includes(item as T)) return invalid(`${path} contains unsupported value ${item}`);
  }
  return values as ReadonlyArray<T>;
};

const principal = (value: unknown, path: string): PolicyPrincipal => {
  const raw = exactRecord(value, path, ["principalId", "projectId", "domainId", "role"]);
  const role = canonicalString(raw.role, `${path}.role`);
  if (role !== "controller" && role !== "worker" && role !== "auditor") {
    return invalid(`${path}.role is unsupported`);
  }
  return Object.freeze({
    principalId: canonicalString(raw.principalId, `${path}.principalId`),
    projectId: canonicalString(raw.projectId, `${path}.projectId`),
    domainId: canonicalString(raw.domainId, `${path}.domainId`),
    role,
  });
};

const binding = (value: unknown, path: string): PolicyBinding => {
  const raw = exactRecord(value, path, ["sessionId", "executionId", "generation"]);
  return Object.freeze({
    sessionId: canonicalString(raw.sessionId, `${path}.sessionId`),
    executionId: canonicalString(raw.executionId, `${path}.executionId`),
    generation: canonicalU64(raw.generation, `${path}.generation`),
  });
};

const ceiling = (value: unknown, path: string): AuthorityCeiling => {
  const raw = exactRecord(value, path, [
    "allowedActions",
    "allowedTargetPrincipalIds",
    "allowedTargetDomainIds",
    "allowedSinks",
    "allowedMaterialClasses",
    "explicitPrivateMaterialIds",
    "allowedContinuationResponses",
    "maxMaterialItems",
    "maxMaterialBytes",
    "maxResponseBytes",
  ]);
  return Object.freeze({
    allowedActions: enumArray(raw.allowedActions, `${path}.allowedActions`, POLICY_ACTIONS),
    allowedTargetPrincipalIds: stringArray(
      raw.allowedTargetPrincipalIds,
      `${path}.allowedTargetPrincipalIds`,
    ),
    allowedTargetDomainIds: stringArray(
      raw.allowedTargetDomainIds,
      `${path}.allowedTargetDomainIds`,
    ),
    allowedSinks: enumArray(raw.allowedSinks, `${path}.allowedSinks`, MATERIAL_SINKS),
    allowedMaterialClasses: stringArray(
      raw.allowedMaterialClasses,
      `${path}.allowedMaterialClasses`,
    ),
    explicitPrivateMaterialIds: stringArray(
      raw.explicitPrivateMaterialIds,
      `${path}.explicitPrivateMaterialIds`,
    ),
    allowedContinuationResponses: stringArray(
      raw.allowedContinuationResponses,
      `${path}.allowedContinuationResponses`,
    ),
    maxMaterialItems: boundedCount(raw.maxMaterialItems, `${path}.maxMaterialItems`),
    maxMaterialBytes: boundedCount(raw.maxMaterialBytes, `${path}.maxMaterialBytes`),
    maxResponseBytes: boundedCount(raw.maxResponseBytes, `${path}.maxResponseBytes`),
  });
};

const grant = (value: unknown, path: string): AuthorityGrant => {
  const raw = exactRecord(value, path, [
    "grantRef",
    "revision",
    "revocationHead",
    "policyRevision",
    "seatId",
    "issuerId",
    "parentGrant",
    "principal",
    "binding",
    "expiresAtEpochMs",
    "ceiling",
  ]);
  const parentGrant =
    raw.parentGrant === null
      ? null
      : (() => {
          const parent = exactRecord(raw.parentGrant, `${path}.parentGrant`, [
            "grantRef",
            "revision",
          ]);
          return Object.freeze({
            grantRef: canonicalString(parent.grantRef, `${path}.parentGrant.grantRef`),
            revision: canonicalU64(parent.revision, `${path}.parentGrant.revision`),
          });
        })();
  return Object.freeze({
    grantRef: canonicalString(raw.grantRef, `${path}.grantRef`),
    revision: canonicalU64(raw.revision, `${path}.revision`),
    revocationHead: canonicalU64(raw.revocationHead, `${path}.revocationHead`),
    policyRevision: canonicalU64(raw.policyRevision, `${path}.policyRevision`),
    seatId: canonicalString(raw.seatId, `${path}.seatId`),
    issuerId: canonicalString(raw.issuerId, `${path}.issuerId`),
    parentGrant,
    principal: principal(raw.principal, `${path}.principal`),
    binding: binding(raw.binding, `${path}.binding`),
    expiresAtEpochMs: epoch(raw.expiresAtEpochMs, `${path}.expiresAtEpochMs`),
    ceiling: ceiling(raw.ceiling, `${path}.ceiling`),
  });
};

const material = (value: unknown, path: string): TaskMaterial => {
  const raw = exactRecord(value, path, [
    "materialId",
    "projectId",
    "domainId",
    "ownerPrincipalId",
    "materialClass",
    "visibility",
    "content",
  ]);
  const visibility = canonicalString(raw.visibility, `${path}.visibility`);
  if (visibility !== "project" && visibility !== "private") {
    return invalid(`${path}.visibility is unsupported`);
  }
  return Object.freeze({
    materialId: canonicalString(raw.materialId, `${path}.materialId`),
    projectId: canonicalString(raw.projectId, `${path}.projectId`),
    domainId: canonicalString(raw.domainId, `${path}.domainId`),
    ownerPrincipalId: canonicalString(raw.ownerPrincipalId, `${path}.ownerPrincipalId`),
    materialClass: canonicalString(raw.materialClass, `${path}.materialClass`),
    visibility,
    content:
      typeof raw.content === "string" ? raw.content : invalid(`${path}.content must be text`),
  });
};

const continuation = (value: unknown, path: string): TrustedContinuation => {
  const raw = exactRecord(value, path, [
    "continuationId",
    "nativeRequestId",
    "requestedAction",
    "principal",
    "binding",
    "expiresAtEpochMs",
    "ceiling",
  ]);
  const rawCeiling = exactRecord(raw.ceiling, `${path}.ceiling`, [
    "allowedResponses",
    "maxResponseBytes",
  ]);
  return Object.freeze({
    continuationId: canonicalString(raw.continuationId, `${path}.continuationId`),
    nativeRequestId: canonicalString(raw.nativeRequestId, `${path}.nativeRequestId`),
    requestedAction: canonicalString(raw.requestedAction, `${path}.requestedAction`),
    principal: principal(raw.principal, `${path}.principal`),
    binding: binding(raw.binding, `${path}.binding`),
    expiresAtEpochMs: epoch(raw.expiresAtEpochMs, `${path}.expiresAtEpochMs`),
    ceiling: Object.freeze({
      allowedResponses: stringArray(
        rawCeiling.allowedResponses,
        `${path}.ceiling.allowedResponses`,
      ),
      maxResponseBytes: boundedCount(
        rawCeiling.maxResponseBytes,
        `${path}.ceiling.maxResponseBytes`,
      ),
    }),
  });
};

const samePrincipal = (left: PolicyPrincipal, right: PolicyPrincipal): boolean =>
  left.principalId === right.principalId &&
  left.projectId === right.projectId &&
  left.domainId === right.domainId &&
  left.role === right.role;

const sameBinding = (left: PolicyBinding, right: PolicyBinding): boolean =>
  left.sessionId === right.sessionId &&
  left.executionId === right.executionId &&
  left.generation === right.generation;

const sameStrings = (left: ReadonlyArray<string>, right: ReadonlyArray<string>): boolean =>
  left.length === right.length && left.every((value, index) => value === right[index]);

const sameContinuation = (left: TrustedContinuation, right: TrustedContinuation): boolean =>
  left.continuationId === right.continuationId &&
  left.nativeRequestId === right.nativeRequestId &&
  left.requestedAction === right.requestedAction &&
  samePrincipal(left.principal, right.principal) &&
  sameBinding(left.binding, right.binding) &&
  left.expiresAtEpochMs === right.expiresAtEpochMs &&
  left.ceiling.maxResponseBytes === right.ceiling.maxResponseBytes &&
  sameStrings(left.ceiling.allowedResponses, right.ceiling.allowedResponses);

const selectedIds = (value: unknown, path: string): ReadonlyArray<string> =>
  stringArray(value, path);

const canonicalJson = (value: unknown): string => {
  if (value === null || typeof value === "boolean" || typeof value === "number") {
    return JSON.stringify(value);
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  const record = value as Readonly<Record<string, unknown>>;
  return `{${Object.keys(record)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${canonicalJson(record[key])}`)
    .join(",")}}`;
};

const digest = (value: unknown): string =>
  `sha256:${NodeCrypto.createHash("sha256").update(canonicalJson(value), "utf8").digest("hex")}`;

const isSubset = (child: ReadonlyArray<string>, parent: ReadonlyArray<string>): boolean =>
  child.every((value) => parent.includes(value));

const ceilingSubset = (child: AuthorityCeiling, parent: AuthorityCeiling): boolean =>
  isSubset(child.allowedActions, parent.allowedActions) &&
  isSubset(child.allowedTargetPrincipalIds, parent.allowedTargetPrincipalIds) &&
  isSubset(child.allowedTargetDomainIds, parent.allowedTargetDomainIds) &&
  isSubset(child.allowedSinks, parent.allowedSinks) &&
  isSubset(child.allowedMaterialClasses, parent.allowedMaterialClasses) &&
  isSubset(child.explicitPrivateMaterialIds, parent.explicitPrivateMaterialIds) &&
  isSubset(child.allowedContinuationResponses, parent.allowedContinuationResponses) &&
  child.maxMaterialItems <= parent.maxMaterialItems &&
  child.maxMaterialBytes <= parent.maxMaterialBytes &&
  child.maxResponseBytes <= parent.maxResponseBytes;

const intersectCeiling = (authority: AuthorityCeiling, local: AuthorityCeiling): AuthorityCeiling =>
  Object.freeze({
    allowedActions: Object.freeze(
      authority.allowedActions.filter((value) => local.allowedActions.includes(value)),
    ),
    allowedTargetPrincipalIds: Object.freeze(
      authority.allowedTargetPrincipalIds.filter((value) =>
        local.allowedTargetPrincipalIds.includes(value),
      ),
    ),
    allowedTargetDomainIds: Object.freeze(
      authority.allowedTargetDomainIds.filter((value) =>
        local.allowedTargetDomainIds.includes(value),
      ),
    ),
    allowedSinks: Object.freeze(
      authority.allowedSinks.filter((value) => local.allowedSinks.includes(value)),
    ),
    allowedMaterialClasses: Object.freeze(
      authority.allowedMaterialClasses.filter((value) =>
        local.allowedMaterialClasses.includes(value),
      ),
    ),
    explicitPrivateMaterialIds: Object.freeze(
      authority.explicitPrivateMaterialIds.filter((value) =>
        local.explicitPrivateMaterialIds.includes(value),
      ),
    ),
    allowedContinuationResponses: Object.freeze(
      authority.allowedContinuationResponses.filter((value) =>
        local.allowedContinuationResponses.includes(value),
      ),
    ),
    maxMaterialItems: Math.min(authority.maxMaterialItems, local.maxMaterialItems),
    maxMaterialBytes: Math.min(authority.maxMaterialBytes, local.maxMaterialBytes),
    maxResponseBytes: Math.min(authority.maxResponseBytes, local.maxResponseBytes),
  });

const packagePreimage = (value: Omit<AuthorizedTaskPackage, "packageDigest">): unknown => value;

const packageMaterialsWithinCeiling = (
  materials: ReadonlyArray<SelectedMaterial>,
  authorityCeiling: AuthorityCeiling,
): boolean => {
  if (materials.length > authorityCeiling.maxMaterialItems) return false;
  let total = 0;
  for (const item of materials) {
    if (!authorityCeiling.allowedMaterialClasses.includes(item.materialClass)) return false;
    if (
      item.visibility === "private" &&
      !authorityCeiling.explicitPrivateMaterialIds.includes(item.materialId)
    )
      return false;
    total += textEncoder.encode(item.content).byteLength;
  }
  return total <= authorityCeiling.maxMaterialBytes;
};

export class PolicyService {
  readonly #resolveGrant: PolicyAdapters["authority"]["resolveGrant"];
  readonly #resolveContinuation: PolicyAdapters["continuations"]["resolveContinuation"];
  readonly #resolveMaterial: PolicyAdapters["materials"]["resolveMaterial"];
  readonly #continuations = new Map<string, ContinuationEntry>();

  constructor(adapters: PolicyAdapters) {
    this.#resolveGrant = adapters.authority.resolveGrant.bind(adapters.authority);
    this.#resolveContinuation = adapters.continuations.resolveContinuation.bind(
      adapters.continuations,
    );
    this.#resolveMaterial = adapters.materials.resolveMaterial.bind(adapters.materials);
  }

  async prepareDelegation(
    input: DelegationRequest,
    nowEpochMs: number,
  ): Promise<AuthorizedTaskPackage> {
    const request = this.#delegationRequest(snapshot(input, "request"));
    const resolvedGrant = await this.#authorize(
      request.grantRef,
      request.source,
      request.sourceBinding,
      request.action,
      request.target.principalId,
      request.target.domainId,
      request.sink,
      nowEpochMs,
    );
    if (request.source.projectId !== request.target.projectId) {
      throw new PolicyError("TARGET_NOT_ALLOWED", "delegation cannot cross project identity");
    }
    if (
      request.action === "delegate" &&
      (request.route !== "controller-worker" ||
        request.source.role !== "controller" ||
        request.target.role !== "worker" ||
        request.sink !== "task-package")
    ) {
      throw new PolicyError("TARGET_NOT_ALLOWED", "delegation requires controller-worker route");
    }
    if (
      request.action === "return-result" &&
      (request.route !== "worker-controller" ||
        request.source.role !== "worker" ||
        request.target.role !== "controller" ||
        request.sink !== "task-package")
    ) {
      throw new PolicyError(
        "TARGET_NOT_ALLOWED",
        "worker result must return through the worker-controller route",
      );
    }
    if (
      request.action === "request-review" &&
      (request.route !== "controller-clean-review" ||
        request.source.role !== "controller" ||
        request.target.role !== "auditor" ||
        request.targetBindingKind !== "new-clean" ||
        request.sink !== "formal-review")
    ) {
      throw new PolicyError(
        "CLEAN_REVIEW_REQUIRED",
        "formal review requires a new clean controller-auditor binding",
      );
    }
    const materials = this.#selectMaterials(
      request.selectedMaterialIds,
      request.source,
      request.sink,
      resolvedGrant.ceiling,
    );
    if (
      request.action === "request-review" &&
      materials.some((entry) => entry.visibility === "private")
    ) {
      throw new PolicyError(
        "CLEAN_REVIEW_REQUIRED",
        "formal review cannot inherit private conversation material",
      );
    }
    if (!ceilingSubset(request.childCeiling, resolvedGrant.ceiling)) {
      throw new PolicyError("AUTHORITY_MISMATCH", "child ceiling exceeds the parent grant");
    }
    if (!packageMaterialsWithinCeiling(materials, request.childCeiling)) {
      throw new PolicyError("MATERIAL_CEILING_EXCEEDED", "task package exceeds the child ceiling");
    }
    const instructionDigest = digest(request.instruction);
    const materialSetDigest = digest(materials.map(({ content: _content, ...item }) => item));
    const unsigned = Object.freeze({
      parentGrantRef: resolvedGrant.grantRef,
      parentGrantRevision: resolvedGrant.revision,
      parentGrantRevocationHead: resolvedGrant.revocationHead,
      parentPolicyRevision: resolvedGrant.policyRevision,
      parentSeatId: resolvedGrant.seatId,
      parentGrantDigest: digest(resolvedGrant),
      parentCeilingDigest: digest(resolvedGrant.ceiling),
      childCeiling: request.childCeiling,
      childCeilingDigest: digest(request.childCeiling),
      action: request.action,
      route: request.route,
      source: request.source,
      target: request.target,
      sourceBinding: request.sourceBinding,
      targetBinding: request.targetBinding,
      targetBindingKind: request.targetBindingKind,
      sink: request.sink,
      instruction: request.instruction,
      instructionDigest,
      materialSetDigest,
      materials,
    }) satisfies Omit<AuthorizedTaskPackage, "packageDigest">;
    return Object.freeze({ packageDigest: digest(packagePreimage(unsigned)), ...unsigned });
  }

  async revalidateTaskPackage(
    input: AuthorizedTaskPackage,
    nowEpochMs: number,
    runtimeCeiling?: AuthorityCeiling,
  ): Promise<RevalidatedTaskPackage> {
    const taskPackage = this.#taskPackage(snapshot(input, "taskPackage"));
    const current = await this.#authorize(
      taskPackage.parentGrantRef,
      taskPackage.source,
      taskPackage.sourceBinding,
      taskPackage.action,
      taskPackage.target.principalId,
      taskPackage.target.domainId,
      taskPackage.sink,
      nowEpochMs,
    );
    if (
      current.revision !== taskPackage.parentGrantRevision ||
      current.revocationHead !== taskPackage.parentGrantRevocationHead ||
      current.policyRevision !== taskPackage.parentPolicyRevision ||
      current.seatId !== taskPackage.parentSeatId ||
      digest(current) !== taskPackage.parentGrantDigest ||
      digest(current.ceiling) !== taskPackage.parentCeilingDigest ||
      !ceilingSubset(taskPackage.childCeiling, current.ceiling)
    ) {
      throw new PolicyError("AUTHORITY_MISMATCH", "parent grant revision or ceiling changed");
    }
    if (taskPackage.source.projectId !== taskPackage.target.projectId) {
      throw new PolicyError("TARGET_NOT_ALLOWED", "task package cannot cross project identity");
    }
    if (
      (taskPackage.action === "delegate" &&
        (taskPackage.route !== "controller-worker" ||
          taskPackage.source.role !== "controller" ||
          taskPackage.target.role !== "worker" ||
          taskPackage.sink !== "task-package")) ||
      (taskPackage.action === "return-result" &&
        (taskPackage.route !== "worker-controller" ||
          taskPackage.source.role !== "worker" ||
          taskPackage.target.role !== "controller" ||
          taskPackage.sink !== "task-package")) ||
      (taskPackage.action === "request-review" &&
        (taskPackage.route !== "controller-clean-review" ||
          taskPackage.source.role !== "controller" ||
          taskPackage.target.role !== "auditor" ||
          taskPackage.targetBindingKind !== "new-clean" ||
          taskPackage.sink !== "formal-review" ||
          taskPackage.materials.some((item) => item.visibility === "private")))
    ) {
      throw new PolicyError("AUTHORITY_MISMATCH", "task package route or role binding is invalid");
    }
    if (!packageMaterialsWithinCeiling(taskPackage.materials, taskPackage.childCeiling)) {
      throw new PolicyError(
        "MATERIAL_CEILING_EXCEEDED",
        "task package exceeds the current child ceiling",
      );
    }
    const local =
      runtimeCeiling === undefined
        ? taskPackage.childCeiling
        : ceiling(snapshot(runtimeCeiling, "runtimeCeiling"), "runtimeCeiling");
    return Object.freeze({
      package: taskPackage,
      effectiveCeiling: intersectCeiling(taskPackage.childCeiling, local),
      revalidatedAtEpochMs: epoch(nowEpochMs, "nowEpochMs"),
    });
  }

  async prepareMaterialExposure(
    input: MaterialExposureRequest,
    nowEpochMs: number,
  ): Promise<AuthorizedMaterialExposure> {
    const request = this.#exposureRequest(snapshot(input, "request"));
    const resolvedGrant = await this.#authorize(
      request.grantRef,
      request.source,
      request.sourceBinding,
      "share-material",
      request.targetPrincipalId,
      request.targetDomainId,
      request.sink,
      nowEpochMs,
    );
    const materials = this.#selectMaterials(
      request.selectedMaterialIds,
      request.source,
      request.sink,
      resolvedGrant.ceiling,
    );
    if (
      request.sink === "formal-review" &&
      materials.some((item) => item.visibility === "private")
    ) {
      throw new PolicyError(
        "CLEAN_REVIEW_REQUIRED",
        "formal review cannot inherit private conversation material",
      );
    }
    return Object.freeze({
      grantRef: resolvedGrant.grantRef,
      grantRevision: resolvedGrant.revision,
      sourcePrincipalId: request.source.principalId,
      targetPrincipalId: request.targetPrincipalId,
      sourceDomainId: request.source.domainId,
      targetDomainId: request.targetDomainId,
      sink: request.sink,
      materials,
    });
  }

  trackContinuation(nativeRequestId: string): TrustedContinuation {
    const requestId = canonicalString(nativeRequestId, "nativeRequestId");
    const resolved = this.#resolveContinuation(requestId);
    if (resolved === null) {
      throw new PolicyError("CONTINUATION_NOT_FOUND", "native continuation is not trusted");
    }
    const record = continuation(snapshot(resolved, "continuation"), "continuation");
    if (record.nativeRequestId !== requestId) {
      throw new PolicyError("CONTINUATION_MISMATCH", "native request identity changed");
    }
    const existing = this.#continuations.get(record.continuationId);
    if (existing !== undefined) {
      if (JSON.stringify(existing.record) !== JSON.stringify(record)) {
        throw new PolicyError("CONTINUATION_MISMATCH", "continuation identity was rebound");
      }
      return existing.record;
    }
    this.#continuations.set(record.continuationId, {
      record,
      state: "pending",
      receipt: null,
      commitFingerprint: null,
      stopReceipt: null,
      stopFingerprint: null,
    });
    return record;
  }

  async resolveContinuation(
    input: ContinuationCommand,
    nowEpochMs: number,
  ): Promise<ContinuationResolution> {
    const command = this.#continuationCommand(snapshot(input, "command"));
    const now = epoch(nowEpochMs, "nowEpochMs");
    const entry = this.#continuations.get(command.continuationId);
    if (entry === undefined) {
      throw new PolicyError("CONTINUATION_NOT_FOUND", "continuation is not tracked");
    }
    const record = this.#requireLiveContinuation(entry);
    if (now >= record.expiresAtEpochMs) {
      if (entry.state === "pending") entry.state = "expired";
      throw new PolicyError("CONTINUATION_EXPIRED", "continuation expired before commitment");
    }
    if (
      command.nativeRequestId !== record.nativeRequestId ||
      command.requestedAction !== record.requestedAction ||
      !samePrincipal(command.principal, record.principal) ||
      !sameBinding(command.binding, record.binding)
    ) {
      throw new PolicyError(
        "CONTINUATION_MISMATCH",
        "principal, domain, action, session, execution, or generation does not match",
      );
    }
    const action: PolicyAction =
      command.kind === "answer" ? "answer-continuation" : "cancel-continuation";
    const resolvedGrant = await this.#authorize(
      command.grantRef,
      command.principal,
      command.binding,
      action,
      command.principal.principalId,
      command.principal.domainId,
      "controller",
      now,
    );
    if (command.kind === "answer") {
      if (
        !record.ceiling.allowedResponses.includes(command.responseKind) ||
        !resolvedGrant.ceiling.allowedContinuationResponses.includes(command.responseKind)
      ) {
        throw new PolicyError("RESPONSE_NOT_ALLOWED", "response kind exceeds the fixed ceiling");
      }
      const bytes = textEncoder.encode(command.content).byteLength;
      if (
        bytes > record.ceiling.maxResponseBytes ||
        bytes > resolvedGrant.ceiling.maxResponseBytes
      ) {
        throw new PolicyError("RESPONSE_CEILING_EXCEEDED", "response exceeds byte ceiling");
      }
    }
    const fingerprint = this.#continuationFingerprint(command);
    if (entry.receipt !== null) {
      if (command.operationId === entry.receipt.operationId) {
        if (entry.commitFingerprint !== fingerprint) {
          throw new PolicyError(
            "OPERATION_CONFLICT",
            "the committed operation id was reused with different content",
          );
        }
        return entry.receipt;
      }
      if (entry.state === "answered" && command.kind === "cancel") {
        if (entry.stopReceipt !== null) {
          if (
            entry.stopReceipt.operationId !== command.operationId ||
            entry.stopFingerprint !== fingerprint
          ) {
            throw new PolicyError(
              "CONTINUATION_TERMINAL",
              "a distinct stop is already pending verification",
            );
          }
          return entry.stopReceipt;
        }
        const stopReceipt = Object.freeze({
          kind: "stop-pending-verification",
          operationId: command.operationId,
          continuationId: record.continuationId,
          nativeRequestId: record.nativeRequestId,
          originalCommitOperationId: entry.receipt.operationId,
          principalId: record.principal.principalId,
          domainId: record.principal.domainId,
          bindingGeneration: record.binding.generation,
          state: "pending-verification",
          requestedAtEpochMs: now,
        }) satisfies ContinuationStopReceipt;
        entry.stopReceipt = stopReceipt;
        entry.stopFingerprint = fingerprint;
        return stopReceipt;
      }
      throw new PolicyError("CONTINUATION_TERMINAL", `continuation is ${entry.state}`);
    }
    if (entry.state !== "pending") {
      throw new PolicyError("CONTINUATION_TERMINAL", `continuation is ${entry.state}`);
    }
    // This synchronous state transition is the single serialization point for answer and cancel.
    entry.state = command.kind === "answer" ? "answered" : "cancelled";
    const receipt = Object.freeze({
      kind: "continuation-commit",
      operationId: command.operationId,
      continuationId: record.continuationId,
      nativeRequestId: record.nativeRequestId,
      requestedAction: record.requestedAction,
      principalId: record.principal.principalId,
      domainId: record.principal.domainId,
      bindingGeneration: record.binding.generation,
      state: entry.state,
      responseKind: command.kind === "answer" ? command.responseKind : null,
      content: command.kind === "answer" ? command.content : null,
      committedAtEpochMs: now,
    }) satisfies ContinuationReceipt;
    entry.receipt = receipt;
    entry.commitFingerprint = fingerprint;
    return receipt;
  }

  pendingContinuations(
    input: PendingContinuationQuery,
    nowEpochMs: number,
  ): ReadonlyArray<TrustedContinuation> {
    const query = this.#pendingQuery(snapshot(input, "query"));
    const now = epoch(nowEpochMs, "nowEpochMs");
    const pending: TrustedContinuation[] = [];
    for (const entry of this.#continuations.values()) {
      if (!this.#isLiveContinuation(entry)) continue;
      if (entry.state === "pending" && now >= entry.record.expiresAtEpochMs) {
        entry.state = "expired";
      }
      if (
        entry.state === "pending" &&
        samePrincipal(query.principal, entry.record.principal) &&
        sameBinding(query.binding, entry.record.binding)
      ) {
        pending.push(entry.record);
      }
    }
    return Object.freeze(pending);
  }

  #isLiveContinuation(entry: ContinuationEntry): boolean {
    const resolved = this.#resolveContinuation(entry.record.nativeRequestId);
    if (resolved === null) return false;
    try {
      const current = continuation(snapshot(resolved, "continuation"), "continuation");
      return sameContinuation(entry.record, current);
    } catch {
      return false;
    }
  }

  #requireLiveContinuation(entry: ContinuationEntry): TrustedContinuation {
    const resolved = this.#resolveContinuation(entry.record.nativeRequestId);
    if (resolved === null) {
      throw new PolicyError(
        "CONTINUATION_NOT_FOUND",
        "native continuation no longer exists; local tracking is not authority",
      );
    }
    const current = continuation(snapshot(resolved, "continuation"), "continuation");
    if (!sameContinuation(entry.record, current)) {
      throw new PolicyError("CONTINUATION_MISMATCH", "native continuation authority changed");
    }
    return entry.record;
  }

  async #authorize(
    grantRef: string,
    requestPrincipal: PolicyPrincipal,
    requestBinding: PolicyBinding,
    action: PolicyAction,
    targetPrincipalId: string,
    targetDomainId: string,
    sink: MaterialSink,
    nowEpochMs: number,
  ): Promise<AuthorityGrant> {
    const now = epoch(nowEpochMs, "nowEpochMs");
    const resolved = await this.#resolveGrant(grantRef);
    if (resolved === null) throw new PolicyError("AUTHORITY_REQUIRED", "grant was not resolved");
    const trusted = grant(snapshot(resolved, "grant"), "grant");
    if (trusted.grantRef !== grantRef) {
      throw new PolicyError("AUTHORITY_MISMATCH", "grant reference changed during resolution");
    }
    if (
      !samePrincipal(trusted.principal, requestPrincipal) ||
      !sameBinding(trusted.binding, requestBinding)
    ) {
      throw new PolicyError(
        "AUTHORITY_MISMATCH",
        "principal, project, domain, session, execution, or generation does not match grant",
      );
    }
    if (now >= trusted.expiresAtEpochMs) {
      throw new PolicyError("AUTHORITY_EXPIRED", "grant has expired");
    }
    if (!trusted.ceiling.allowedActions.includes(action)) {
      throw new PolicyError("ACTION_NOT_ALLOWED", `${action} exceeds grant ceiling`);
    }
    if (
      !trusted.ceiling.allowedTargetPrincipalIds.includes(targetPrincipalId) ||
      !trusted.ceiling.allowedTargetDomainIds.includes(targetDomainId)
    ) {
      throw new PolicyError(
        "TARGET_NOT_ALLOWED",
        "target principal or domain exceeds grant ceiling",
      );
    }
    if (!trusted.ceiling.allowedSinks.includes(sink)) {
      throw new PolicyError("SINK_NOT_ALLOWED", `${sink} exceeds grant ceiling`);
    }
    return trusted;
  }

  #selectMaterials(
    ids: ReadonlyArray<string>,
    source: PolicyPrincipal,
    sink: MaterialSink,
    authorityCeiling: AuthorityCeiling,
  ): ReadonlyArray<SelectedMaterial> {
    if (ids.length > authorityCeiling.maxMaterialItems) {
      throw new PolicyError("MATERIAL_CEILING_EXCEEDED", "material item ceiling exceeded");
    }
    let totalBytes = 0;
    const selected: SelectedMaterial[] = [];
    for (const materialId of ids) {
      const resolved = this.#resolveMaterial(materialId);
      if (resolved === null) {
        throw new PolicyError("MATERIAL_NOT_FOUND", `material ${materialId} was not resolved`);
      }
      const trusted = material(
        snapshot(resolved, `material(${materialId})`),
        `material(${materialId})`,
      );
      if (trusted.materialId !== materialId) {
        throw new PolicyError(
          "MATERIAL_NOT_ALLOWED",
          "material identity changed during resolution",
        );
      }
      if (trusted.projectId !== source.projectId) {
        throw new PolicyError("MATERIAL_NOT_ALLOWED", "material belongs to another project");
      }
      if (!authorityCeiling.allowedMaterialClasses.includes(trusted.materialClass)) {
        throw new PolicyError("MATERIAL_NOT_ALLOWED", "material class exceeds grant ceiling");
      }
      if (
        trusted.visibility === "private" &&
        !authorityCeiling.explicitPrivateMaterialIds.includes(trusted.materialId)
      ) {
        throw new PolicyError(
          "PRIVATE_MATERIAL_NOT_EXPLICIT",
          `private material cannot flow automatically to ${sink}`,
        );
      }
      totalBytes += textEncoder.encode(trusted.content).byteLength;
      if (totalBytes > authorityCeiling.maxMaterialBytes) {
        throw new PolicyError("MATERIAL_CEILING_EXCEEDED", "material byte ceiling exceeded");
      }
      selected.push(
        Object.freeze({
          materialId: trusted.materialId,
          materialClass: trusted.materialClass,
          visibility: trusted.visibility,
          content: trusted.content,
          contentDigest: digest(trusted.content),
        }),
      );
    }
    return Object.freeze(selected);
  }

  #delegationRequest(value: unknown): DelegationRequest {
    const raw = exactRecord(value, "request", [
      "grantRef",
      "action",
      "route",
      "source",
      "target",
      "sourceBinding",
      "targetBinding",
      "targetBindingKind",
      "sink",
      "selectedMaterialIds",
      "childCeiling",
      "instruction",
    ]);
    const action = canonicalString(raw.action, "request.action");
    if (action !== "delegate" && action !== "return-result" && action !== "request-review") {
      return invalid("request.action is unsupported");
    }
    const route = canonicalString(raw.route, "request.route");
    if (
      route !== "controller-worker" &&
      route !== "worker-controller" &&
      route !== "controller-clean-review"
    ) {
      return invalid("request.route is unsupported");
    }
    const targetBindingKind = canonicalString(raw.targetBindingKind, "request.targetBindingKind");
    if (targetBindingKind !== "existing" && targetBindingKind !== "new-clean") {
      return invalid("request.targetBindingKind is unsupported");
    }
    const sink = canonicalString(raw.sink, "request.sink");
    if (sink !== "task-package" && sink !== "formal-review") {
      return invalid("request.sink is unsupported");
    }
    if (typeof raw.instruction !== "string") return invalid("request.instruction must be text");
    return Object.freeze({
      grantRef: canonicalString(raw.grantRef, "request.grantRef"),
      action,
      route,
      source: principal(raw.source, "request.source"),
      target: principal(raw.target, "request.target"),
      sourceBinding: binding(raw.sourceBinding, "request.sourceBinding"),
      targetBinding: binding(raw.targetBinding, "request.targetBinding"),
      targetBindingKind,
      sink,
      selectedMaterialIds: selectedIds(raw.selectedMaterialIds, "request.selectedMaterialIds"),
      childCeiling: ceiling(raw.childCeiling, "request.childCeiling"),
      instruction: raw.instruction,
    });
  }

  #taskPackage(value: unknown): AuthorizedTaskPackage {
    const raw = exactRecord(value, "taskPackage", [
      "packageDigest",
      "parentGrantRef",
      "parentGrantRevision",
      "parentGrantRevocationHead",
      "parentPolicyRevision",
      "parentSeatId",
      "parentGrantDigest",
      "parentCeilingDigest",
      "childCeiling",
      "childCeilingDigest",
      "action",
      "route",
      "source",
      "target",
      "sourceBinding",
      "targetBinding",
      "targetBindingKind",
      "sink",
      "instruction",
      "instructionDigest",
      "materialSetDigest",
      "materials",
    ]);
    const action = canonicalString(raw.action, "taskPackage.action");
    if (action !== "delegate" && action !== "return-result" && action !== "request-review") {
      return invalid("taskPackage.action is unsupported");
    }
    const route = canonicalString(raw.route, "taskPackage.route");
    if (
      route !== "controller-worker" &&
      route !== "worker-controller" &&
      route !== "controller-clean-review"
    ) {
      return invalid("taskPackage.route is unsupported");
    }
    const targetBindingKind = canonicalString(
      raw.targetBindingKind,
      "taskPackage.targetBindingKind",
    );
    if (targetBindingKind !== "existing" && targetBindingKind !== "new-clean") {
      return invalid("taskPackage.targetBindingKind is unsupported");
    }
    const sink = canonicalString(raw.sink, "taskPackage.sink");
    if (sink !== "task-package" && sink !== "formal-review") {
      return invalid("taskPackage.sink is unsupported");
    }
    if (typeof raw.instruction !== "string") return invalid("taskPackage.instruction must be text");
    if (!Array.isArray(raw.materials)) return invalid("taskPackage.materials must be an array");
    const materials = Object.freeze(
      raw.materials.map((entry, index) => {
        const materialRaw = exactRecord(entry, `taskPackage.materials[${index}]`, [
          "materialId",
          "materialClass",
          "visibility",
          "content",
          "contentDigest",
        ]);
        const visibility = canonicalString(
          materialRaw.visibility,
          `taskPackage.materials[${index}].visibility`,
        );
        if (visibility !== "project" && visibility !== "private")
          return invalid("taskPackage material visibility is unsupported");
        const content =
          typeof materialRaw.content === "string"
            ? materialRaw.content
            : invalid(`taskPackage.materials[${index}].content must be text`);
        const contentDigest = canonicalString(
          materialRaw.contentDigest,
          `taskPackage.materials[${index}].contentDigest`,
        );
        if (!SHA256_PATTERN.test(contentDigest) || contentDigest !== digest(content)) {
          return invalid(`taskPackage.materials[${index}].contentDigest is invalid`);
        }
        return Object.freeze({
          materialId: canonicalString(
            materialRaw.materialId,
            `taskPackage.materials[${index}].materialId`,
          ),
          materialClass: canonicalString(
            materialRaw.materialClass,
            `taskPackage.materials[${index}].materialClass`,
          ),
          visibility,
          content,
          contentDigest,
        });
      }),
    );
    const childCeiling = ceiling(raw.childCeiling, "taskPackage.childCeiling");
    const unsigned = Object.freeze({
      parentGrantRef: canonicalString(raw.parentGrantRef, "taskPackage.parentGrantRef"),
      parentGrantRevision: canonicalU64(raw.parentGrantRevision, "taskPackage.parentGrantRevision"),
      parentGrantRevocationHead: canonicalU64(
        raw.parentGrantRevocationHead,
        "taskPackage.parentGrantRevocationHead",
      ),
      parentPolicyRevision: canonicalU64(
        raw.parentPolicyRevision,
        "taskPackage.parentPolicyRevision",
      ),
      parentSeatId: canonicalString(raw.parentSeatId, "taskPackage.parentSeatId"),
      parentGrantDigest: canonicalString(raw.parentGrantDigest, "taskPackage.parentGrantDigest"),
      parentCeilingDigest: canonicalString(
        raw.parentCeilingDigest,
        "taskPackage.parentCeilingDigest",
      ),
      childCeiling,
      childCeilingDigest: canonicalString(raw.childCeilingDigest, "taskPackage.childCeilingDigest"),
      action,
      route,
      source: principal(raw.source, "taskPackage.source"),
      target: principal(raw.target, "taskPackage.target"),
      sourceBinding: binding(raw.sourceBinding, "taskPackage.sourceBinding"),
      targetBinding: binding(raw.targetBinding, "taskPackage.targetBinding"),
      targetBindingKind,
      sink,
      instruction: raw.instruction,
      instructionDigest: canonicalString(raw.instructionDigest, "taskPackage.instructionDigest"),
      materialSetDigest: canonicalString(raw.materialSetDigest, "taskPackage.materialSetDigest"),
      materials,
    }) satisfies Omit<AuthorizedTaskPackage, "packageDigest">;
    for (const [path, candidate] of [
      ["parentGrantDigest", unsigned.parentGrantDigest],
      ["parentCeilingDigest", unsigned.parentCeilingDigest],
      ["childCeilingDigest", unsigned.childCeilingDigest],
      ["instructionDigest", unsigned.instructionDigest],
      ["materialSetDigest", unsigned.materialSetDigest],
    ] as const) {
      if (!SHA256_PATTERN.test(candidate)) return invalid(`taskPackage.${path} is invalid`);
    }
    if (
      unsigned.childCeilingDigest !== digest(unsigned.childCeiling) ||
      unsigned.instructionDigest !== digest(unsigned.instruction) ||
      unsigned.materialSetDigest !==
        digest(unsigned.materials.map(({ content: _content, ...item }) => item))
    ) {
      return invalid("taskPackage bound digest does not match content");
    }
    const packageDigest = canonicalString(raw.packageDigest, "taskPackage.packageDigest");
    if (
      !SHA256_PATTERN.test(packageDigest) ||
      packageDigest !== digest(packagePreimage(unsigned))
    ) {
      return invalid("taskPackage.packageDigest does not match canonical package");
    }
    return Object.freeze({ packageDigest, ...unsigned });
  }

  #exposureRequest(value: unknown): MaterialExposureRequest {
    const raw = exactRecord(value, "request", [
      "grantRef",
      "source",
      "sourceBinding",
      "targetPrincipalId",
      "targetDomainId",
      "sink",
      "selectedMaterialIds",
    ]);
    const sink = canonicalString(raw.sink, "request.sink");
    if (!MATERIAL_SINKS.includes(sink as MaterialSink))
      return invalid("request.sink is unsupported");
    return Object.freeze({
      grantRef: canonicalString(raw.grantRef, "request.grantRef"),
      source: principal(raw.source, "request.source"),
      sourceBinding: binding(raw.sourceBinding, "request.sourceBinding"),
      targetPrincipalId: canonicalString(raw.targetPrincipalId, "request.targetPrincipalId"),
      targetDomainId: canonicalString(raw.targetDomainId, "request.targetDomainId"),
      sink: sink as MaterialSink,
      selectedMaterialIds: selectedIds(raw.selectedMaterialIds, "request.selectedMaterialIds"),
    });
  }

  #continuationCommand(value: unknown): ContinuationCommand {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      return invalid("command must be an object");
    }
    const kind = canonicalString((value as Readonly<Record<string, unknown>>).kind, "command.kind");
    const keys = [
      "grantRef",
      "operationId",
      "continuationId",
      "nativeRequestId",
      "requestedAction",
      "principal",
      "binding",
      "kind",
    ];
    if (kind === "answer") keys.push("responseKind", "content");
    else if (kind !== "cancel") return invalid("command.kind is unsupported");
    const raw = exactRecord(value, "command", keys);
    const base = {
      grantRef: canonicalString(raw.grantRef, "command.grantRef"),
      operationId: canonicalString(raw.operationId, "command.operationId"),
      continuationId: canonicalString(raw.continuationId, "command.continuationId"),
      nativeRequestId: canonicalString(raw.nativeRequestId, "command.nativeRequestId"),
      requestedAction: canonicalString(raw.requestedAction, "command.requestedAction"),
      principal: principal(raw.principal, "command.principal"),
      binding: binding(raw.binding, "command.binding"),
    };
    if (kind === "cancel") return Object.freeze({ ...base, kind });
    if (typeof raw.content !== "string") return invalid("command.content must be text");
    return Object.freeze({
      ...base,
      kind,
      responseKind: canonicalString(raw.responseKind, "command.responseKind"),
      content: raw.content,
    });
  }

  #pendingQuery(value: unknown): PendingContinuationQuery {
    const raw = exactRecord(value, "query", ["principal", "binding"]);
    return Object.freeze({
      principal: principal(raw.principal, "query.principal"),
      binding: binding(raw.binding, "query.binding"),
    });
  }

  #continuationFingerprint(command: ContinuationCommand): string {
    return JSON.stringify({
      continuationId: command.continuationId,
      nativeRequestId: command.nativeRequestId,
      requestedAction: command.requestedAction,
      principal: command.principal,
      binding: command.binding,
      kind: command.kind,
      responseKind: command.kind === "answer" ? command.responseKind : null,
      content: command.kind === "answer" ? command.content : null,
    });
  }
}
