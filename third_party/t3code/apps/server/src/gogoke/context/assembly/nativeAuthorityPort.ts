import type {
  ContextManifest,
  ContextObject,
  ContextScope,
  ContextState,
} from "../../contracts/model.ts";
import type { NativeStoreSession } from "../../bootstrap/nativeStoreService.ts";
import type {
  NativeAuthorizedContextReadSource,
  NativeContextAssemblyBasis,
  NativeContextAssemblySnapshot,
  NativeContextAssemblySource,
  NativeContextManifestCommitRequest,
  NativeContextManifestReceipt,
  NativeContextManifestReplayIdentity,
  NativeContextPartitionBinding,
  NativeContextReadRequest,
  NativeGranteeContextReadRequest,
  TaskContextRequirements,
} from "../../persistence/base/nativeHostClient.ts";
import { NativeHostClientError } from "../../persistence/base/nativeHostClient.ts";
import {
  ContextAssemblyError,
  type AssemblyBasis,
  type AssemblyIdentity,
  type AssemblyRequest,
  type CommitManifestRequest,
  type CommitManifestResult,
  type ContextAssemblyAuthorityPort,
  type ContextVersionRef,
  type ExpectedContextVersion,
  type ReplayManifestRequest,
  type ResolvedContextVersion,
} from "./model.ts";
import { canonical, hashData, hashText } from "./passive.ts";

/**
 * The content source is deliberately non-authorizing.  Every call receives
 * the exact native rows that are current for this operation.  It must not
 * search a store, derive grants, or widen that set on its own.
 */
export interface NativeBoundedContextSource {
  search(input: {
    readonly query: string;
    readonly authorizedSources: ReadonlyArray<NativeContextAssemblySource>;
  }): Promise<ReadonlyArray<ContextVersionRef>>;
  load(input: {
    readonly references: ReadonlyArray<ContextVersionRef>;
    readonly authorizedSources: ReadonlyArray<NativeContextAssemblySource>;
    readonly authorizedReads: ReadonlyArray<NativeAuthorizedContextReadSource>;
  }): Promise<ReadonlyArray<ResolvedContextVersion>>;
}

/** An operation-scoped plan. Mandatory Context refs come only from current native Task authority. */
export interface NativeContextAssemblyPlan {
  readonly snapshot: NativeContextAssemblySnapshot;
  /** Fixed by the operation owner; reused verbatim for idempotent retries. */
  readonly recordedAt: string;
}

const identityKeys = [
  "principalId",
  "seatId",
  "taskId",
  "sessionId",
  "domainId",
  "bindingId",
  "bindingGeneration",
  "sourceEpoch",
  "runtimeInstanceId",
] as const;

const scopes: ReadonlyArray<ContextScope> = ["GLOBAL", "PROJECT", "SESSION"];
const states: ReadonlyArray<ContextState> = [
  "ACTIVE",
  "SUPERSEDED",
  "CONFLICTED",
  "STALE",
  "REVOKED",
  "ARCHIVED",
];

const protocol = (): never => {
  throw new ContextAssemblyError("AUTHORITY_PROTOCOL_ERROR");
};
const denied = (): never => {
  throw new ContextAssemblyError("ACCESS_DENIED");
};
const stale = (): never => {
  throw new ContextAssemblyError("STALE_ASSEMBLY");
};
const unavailable = (): never => {
  throw new ContextAssemblyError("AUTHORITY_UNAVAILABLE");
};

const text = (value: unknown): string => {
  if (typeof value !== "string" || value.length === 0 || value.length > 256) return protocol();
  return value;
};

const u64 = (value: unknown): string => {
  if (
    typeof value !== "string" ||
    !/^(?:0|[1-9][0-9]*)$/u.test(value) ||
    value.length > 20 ||
    BigInt(value) > 18_446_744_073_709_551_615n
  ) {
    return protocol();
  }
  return value;
};

const digest = (value: unknown): string => {
  if (typeof value !== "string" || !/^sha256:[0-9a-f]{64}$/u.test(value)) return protocol();
  return value;
};

const recordedAt = (value: unknown): string => {
  if (
    typeof value !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$/u.test(value)
  )
    return protocol();
  return value;
};

const safeCount = (value: unknown, minimum = 0): number => {
  if (!Number.isSafeInteger(value) || (value as number) < minimum) return protocol();
  return value as number;
};

const refKey = (ref: ContextVersionRef): string =>
  `${ref.sourceDomainId}\u0000${ref.contextId}\u0000${ref.version}`;

const grantKey = (grant: {
  readonly grantId: string;
  readonly revision: string;
  readonly revocationHead: string;
}): string => `${grant.grantId}\u0000${grant.revision}\u0000${grant.revocationHead}`;

const partitionKey = (partition: {
  readonly sourceDomainId: string;
  readonly grant: {
    readonly grantId: string;
    readonly revision: string;
    readonly revocationHead: string;
  };
}): string => `${partition.sourceDomainId}\u0000${grantKey(partition.grant)}`;

function copyRef(value: ContextVersionRef): ContextVersionRef {
  return Object.freeze({
    sourceDomainId: text(value.sourceDomainId),
    contextId: text(value.contextId),
    version: u64(value.version),
  });
}

function copyGrant(value: {
  readonly grantId: string;
  readonly revision: string;
  readonly revocationHead: string;
}) {
  return Object.freeze({
    grantId: text(value.grantId),
    revision: u64(value.revision),
    revocationHead: u64(value.revocationHead),
  });
}

function copyPartition(value: NativeContextPartitionBinding): NativeContextPartitionBinding {
  if (!scopes.includes(value.destinationScope)) return protocol();
  return Object.freeze({
    sourceDomainId: text(value.sourceDomainId),
    destinationScope: value.destinationScope,
    promotionKind: text(value.promotionKind),
    grant: copyGrant(value.grant),
  });
}

function copySnapshot(value: NativeContextAssemblySnapshot): NativeContextAssemblySnapshot {
  const partitionBindings = Object.freeze(value.partitionBindings.map(copyPartition));
  if (
    partitionBindings.length === 0 ||
    new Set(partitionBindings.map(partitionKey)).size !== partitionBindings.length
  ) {
    return protocol();
  }
  const snapshot = Object.freeze({
    operationId: text(value.operationId),
    principalId: text(value.principalId),
    seatId: text(value.seatId),
    taskId: text(value.taskId),
    sessionId: text(value.sessionId),
    domainId: text(value.domainId),
    bindingId: text(value.bindingId),
    bindingGeneration: u64(value.bindingGeneration),
    sourceEpoch: u64(value.sourceEpoch),
    runtimeInstanceId: text(value.runtimeInstanceId),
    taskRevision: u64(value.taskRevision),
    policyRevision: u64(value.policyRevision),
    authRevision: u64(value.authRevision),
    revocationHead: u64(value.revocationHead),
    selectionDecisionId: text(value.selectionDecisionId),
    manifestId: text(value.manifestId),
    admissionActionOperationId: text(value.admissionActionOperationId),
    admissionDigest: digest(value.admissionDigest),
    maxContentBytes: safeCount(value.maxContentBytes),
    maxCandidates: safeCount(value.maxCandidates, 1),
    partitionBindings,
  });
  if (snapshot.maxCandidates > 64) return protocol();
  return snapshot;
}

function copyPlan(value: NativeContextAssemblyPlan): NativeContextAssemblyPlan {
  const keys = Reflect.ownKeys(value);
  if (
    keys.length !== 2 ||
    !keys.includes("snapshot") ||
    !keys.includes("recordedAt") ||
    keys.some((key) => typeof key !== "string")
  )
    return protocol();
  const snapshot = copySnapshot(value.snapshot);
  return Object.freeze({ snapshot, recordedAt: recordedAt(value.recordedAt) });
}

function identityOf(
  snapshot: NativeContextAssemblySnapshot,
  operationId = snapshot.operationId,
): NativeContextManifestReplayIdentity {
  return Object.freeze({
    operationId,
    principalId: snapshot.principalId,
    seatId: snapshot.seatId,
    taskId: snapshot.taskId,
    sessionId: snapshot.sessionId,
    domainId: snapshot.domainId,
    bindingId: snapshot.bindingId,
    bindingGeneration: snapshot.bindingGeneration,
    sourceEpoch: snapshot.sourceEpoch,
    runtimeInstanceId: snapshot.runtimeInstanceId,
  });
}

function requestMatchesSnapshot(
  request: AssemblyRequest,
  snapshot: NativeContextAssemblySnapshot,
): boolean {
  return (
    request.operationId === snapshot.operationId &&
    request.manifestId === snapshot.manifestId &&
    request.maxContentBytes >= 0 &&
    Number.isSafeInteger(request.maxContentBytes) &&
    request.maxContentBytes <= snapshot.maxContentBytes &&
    identityKeys.every((key) => request[key] === snapshot[key])
  );
}

function basisIdentityMatches(
  basis: AssemblyBasis,
  snapshot: NativeContextAssemblySnapshot,
): boolean {
  return identityKeys.every((key) => basis[key] === snapshot[key]);
}

function sameRef(left: ContextVersionRef, right: ContextVersionRef): boolean {
  return refKey(left) === refKey(right);
}

function sameGrant(
  left: { readonly grantId: string; readonly revision: string; readonly revocationHead: string },
  right: { readonly grantId: string; readonly revision: string; readonly revocationHead: string },
): boolean {
  return grantKey(left) === grantKey(right);
}

function sameExpected(left: ExpectedContextVersion, right: NativeContextAssemblySource): boolean {
  return (
    left.sourceDomainId === right.sourceDomainId &&
    left.contextId === right.contextId &&
    left.version === right.version &&
    left.contentHash === right.contentHash &&
    left.stateRevision === right.stateRevision &&
    left.accessPolicyRevision === right.accessPolicyRevision
  );
}

const excludedReasons = new Set(["NEEDS_EVIDENCE", "NEEDS_BUDGET", "VERSION_NOT_SELECTED"]);

function manifestExcludedRefs(manifest: ContextManifest): ReadonlyArray<ContextVersionRef> {
  const snapshot = manifest.sourceSnapshot;
  if (typeof snapshot !== "object" || snapshot === null || Array.isArray(snapshot))
    return protocol();
  const excluded = (snapshot as Record<string, unknown>).excluded;
  if (!Array.isArray(excluded)) return protocol();
  const refs = excluded.map((value) => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) return protocol();
    const record = value as Record<string, unknown>;
    const keys = Reflect.ownKeys(record);
    if (
      keys.length !== 4 ||
      !["sourceDomainId", "contextId", "version", "reason"].every((key) => keys.includes(key)) ||
      typeof record.reason !== "string" ||
      !excludedReasons.has(record.reason)
    )
      return protocol();
    return copyRef(record as unknown as ContextVersionRef);
  });
  if (new Set(refs.map(refKey)).size !== refs.length) return protocol();
  return Object.freeze(refs);
}

const expectedManifestConstraintFields = [
  "sourceDomainId",
  "contextId",
  "version",
  "contentHash",
  "stateRevision",
  "accessPolicyRevision",
] as const;

function manifestRequiredVersions(
  manifest: ContextManifest,
): ReadonlyArray<ExpectedContextVersion> {
  if (!Array.isArray(manifest.requiredConstraints)) return protocol();
  const refs = manifest.requiredConstraints.map((value) => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) return protocol();
    const record = value as Record<string, unknown>;
    if (
      Reflect.ownKeys(record).length !== expectedManifestConstraintFields.length ||
      expectedManifestConstraintFields.some((key) => !Object.hasOwn(record, key))
    )
      return protocol();
    digest(record.contentHash);
    u64(record.stateRevision);
    u64(record.accessPolicyRevision);
    const ref = copyRef(record as unknown as ContextVersionRef);
    return Object.freeze({
      ...ref,
      contentHash: digest(record.contentHash),
      stateRevision: u64(record.stateRevision),
      accessPolicyRevision: u64(record.accessPolicyRevision),
    });
  });
  if (new Set(refs.map(refKey)).size !== refs.length) return protocol();
  return Object.freeze(refs);
}

interface ParsedIncludedVersion {
  readonly version: ExpectedContextVersion;
  readonly reason: "MANDATORY_CONSTRAINT" | "AUTHORIZED_RETRIEVAL";
}

function manifestIncludedVersions(manifest: ContextManifest): ReadonlyArray<ParsedIncludedVersion> {
  if (!Array.isArray(manifest.includedVersions)) return protocol();
  const refs = manifest.includedVersions.map((value) => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) return protocol();
    const record = value as Record<string, unknown>;
    if (
      Reflect.ownKeys(record).length !== expectedManifestConstraintFields.length + 1 ||
      expectedManifestConstraintFields.some((key) => !Object.hasOwn(record, key)) ||
      !Object.hasOwn(record, "reason") ||
      (record.reason !== "MANDATORY_CONSTRAINT" && record.reason !== "AUTHORIZED_RETRIEVAL")
    )
      return protocol();
    digest(record.contentHash);
    u64(record.stateRevision);
    u64(record.accessPolicyRevision);
    const ref = copyRef(record as unknown as ContextVersionRef);
    return Object.freeze({
      version: Object.freeze({
        ...ref,
        contentHash: digest(record.contentHash),
        stateRevision: u64(record.stateRevision),
        accessPolicyRevision: u64(record.accessPolicyRevision),
      }),
      reason: record.reason,
    });
  });
  if (new Set(refs.map(({ version }) => refKey(version))).size !== refs.length) return protocol();
  return Object.freeze(refs);
}

function refsOfVersions(
  versions: ReadonlyArray<ExpectedContextVersion>,
): ReadonlyArray<ContextVersionRef> {
  return Object.freeze(
    versions.map(({ sourceDomainId, contextId, version }) =>
      Object.freeze({ sourceDomainId, contextId, version }),
    ),
  );
}

function assertMandatoryIncludedAlignment(
  required: ReadonlyArray<ExpectedContextVersion>,
  included: ReadonlyArray<ParsedIncludedVersion>,
): void {
  const requiredByRef = new Map(required.map((version) => [refKey(version), version]));
  const includedByRef = new Map(included.map((item) => [refKey(item.version), item]));
  for (const version of required) {
    const item = includedByRef.get(refKey(version));
    if (
      item === undefined ||
      item.reason !== "MANDATORY_CONSTRAINT" ||
      !sameExpectedVersions(version, item.version)
    )
      return protocol();
  }
  for (const item of included) {
    if (item.reason === "MANDATORY_CONSTRAINT") {
      const requiredVersion = requiredByRef.get(refKey(item.version));
      if (requiredVersion === undefined || !sameExpectedVersions(requiredVersion, item.version))
        return protocol();
    }
  }
}

function sameExpectedVersions(
  left: ExpectedContextVersion,
  right: ExpectedContextVersion,
): boolean {
  return (
    left.sourceDomainId === right.sourceDomainId &&
    left.contextId === right.contextId &&
    left.version === right.version &&
    left.contentHash === right.contentHash &&
    left.stateRevision === right.stateRevision &&
    left.accessPolicyRevision === right.accessPolicyRevision
  );
}

function sameRefSequence(
  left: ReadonlyArray<ContextVersionRef>,
  right: ReadonlyArray<ContextVersionRef>,
): boolean {
  return left.length === right.length && left.every((ref, index) => sameRef(ref, right[index]!));
}

function manifestTopLevelMatches(
  manifest: ContextManifest,
  snapshot: NativeContextAssemblySnapshot,
): boolean {
  return (
    manifest.manifestId === snapshot.manifestId &&
    manifest.taskId === snapshot.taskId &&
    manifest.seatId === snapshot.seatId &&
    manifest.domainId === snapshot.domainId &&
    manifest.bindingGeneration === snapshot.bindingGeneration &&
    manifest.policyRevision === snapshot.policyRevision &&
    manifest.selectionDecisionId === snapshot.selectionDecisionId
  );
}

function partitionFor(
  partitions: ReadonlyArray<NativeContextPartitionBinding>,
  row: NativeContextAssemblySource,
): NativeContextPartitionBinding | undefined {
  return partitions.find(
    (partition) =>
      partition.sourceDomainId === row.sourceDomainId && sameGrant(partition.grant, row.grant),
  );
}

function validateBasis(
  basis: NativeContextAssemblyBasis,
  snapshot: NativeContextAssemblySnapshot,
  requestedMaxContentBytes: number,
): ReadonlyArray<ContextVersionRef> {
  if (
    basis.operationId !== snapshot.operationId ||
    basis.bindingGeneration !== snapshot.bindingGeneration ||
    basis.sourceEpoch !== snapshot.sourceEpoch ||
    basis.taskRevision !== snapshot.taskRevision ||
    basis.policyRevision !== snapshot.policyRevision ||
    basis.authRevision !== snapshot.authRevision ||
    basis.revocationHead !== snapshot.revocationHead ||
    basis.maxContentBytes !== requestedMaxContentBytes ||
    basis.maxCandidates !== snapshot.maxCandidates
  ) {
    return stale();
  }
  const mandatoryRefs = requestedRefs(basis.mandatoryRefs);
  if (mandatoryRefs.length > basis.maxCandidates) return protocol();
  const nativePartitions = basis.partitionBindings.map(copyPartition);
  if (
    nativePartitions.length !== snapshot.partitionBindings.length ||
    new Set(nativePartitions.map(partitionKey)).size !== nativePartitions.length ||
    nativePartitions.some(
      (partition) =>
        !snapshot.partitionBindings.some(
          (expected) =>
            partitionKey(expected) === partitionKey(partition) &&
            expected.destinationScope === partition.destinationScope &&
            expected.promotionKind === partition.promotionKind,
        ),
    )
  ) {
    return stale();
  }
  return mandatoryRefs;
}

function basisProvenanceKey(basis: AssemblyBasis): string {
  return canonical(basis);
}

function frozenBasis(basis: AssemblyBasis): boolean {
  return (
    Object.isFrozen(basis) &&
    Object.isFrozen(basis.partitions) &&
    basis.partitions.every(Object.isFrozen) &&
    Object.isFrozen(basis.requiredConstraints) &&
    basis.requiredConstraints.every(Object.isFrozen)
  );
}

function validateCurrentTask(task: TaskContextRequirements, basis: AssemblyBasis): void {
  if (task.domainId !== basis.domainId || task.taskId !== basis.taskId) return protocol();
  const refs = requestedRefs(task.mandatoryRefs);
  if (
    task.taskRevision !== basis.taskRevision ||
    !sameRefSequence(refs, basis.requiredConstraints)
  )
    return stale();
}

function basisPartitions(
  basis: NativeContextAssemblyBasis,
): ReadonlyArray<AssemblyBasis["partitions"][number]> {
  return Object.freeze(
    basis.partitionBindings.map((partition) =>
      Object.freeze({
        sourceDomainId: partition.sourceDomainId,
        authorizationRef: partition.grant.grantId,
        authorizationRevision: partition.grant.revision,
      }),
    ),
  );
}

function sourceRows(
  value: ReadonlyArray<NativeContextAssemblySource>,
  basis: AssemblyBasis,
  plan: NativeContextAssemblyPlan,
): ReadonlyArray<NativeContextAssemblySource> {
  const partitions = plan.snapshot.partitionBindings;
  const seen = new Set<string>();
  const rows = Object.freeze(
    value.map((raw) => {
      const row = Object.freeze({
        sourceDomainId: text(raw.sourceDomainId),
        contextId: text(raw.contextId),
        version: u64(raw.version),
        scope: text(raw.scope),
        kind: text(raw.kind),
        contentHash: digest(raw.contentHash),
        sourceRef: text(raw.sourceRef),
        sourceHash: digest(raw.sourceHash),
        sourceAuthorityKind: text(raw.sourceAuthorityKind),
        sourceAuthorityRef: text(raw.sourceAuthorityRef),
        accessPolicyRevision: u64(raw.accessPolicyRevision),
        stateRevision: u64(raw.stateRevision),
        grant: copyGrant(raw.grant),
      });
      if (
        !scopes.includes(row.scope as ContextScope) ||
        partitionFor(partitions, row) === undefined
      )
        return denied();
      const key = `${row.sourceDomainId}\u0000${row.contextId}\u0000${row.version}`;
      if (seen.has(key)) return protocol();
      seen.add(key);
      if (
        !basis.partitions.some(
          (partition) =>
            partition.sourceDomainId === row.sourceDomainId &&
            partition.authorizationRef === row.grant.grantId &&
            partition.authorizationRevision === row.grant.revision,
        )
      ) {
        return stale();
      }
      return row;
    }),
  );
  const available = new Set(rows.map(refKey));
  if (basis.requiredConstraints.some((reference) => !available.has(refKey(reference))))
    return denied();
  return rows;
}

function sourceMap(
  rows: ReadonlyArray<NativeContextAssemblySource>,
): ReadonlyMap<string, NativeContextAssemblySource> {
  return new Map(rows.map((row) => [refKey(row), row]));
}

function requestedRefs(value: ReadonlyArray<ContextVersionRef>): ReadonlyArray<ContextVersionRef> {
  const refs = Object.freeze(value.map(copyRef));
  if (new Set(refs.map(refKey)).size !== refs.length) return protocol();
  return refs;
}

function nativeReadRequest(
  basis: AssemblyBasis,
  row: NativeContextAssemblySource,
  partition: NativeContextPartitionBinding,
): NativeGranteeContextReadRequest {
  const source: NativeContextReadRequest = Object.freeze({
    sourceDomainId: row.sourceDomainId,
    contextId: row.contextId,
    version: row.version,
    expectedScope: row.scope,
    expectedContentHash: row.contentHash,
    expectedAccessPolicyRevision: row.accessPolicyRevision,
    destinationDomainId: basis.domainId,
    destinationScope: partition.destinationScope,
    promotionKind: partition.promotionKind,
    policyRevision: basis.policyRevision,
    grant: row.grant,
  });
  return Object.freeze({ principalId: basis.principalId, seatId: basis.seatId, source });
}

function readSourceKey(value: NativeAuthorizedContextReadSource): string {
  return `${value.sourceDomainId}\u0000${value.contextId}\u0000${value.version}`;
}

function validateMaterial(
  material: ResolvedContextVersion,
  row: NativeContextAssemblySource,
  read: NativeAuthorizedContextReadSource,
): ResolvedContextVersion {
  const object = material.object as ContextObject;
  if (
    typeof material.content !== "string" ||
    object.contextId !== row.contextId ||
    object.version !== row.version ||
    object.domainId !== row.sourceDomainId ||
    object.scope !== row.scope ||
    object.kind !== row.kind ||
    object.contentHash !== row.contentHash ||
    object.accessPolicyRevision !== row.accessPolicyRevision ||
    read.state !== material.state ||
    read.stateRevision !== material.stateRevision ||
    read.accessPolicyRevision !== material.accessPolicyRevision ||
    read.contentHash !== object.contentHash ||
    hashText(material.content) !== object.contentHash
  ) {
    return protocol();
  }
  const source =
    typeof object.sourceRef === "object" &&
    object.sourceRef !== null &&
    !Array.isArray(object.sourceRef)
      ? (object.sourceRef as Record<string, unknown>)
      : undefined;
  const authority =
    typeof object.sourceAuthority === "object" &&
    object.sourceAuthority !== null &&
    !Array.isArray(object.sourceAuthority)
      ? (object.sourceAuthority as Record<string, unknown>)
      : undefined;
  if (
    source === undefined ||
    typeof source.ref !== "string" ||
    typeof source.hash !== "string" ||
    source.ref !== row.sourceRef ||
    source.hash !== row.sourceHash ||
    authority === undefined ||
    typeof authority.kind !== "string" ||
    typeof authority.ref !== "string" ||
    authority.kind !== row.sourceAuthorityKind ||
    authority.ref !== row.sourceAuthorityRef
  )
    return protocol();
  const state = material.state;
  if (!states.includes(state) || !states.includes(object.validity)) return protocol();
  return Object.freeze({
    object: Object.freeze(object),
    content: material.content,
    state,
    stateRevision: read.stateRevision,
    accessPolicyRevision: read.accessPolicyRevision,
  });
}

function validateReceipt(
  receipt: NativeContextManifestReceipt,
  request: CommitManifestRequest,
): CommitManifestResult {
  if (
    receipt.operationId !== request.operationId ||
    receipt.manifestId !== request.manifest.manifestId ||
    receipt.manifestHash !== request.manifest.manifestHash ||
    receipt.canonicalManifest !== canonical(request.manifest)
  ) {
    throw new ContextAssemblyError("COMMIT_OUTCOME_UNKNOWN");
  }
  return Object.freeze({
    kind: receipt.disposition === "COMMITTED" ? "committed" : "replayed",
    operationId: receipt.operationId,
    manifestId: receipt.manifestId,
    manifestHash: receipt.manifestHash,
  });
}

function replayManifest(
  receipt: NativeContextManifestReceipt,
  request: ReplayManifestRequest,
  snapshot: NativeContextAssemblySnapshot,
  requiredConstraints: ReadonlyArray<ContextVersionRef>,
): ContextManifest {
  if (
    receipt.operationId !== request.operationId ||
    receipt.operationId !== snapshot.operationId ||
    receipt.manifestId !== snapshot.manifestId ||
    receipt.disposition !== "REPLAYED"
  ) {
    return protocol();
  }
  let value: unknown;
  try {
    value = JSON.parse(receipt.canonicalManifest) as unknown;
  } catch {
    return protocol();
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) return protocol();
  if (receipt.canonicalManifest !== canonical(value)) return protocol();
  const record = value as Record<string, unknown>;
  const keys = [
    "manifestId",
    "taskId",
    "seatId",
    "bindingGeneration",
    "domainId",
    "policyRevision",
    "sourceSnapshot",
    "requiredConstraints",
    "includedVersions",
    "redactions",
    "selectionDecisionId",
    "manifestHash",
  ];
  if (
    Reflect.ownKeys(record).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(record, key))
  )
    return protocol();
  if (
    typeof record.manifestHash !== "string" ||
    record.manifestHash !== receipt.manifestHash ||
    typeof record.manifestId !== "string" ||
    !manifestTopLevelMatches(record as unknown as ContextManifest, snapshot) ||
    record.manifestId.length === 0 ||
    !Array.isArray(record.requiredConstraints) ||
    !Array.isArray(record.includedVersions) ||
    !Array.isArray(record.redactions)
  ) {
    return protocol();
  }
  if (
    typeof record.sourceSnapshot !== "object" ||
    record.sourceSnapshot === null ||
    Array.isArray(record.sourceSnapshot)
  )
    return protocol();
  const source = record.sourceSnapshot as Record<string, unknown>;
  const sourceKeys = [
    "assemblySchema",
    "operationId",
    "requestDigest",
    "principalId",
    "sessionId",
    "bindingId",
    "sourceEpoch",
    "runtimeInstanceId",
    "taskRevision",
    "authRevision",
    "revocationHead",
    "partitions",
    "mode",
    "excluded",
  ];
  if (
    Reflect.ownKeys(source).length !== sourceKeys.length ||
    sourceKeys.some((key) => !Object.hasOwn(source, key)) ||
    source.assemblySchema !== "gogoke.context-assembly.v1" ||
    source.mode !== "FIXED_SOURCE_RULES" ||
    source.operationId !== request.operationId ||
    source.operationId !== snapshot.operationId ||
    source.principalId !== snapshot.principalId ||
    source.sessionId !== snapshot.sessionId ||
    source.bindingId !== snapshot.bindingId ||
    source.sourceEpoch !== snapshot.sourceEpoch ||
    source.runtimeInstanceId !== snapshot.runtimeInstanceId ||
    source.taskRevision !== snapshot.taskRevision ||
    source.authRevision !== snapshot.authRevision ||
    source.revocationHead !== snapshot.revocationHead
  )
    return protocol();
  digest(source.requestDigest);
  u64(source.sourceEpoch);
  u64(source.taskRevision);
  u64(source.authRevision);
  u64(source.revocationHead);
  if (!Array.isArray(source.partitions)) return protocol();
  const sourceDomains = source.partitions.map((value) => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) return protocol();
    const partition = value as Record<string, unknown>;
    if (Reflect.ownKeys(partition).length !== 1 || typeof partition.sourceDomainId !== "string")
      return protocol();
    return text(partition.sourceDomainId);
  });
  const requiredVersions = manifestRequiredVersions(record as unknown as ContextManifest);
  const requiredRefs = refsOfVersions(requiredVersions);
  if (!sameRefSequence(requiredRefs, requiredConstraints)) return protocol();
  const includedVersions = manifestIncludedVersions(record as unknown as ContextManifest);
  assertMandatoryIncludedAlignment(requiredVersions, includedVersions);
  const includedRefs = refsOfVersions(includedVersions.map((item) => item.version));
  const excludedRefs = manifestExcludedRefs(record as unknown as ContextManifest);
  const disclosedDomains = new Set(
    [...requiredRefs, ...includedRefs, ...excludedRefs].map((ref) => ref.sourceDomainId),
  );
  const allowedDomains = new Set(
    snapshot.partitionBindings.map((partition) => partition.sourceDomainId),
  );
  if (
    new Set(sourceDomains).size !== sourceDomains.length ||
    sourceDomains.length !== disclosedDomains.size ||
    sourceDomains.some((domain) => !disclosedDomains.has(domain)) ||
    sourceDomains.some((domain) => !allowedDomains.has(domain))
  )
    return protocol();
  const { manifestHash: ignored, ...body } = record;
  if (hashData(body) !== record.manifestHash) return protocol();
  return Object.freeze({
    ...body,
    manifestHash: record.manifestHash,
  }) as ContextManifest;
}

function mapNativeCommitError(error: unknown): CommitManifestResult | undefined {
  if (!(error instanceof NativeHostClientError) || error.code !== "HOST_OPERATION")
    return undefined;
  const match = /^HOST_OPERATION: ERR\t([^\t]+)\t(?:0|[1-9][0-9]*)us$/u.exec(error.message);
  if (match === null) return undefined;
  if (match[1] === "OperationConflict") return Object.freeze({ kind: "conflict" });
  if (match[1] === "AccessDenied") return Object.freeze({ kind: "denied" });
  return undefined;
}

/**
 * Native-backed assembly port. Authority, Task requirements, grants, state and
 * durable commit/replay all come from the typed native session. The issued-basis
 * registry is process-local provenance only; current Task truth is always read
 * again from native authority before every downstream operation.
 */
export class NativeContextAssemblyAuthorityPort implements ContextAssemblyAuthorityPort {
  readonly #store!: NativeStoreSession;
  readonly #plan!: NativeContextAssemblyPlan;
  readonly #content!: NativeBoundedContextSource;
  readonly #issuedBasisIdentities = new Set<string>();

  constructor(input: {
    readonly store: NativeStoreSession;
    readonly plan: NativeContextAssemblyPlan;
    readonly content: NativeBoundedContextSource;
  }) {
    if (typeof input.store !== "object" || input.store === null) return protocol();
    if (typeof input.content !== "object" || input.content === null) return protocol();
    if (typeof input.content.search !== "function" || typeof input.content.load !== "function")
      return protocol();
    this.#store = input.store;
    this.#plan = copyPlan(input.plan);
    this.#content = Object.freeze({
      search: input.content.search.bind(input.content),
      load: input.content.load.bind(input.content),
    });
  }

  async openAssembly(request: AssemblyRequest): Promise<AssemblyBasis | null> {
    if (!requestMatchesSnapshot(request, this.#plan.snapshot)) return null;
    const effectiveSnapshot = Object.freeze({
      ...this.#plan.snapshot,
      maxContentBytes: request.maxContentBytes,
      partitionBindings: this.#plan.snapshot.partitionBindings,
    });
    try {
      await this.#store.publishContextAssemblySnapshot(effectiveSnapshot);
      const nativeBasis = await this.#store.readContextAssemblyBasis(identityOf(effectiveSnapshot));
      const mandatoryRefs = validateBasis(nativeBasis, effectiveSnapshot, request.maxContentBytes);
      const partitions = basisPartitions(nativeBasis);
      const basis = Object.freeze({
        principalId: effectiveSnapshot.principalId,
        seatId: effectiveSnapshot.seatId,
        taskId: effectiveSnapshot.taskId,
        sessionId: effectiveSnapshot.sessionId,
        domainId: effectiveSnapshot.domainId,
        bindingId: effectiveSnapshot.bindingId,
        bindingGeneration: effectiveSnapshot.bindingGeneration,
        sourceEpoch: effectiveSnapshot.sourceEpoch,
        runtimeInstanceId: effectiveSnapshot.runtimeInstanceId,
        readRef: effectiveSnapshot.operationId,
        admissionRef: effectiveSnapshot.admissionActionOperationId,
        taskRevision: nativeBasis.taskRevision,
        policyRevision: nativeBasis.policyRevision,
        authRevision: nativeBasis.authRevision,
        revocationHead: nativeBasis.revocationHead,
        selectionDecisionId: effectiveSnapshot.selectionDecisionId,
        maxContentBytes: nativeBasis.maxContentBytes,
        maxCandidates: nativeBasis.maxCandidates,
        partitions,
        requiredConstraints: mandatoryRefs,
      });
      this.#issuedBasisIdentities.add(basisProvenanceKey(basis));
      return basis;
    } catch (error) {
      if (error instanceof ContextAssemblyError) throw error;
      return unavailable();
    }
  }

  async searchVisibleContext(
    basis: AssemblyBasis,
    query: string,
  ): Promise<ReadonlyArray<ContextVersionRef>> {
    this.#validateBasisInput(basis);
    if (typeof query !== "string" || query.length > 4096) return protocol();
    let rows: ReadonlyArray<NativeContextAssemblySource>;
    try {
      await this.#readAndValidateCurrentTask(basis);
      rows = sourceRows(
        await this.#store.listContextAssemblySources(identityOf(this.#plan.snapshot)),
        basis,
        this.#plan,
      );
    } catch (error) {
      if (error instanceof ContextAssemblyError) throw error;
      return unavailable();
    }
    let found: ReadonlyArray<ContextVersionRef>;
    try {
      found = await this.#content.search({ query, authorizedSources: rows });
    } catch (error) {
      if (error instanceof ContextAssemblyError) throw error;
      return unavailable();
    }
    const references = requestedRefs(found);
    if (references.length > basis.maxCandidates) return protocol();
    const allowed = sourceMap(rows);
    if (references.some((reference) => !allowed.has(refKey(reference)))) return denied();
    return references;
  }

  async loadVisibleVersions(
    basis: AssemblyBasis,
    references: ReadonlyArray<ContextVersionRef>,
  ): Promise<ReadonlyArray<ResolvedContextVersion>> {
    this.#validateBasisInput(basis);
    const requested = requestedRefs(references);
    let rows: ReadonlyArray<NativeContextAssemblySource>;
    try {
      await this.#readAndValidateCurrentTask(basis);
      rows = sourceRows(
        await this.#store.listContextAssemblySources(identityOf(this.#plan.snapshot)),
        basis,
        this.#plan,
      );
    } catch (error) {
      if (error instanceof ContextAssemblyError) throw error;
      return unavailable();
    }
    const byReference = sourceMap(rows);
    const selectedRows = requested.map((reference) => {
      const row = byReference.get(refKey(reference));
      if (row === undefined) return denied();
      return row;
    });
    const groups = new Map<
      string,
      {
        readonly partition: NativeContextPartitionBinding;
        readonly rows: NativeContextAssemblySource[];
      }
    >();
    for (const row of selectedRows) {
      const partition = partitionFor(this.#plan.snapshot.partitionBindings, row);
      if (partition === undefined) return stale();
      const key = `${partition.destinationScope}\u0000${partition.promotionKind}`;
      const existing = groups.get(key);
      if (existing === undefined) groups.set(key, { partition, rows: [row] });
      else existing.rows.push(row);
    }
    const authorizedReads: NativeAuthorizedContextReadSource[] = [];
    try {
      for (const group of groups.values()) {
        const readRequests = group.rows.map((row) =>
          nativeReadRequest(basis, row, group.partition),
        );
        const readSet = await this.#store.readGranteeContextSet(readRequests);
        if (
          readSet.principalId !== basis.principalId ||
          readSet.seatId !== basis.seatId ||
          readSet.policyRevision !== basis.policyRevision ||
          readSet.revocationHead !== basis.revocationHead ||
          readSet.destinationDomainId !== basis.domainId ||
          readSet.destinationScope !== group.partition.destinationScope ||
          readSet.promotionKind !== group.partition.promotionKind
        )
          return stale();
        for (const read of readSet.sources) {
          const row = byReference.get(readSourceKey(read));
          if (
            row === undefined ||
            read.grantRevision !== row.grant.revision ||
            read.revocationHead !== row.grant.revocationHead ||
            read.sourceDomainId !== row.sourceDomainId ||
            read.contextId !== row.contextId ||
            read.version !== row.version ||
            read.scope !== row.scope ||
            read.kind !== row.kind ||
            read.contentHash !== row.contentHash ||
            read.sourceRef !== row.sourceRef ||
            read.sourceHash !== row.sourceHash ||
            read.sourceAuthorityKind !== row.sourceAuthorityKind ||
            read.sourceAuthorityRef !== row.sourceAuthorityRef ||
            read.accessPolicyRevision !== row.accessPolicyRevision
          )
            return stale();
          authorizedReads.push(read);
        }
      }
    } catch (error) {
      if (error instanceof ContextAssemblyError) throw error;
      return unavailable();
    }
    if (authorizedReads.length !== requested.length) return stale();
    let materials: ReadonlyArray<ResolvedContextVersion>;
    try {
      materials = await this.#content.load({
        references: requested,
        authorizedSources: Object.freeze(selectedRows),
        authorizedReads: Object.freeze(authorizedReads),
      });
    } catch (error) {
      if (error instanceof ContextAssemblyError) throw error;
      return unavailable();
    }
    const materialMap = new Map<string, ResolvedContextVersion>();
    for (const material of materials) {
      const key = refKey({
        sourceDomainId: material.object.domainId,
        contextId: material.object.contextId,
        version: material.object.version,
      });
      if (materialMap.has(key)) return protocol();
      const row = byReference.get(key);
      const read = authorizedReads.find((entry) => readSourceKey(entry) === key);
      if (row === undefined || read === undefined) return denied();
      materialMap.set(key, validateMaterial(material, row, read));
    }
    if (
      materialMap.size !== requested.length ||
      requested.some((reference) => !materialMap.has(refKey(reference)))
    )
      return denied();
    return Object.freeze(requested.map((reference) => materialMap.get(refKey(reference))!));
  }

  async commitManifest(request: CommitManifestRequest): Promise<CommitManifestResult> {
    this.#validateBasisInput(request.basis);
    try {
      await this.#readAndValidateCurrentTask(request.basis);
    } catch (error) {
      if (error instanceof ContextAssemblyError) throw error;
      return unavailable();
    }
    if (
      request.operationId !== this.#plan.snapshot.operationId ||
      !manifestTopLevelMatches(request.manifest, this.#plan.snapshot)
    )
      return denied();
    const requiredRecords = manifestRequiredVersions(request.manifest);
    if (!sameRefSequence(refsOfVersions(requiredRecords), request.basis.requiredConstraints))
      return denied();
    const includedRecords = manifestIncludedVersions(request.manifest);
    assertMandatoryIncludedAlignment(requiredRecords, includedRecords);
    const expected = Object.freeze(
      request.expectedVersions.map((value) =>
        Object.freeze({
          sourceDomainId: text(value.sourceDomainId),
          contextId: text(value.contextId),
          version: u64(value.version),
          contentHash: digest(value.contentHash),
          stateRevision: u64(value.stateRevision),
          accessPolicyRevision: u64(value.accessPolicyRevision),
        }),
      ),
    );
    if (new Set(expected.map(refKey)).size !== expected.length) return protocol();
    const expectedByRef = new Map(expected.map((version) => [refKey(version), version]));
    if (
      includedRecords.length !== expected.length ||
      includedRecords.some((item) => {
        const expectedVersion = expectedByRef.get(refKey(item.version));
        return (
          expectedVersion === undefined || !sameExpectedVersions(item.version, expectedVersion)
        );
      })
    )
      return protocol();
    const excluded = manifestExcludedRefs(request.manifest);
    const expectedKeys = new Set(expected.map(refKey));
    if (excluded.some((reference) => expectedKeys.has(refKey(reference)))) return protocol();
    let rows: ReadonlyArray<NativeContextAssemblySource>;
    try {
      rows = sourceRows(
        await this.#store.listContextAssemblySources(identityOf(this.#plan.snapshot)),
        request.basis,
        this.#plan,
      );
    } catch (error) {
      if (error instanceof ContextAssemblyError) throw error;
      return unavailable();
    }
    const byReference = sourceMap(rows);
    const readRequests: NativeGranteeContextReadRequest[] = [];
    for (const requiredVersion of requiredRecords) {
      const expectedVersion = expectedByRef.get(refKey(requiredVersion));
      const row = byReference.get(refKey(requiredVersion));
      if (
        expectedVersion === undefined ||
        row === undefined ||
        !sameExpectedVersions(requiredVersion, expectedVersion) ||
        !sameExpected(requiredVersion, row)
      )
        return Object.freeze({ kind: "stale" });
    }
    for (const version of expected) {
      const row = byReference.get(refKey(version));
      if (row === undefined) return Object.freeze({ kind: "stale" });
      if (!sameExpected(version, row)) return Object.freeze({ kind: "stale" });
      const partition = partitionFor(this.#plan.snapshot.partitionBindings, row);
      if (partition === undefined) return Object.freeze({ kind: "stale" });
      readRequests.push(nativeReadRequest(request.basis, row, partition));
    }
    for (const reference of excluded) {
      const row = byReference.get(refKey(reference));
      if (row === undefined) return Object.freeze({ kind: "stale" });
      const partition = partitionFor(this.#plan.snapshot.partitionBindings, row);
      if (partition === undefined) return Object.freeze({ kind: "stale" });
      readRequests.push(nativeReadRequest(request.basis, row, partition));
    }
    const nativeRequest: NativeContextManifestCommitRequest = Object.freeze({
      operationId: request.operationId,
      requestDigest: request.requestDigest,
      eventId: `${request.operationId}:event`,
      receiptId: `${request.operationId}:receipt`,
      recordedAt: this.#plan.recordedAt,
      readRequests: Object.freeze(readRequests),
      expectedVersions: expected,
      canonicalManifest: canonical(request.manifest),
    });
    try {
      return validateReceipt(await this.#store.commitContextManifest(nativeRequest), request);
    } catch (error) {
      const mapped = mapNativeCommitError(error);
      if (mapped !== undefined) return mapped;
      if (error instanceof ContextAssemblyError) throw error;
      throw new ContextAssemblyError("COMMIT_OUTCOME_UNKNOWN");
    }
  }

  async readCurrentManifest(request: ReplayManifestRequest): Promise<ContextManifest | null> {
    const snapshot = this.#plan.snapshot;
    if (
      request.operationId !== snapshot.operationId ||
      !identityKeys.every((key) => request[key] === snapshot[key])
    )
      return null;
    try {
      const currentTask = await this.#store.readTaskContextRequirements({
        domainId: snapshot.domainId,
        taskId: snapshot.taskId,
      });
      if (
        currentTask.domainId !== snapshot.domainId ||
        currentTask.taskId !== snapshot.taskId
      )
        return protocol();
      if (currentTask.taskRevision !== snapshot.taskRevision) return stale();
      const mandatoryRefs = requestedRefs(currentTask.mandatoryRefs);
      const receipt = await this.#store.readContextManifest(
        identityOf(snapshot, request.operationId),
      );
      return replayManifest(receipt, request, snapshot, mandatoryRefs);
    } catch (error) {
      if (error instanceof ContextAssemblyError) throw error;
      return unavailable();
    }
  }

  #validateBasisInput(basis: AssemblyBasis): void {
    if (!frozenBasis(basis) || !this.#issuedBasisIdentities.has(basisProvenanceKey(basis)))
      return denied();
    if (!basisIdentityMatches(basis, this.#plan.snapshot)) return denied();
    if (
      basis.readRef !== this.#plan.snapshot.operationId ||
      basis.admissionRef !== this.#plan.snapshot.admissionActionOperationId ||
      basis.taskRevision !== this.#plan.snapshot.taskRevision ||
      basis.policyRevision !== this.#plan.snapshot.policyRevision ||
      basis.authRevision !== this.#plan.snapshot.authRevision ||
      basis.revocationHead !== this.#plan.snapshot.revocationHead ||
      basis.maxCandidates !== this.#plan.snapshot.maxCandidates ||
      basis.maxContentBytes > this.#plan.snapshot.maxContentBytes
    )
      return stale();
    if (!Number.isSafeInteger(basis.maxContentBytes) || basis.maxContentBytes < 0)
      return protocol();
    const planPartitions = new Set(
      this.#plan.snapshot.partitionBindings.map(
        (partition) =>
          `${partition.sourceDomainId}\u0000${partition.grant.grantId}\u0000${partition.grant.revision}`,
      ),
    );
    const basisPartitions = basis.partitions.map(
      (partition) =>
        `${partition.sourceDomainId}\u0000${partition.authorizationRef}\u0000${partition.authorizationRevision}`,
    );
    if (
      basisPartitions.length !== planPartitions.size ||
      new Set(basisPartitions).size !== basisPartitions.length ||
      basisPartitions.some((partition) => !planPartitions.has(partition))
    )
      return denied();
    requestedRefs(basis.requiredConstraints);
  }

  async #readAndValidateCurrentTask(basis: AssemblyBasis): Promise<void> {
    const task = await this.#store.readTaskContextRequirements({
      domainId: basis.domainId,
      taskId: basis.taskId,
    });
    validateCurrentTask(task, basis);
  }
}

export function createNativeContextAssemblyAuthorityPort(input: {
  readonly store: NativeStoreSession;
  readonly plan: NativeContextAssemblyPlan;
  readonly content: NativeBoundedContextSource;
}): ContextAssemblyAuthorityPort {
  return new NativeContextAssemblyAuthorityPort(input);
}
