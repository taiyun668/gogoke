import { Buffer } from "node:buffer";
import type { ContextManifest, ContextObject, ContextState, JsonValue, SeatId, U64String } from "../../contracts/model.ts";
import {
  type AssemblyBasis, type AssemblyIdentity, type AssemblyRequest,
  type ContextAssemblyAuthorityPort, type ContextVersionRef, type ExpectedContextVersion,
  type ReplayManifestRequest, type ResolvedContextVersion,
} from "./model.ts";
import { canonical, captureMethods, count, digest, fail, hashData, hashText,
  identifier, jsonData, ownArray, ownRecord, u64 } from "./passive.ts";

const IDENTITY = Object.freeze(["principalId", "seatId", "taskId", "sessionId", "domainId",
  "bindingId", "bindingGeneration", "sourceEpoch", "runtimeInstanceId"] as const);
const STATES: readonly ContextState[] = Object.freeze([
  "ACTIVE", "SUPERSEDED", "CONFLICTED", "STALE", "REVOKED", "ARCHIVED",
]);
const MANIFEST_KEYS = Object.freeze(["manifestId", "taskId", "seatId", "bindingGeneration", "domainId",
  "policyRevision", "sourceSnapshot", "requiredConstraints", "includedVersions", "redactions",
  "selectionDecisionId", "manifestHash"]);

function identity(raw: AssemblyIdentity | Readonly<Record<string, unknown>>, input = false): AssemblyIdentity {
  const code = input ? "INVALID_INPUT" : "AUTHORITY_PROTOCOL_ERROR";
  return Object.freeze({
    principalId: identifier(raw.principalId, code), seatId: identifier(raw.seatId, code),
    taskId: identifier(raw.taskId, code), sessionId: identifier(raw.sessionId, code),
    domainId: identifier(raw.domainId, code), bindingId: identifier(raw.bindingId, code),
    bindingGeneration: u64(raw.bindingGeneration, code), sourceEpoch: u64(raw.sourceEpoch, code),
    runtimeInstanceId: identifier(raw.runtimeInstanceId, code),
  });
}

function requestSnapshot(input: AssemblyRequest): AssemblyRequest {
  const raw = ownRecord(input, [...IDENTITY, "operationId", "manifestId", "query", "maxContentBytes"], [], "INVALID_INPUT");
  if (typeof raw.query !== "string" || raw.query.length > 4096) return fail("INVALID_INPUT");
  return Object.freeze({ ...identity(raw, true), operationId: identifier(raw.operationId, "INVALID_INPUT"),
    manifestId: identifier(raw.manifestId, "INVALID_INPUT"), query: raw.query,
    maxContentBytes: count(raw.maxContentBytes, "INVALID_INPUT") });
}

function refSnapshot(input: unknown): ContextVersionRef {
  const raw = ownRecord(input, ["sourceDomainId", "contextId", "version"]);
  return Object.freeze({ sourceDomainId: identifier(raw.sourceDomainId),
    contextId: identifier(raw.contextId), version: u64(raw.version) });
}
const refKey = (ref: ContextVersionRef): string => canonical([ref.sourceDomainId, ref.contextId, ref.version]);
const objectKey = (ref: ContextVersionRef): string => canonical([ref.sourceDomainId, ref.contextId]);

function references(input: unknown): ReadonlyArray<ContextVersionRef> {
  const refs = ownArray(input).map(refSnapshot);
  if (new Set(refs.map(refKey)).size !== refs.length) return fail("AUTHORITY_PROTOCOL_ERROR");
  return Object.freeze(refs);
}

function basisSnapshot(input: unknown, request: AssemblyRequest): AssemblyBasis {
  const raw = ownRecord(input, [...IDENTITY, "readRef", "admissionRef", "taskRevision", "policyRevision",
    "authRevision", "revocationHead", "selectionDecisionId", "maxContentBytes", "maxCandidates",
    "partitions", "requiredConstraints"]);
  const bound = identity(raw);
  if (IDENTITY.some(key => bound[key] !== request[key])) return fail("ACCESS_DENIED");
  const partitions = Object.freeze(ownArray(raw.partitions).map(value => {
    const p = ownRecord(value, ["sourceDomainId", "authorizationRef", "authorizationRevision"]);
    return Object.freeze({ sourceDomainId: identifier(p.sourceDomainId),
      authorizationRef: identifier(p.authorizationRef), authorizationRevision: u64(p.authorizationRevision) });
  }));
  if (new Set(partitions.map(p => canonical([p.sourceDomainId, p.authorizationRef]))).size !== partitions.length) {
    return fail("AUTHORITY_PROTOCOL_ERROR");
  }
  const requiredConstraints = references(raw.requiredConstraints);
  const domains = new Set(partitions.map(p => p.sourceDomainId));
  if (requiredConstraints.some(ref => !domains.has(ref.sourceDomainId))) return fail("ACCESS_DENIED");
  if (new Set(requiredConstraints.map(objectKey)).size !== requiredConstraints.length) return fail("NEEDS_EVIDENCE");
  return Object.freeze({ ...bound, readRef: identifier(raw.readRef), admissionRef: identifier(raw.admissionRef),
    taskRevision: u64(raw.taskRevision), policyRevision: u64(raw.policyRevision), authRevision: u64(raw.authRevision),
    revocationHead: u64(raw.revocationHead), selectionDecisionId: identifier(raw.selectionDecisionId),
    maxContentBytes: count(raw.maxContentBytes), maxCandidates: count(raw.maxCandidates), partitions, requiredConstraints });
}

function stringRefs(input: unknown): ReadonlyArray<string> {
  const result = ownArray(input).map(value => {
    if (typeof value !== "string" || value.length > 277 ||
        !/^[A-Za-z0-9][A-Za-z0-9._:/-]{0,255}@(0|[1-9][0-9]*)$/.test(value)) {
      return fail("AUTHORITY_PROTOCOL_ERROR");
    }
    u64(value.slice(value.lastIndexOf("@") + 1));
    return value;
  });
  if (new Set(result).size !== result.length) return fail("AUTHORITY_PROTOCOL_ERROR");
  return Object.freeze(result);
}
function state(input: unknown): ContextState {
  if (typeof input !== "string" || !STATES.includes(input as ContextState)) return fail("AUTHORITY_PROTOCOL_ERROR");
  return input as ContextState;
}

function materialSnapshot(input: unknown): ResolvedContextVersion {
  const raw = ownRecord(input, ["object", "content", "state", "stateRevision", "accessPolicyRevision"]);
  if (typeof raw.content !== "string") return fail("AUTHORITY_PROTOCOL_ERROR");
  const o = ownRecord(raw.object, ["contextId", "version", "scope", "domainId", "kind", "contentHash",
    "sourceRef", "sourceAuthority", "derivedFrom", "validity", "supersedes", "accessPolicyRevision"]);
  if (o.scope !== "GLOBAL" && o.scope !== "PROJECT" && o.scope !== "SESSION") return fail("AUTHORITY_PROTOCOL_ERROR");
  const object: ContextObject = Object.freeze({
    contextId: identifier(o.contextId), version: u64(o.version) as U64String, scope: o.scope,
    domainId: identifier(o.domainId), kind: identifier(o.kind), contentHash: digest(o.contentHash),
    sourceRef: jsonData(o.sourceRef), sourceAuthority: jsonData(o.sourceAuthority),
    derivedFrom: stringRefs(o.derivedFrom), validity: state(o.validity), supersedes: stringRefs(o.supersedes),
    accessPolicyRevision: u64(o.accessPolicyRevision) as U64String,
  });
  return Object.freeze({ object, content: raw.content, state: state(raw.state),
    stateRevision: u64(raw.stateRevision), accessPolicyRevision: u64(raw.accessPolicyRevision) });
}

function versionOf(material: ResolvedContextVersion): ExpectedContextVersion {
  return Object.freeze({ sourceDomainId: material.object.domainId, contextId: material.object.contextId,
    version: material.object.version, contentHash: material.object.contentHash,
    stateRevision: material.stateRevision, accessPolicyRevision: material.accessPolicyRevision });
}

const EXPECTED_FIELDS = Object.freeze(["sourceDomainId", "contextId", "version", "contentHash", "stateRevision", "accessPolicyRevision"]);

function expectedSnapshot(input: unknown, withReason = false): ExpectedContextVersion {
  const raw = ownRecord(input, EXPECTED_FIELDS, withReason ? ["reason"] : []);
  return Object.freeze({ sourceDomainId: identifier(raw.sourceDomainId), contextId: identifier(raw.contextId),
    version: u64(raw.version), contentHash: digest(raw.contentHash), stateRevision: u64(raw.stateRevision),
    accessPolicyRevision: u64(raw.accessPolicyRevision) });
}

/** Hash identity alone does not establish mandatory-set or selection consistency. */
function assertManifestIntegrity(body: Omit<ContextManifest, "manifestHash">, source: Readonly<Record<string, unknown>>): void {
  const domains = ownArray(source.partitions).map(value => identifier(ownRecord(value, ["sourceDomainId"]).sourceDomainId));
  const domainSet = new Set(domains);
  if (domainSet.size !== domains.length) return fail("AUTHORITY_PROTOCOL_ERROR");
  const included = ownArray(body.includedVersions).map(value => {
    const raw = ownRecord(value, [...EXPECTED_FIELDS, "reason"]);
    if (raw.reason !== "MANDATORY_CONSTRAINT" && raw.reason !== "AUTHORIZED_RETRIEVAL") return fail("AUTHORITY_PROTOCOL_ERROR");
    return Object.freeze({ version: expectedSnapshot(raw, true), reason: raw.reason });
  });
  const required = ownArray(body.requiredConstraints).map(value => expectedSnapshot(value));
  if (new Set(included.map(item => objectKey(item.version))).size !== included.length ||
      new Set(required.map(objectKey)).size !== required.length) return fail("AUTHORITY_PROTOCOL_ERROR");
  const selected = new Map(included.map(item => [refKey(item.version), item]));
  const mandatory = new Map(required.map(item => [refKey(item), item]));
  for (const item of required) {
    const selectedItem = selected.get(refKey(item));
    if (!selectedItem || selectedItem.reason !== "MANDATORY_CONSTRAINT" ||
        canonical(selectedItem.version) !== canonical(item)) return fail("AUTHORITY_PROTOCOL_ERROR");
  }
  const usedDomains = new Set<string>();
  for (const item of included) {
    if (!domainSet.has(item.version.sourceDomainId) ||
        (item.reason === "MANDATORY_CONSTRAINT") !== mandatory.has(refKey(item.version))) return fail("AUTHORITY_PROTOCOL_ERROR");
    usedDomains.add(item.version.sourceDomainId);
  }
  const excludedKeys = new Set<string>();
  for (const value of ownArray(source.excluded)) {
    const raw = ownRecord(value, ["sourceDomainId", "contextId", "version", "reason"]);
    if (raw.reason !== "NEEDS_EVIDENCE" && raw.reason !== "NEEDS_BUDGET" && raw.reason !== "VERSION_NOT_SELECTED") {
      return fail("AUTHORITY_PROTOCOL_ERROR");
    }
    const reference = refSnapshot({ sourceDomainId: raw.sourceDomainId, contextId: raw.contextId, version: raw.version });
    const key = refKey(reference);
    if (!domainSet.has(reference.sourceDomainId) || selected.has(key) || excludedKeys.has(key)) return fail("AUTHORITY_PROTOCOL_ERROR");
    excludedKeys.add(key);
    usedDomains.add(reference.sourceDomainId);
  }
  if (usedDomains.size !== domainSet.size || body.redactions.length !== 0) return fail("AUTHORITY_PROTOCOL_ERROR");
}

function manifestSnapshot(input: unknown, request: ReplayManifestRequest): ContextManifest {
  const raw = ownRecord(input, MANIFEST_KEYS);
  const body = Object.freeze({
    manifestId: identifier(raw.manifestId), taskId: identifier(raw.taskId), seatId: identifier(raw.seatId) as SeatId,
    bindingGeneration: u64(raw.bindingGeneration) as U64String, domainId: identifier(raw.domainId),
    policyRevision: u64(raw.policyRevision) as U64String, sourceSnapshot: jsonData(raw.sourceSnapshot),
    requiredConstraints: Object.freeze(ownArray(raw.requiredConstraints).map(jsonData)),
    includedVersions: Object.freeze(ownArray(raw.includedVersions).map(jsonData)),
    redactions: Object.freeze(ownArray(raw.redactions).map(jsonData)), selectionDecisionId: identifier(raw.selectionDecisionId),
  });
  const manifestHash = digest(raw.manifestHash);
  if (manifestHash !== hashData(body)) return fail("AUTHORITY_PROTOCOL_ERROR");
  const source = ownRecord(body.sourceSnapshot, ["assemblySchema", "operationId", "requestDigest", "principalId",
    "sessionId", "bindingId", "sourceEpoch", "runtimeInstanceId", "taskRevision", "authRevision", "revocationHead",
    "partitions", "mode", "excluded"]);
  if (source.assemblySchema !== "gogoke.context-assembly.v1" || source.mode !== "FIXED_SOURCE_RULES") {
    return fail("AUTHORITY_PROTOCOL_ERROR");
  }
  digest(source.requestDigest);
  for (const key of ["taskRevision", "authRevision", "revocationHead", "sourceEpoch"] as const) u64(source[key]);
  if (source.operationId !== request.operationId || source.principalId !== request.principalId ||
      source.sessionId !== request.sessionId || source.bindingId !== request.bindingId ||
      source.sourceEpoch !== request.sourceEpoch || source.runtimeInstanceId !== request.runtimeInstanceId ||
      body.taskId !== request.taskId || body.seatId !== request.seatId || body.domainId !== request.domainId ||
      body.bindingGeneration !== request.bindingGeneration) return fail("ACCESS_DENIED");
  assertManifestIntegrity(body, source);
  return Object.freeze({ ...body, manifestHash });
}

/**
 * Fixed-source, no-model preparation. There is no private fallback store or cache.
 * Port wiring/SQL/FTS/grant enforcement and the common ActionAdmission still require
 * authoritative integration; constructing this class does not qualify that path.
 */
export class ContextManifestAssembler {
  readonly #authority: ContextAssemblyAuthorityPort;
  constructor(authority: ContextAssemblyAuthorityPort) {
    this.#authority = captureMethods(authority, ["openAssembly", "searchVisibleContext", "loadVisibleVersions",
      "commitManifest", "readCurrentManifest"]);
  }

  async #read<T>(read: () => Promise<T>): Promise<T> {
    try { return await read(); }
    catch { return fail("AUTHORITY_UNAVAILABLE"); }
  }

  async replay(input: ReplayManifestRequest): Promise<ContextManifest> {
    const raw = ownRecord(input, [...IDENTITY, "operationId"], [], "INVALID_INPUT");
    const request = Object.freeze({ ...identity(raw, true), operationId: identifier(raw.operationId, "INVALID_INPUT") });
    // Always re-enter the authoritative read, including repeated operation IDs.
    const value = await this.#read(() => this.#authority.readCurrentManifest(request));
    if (value === null) return fail("ACCESS_DENIED");
    return manifestSnapshot(value, request);
  }

  async assemble(input: AssemblyRequest): Promise<ContextManifest> {
    const request = requestSnapshot(input); // synchronous snapshot before the first await
    const opened = await this.#read(() => this.#authority.openAssembly(request));
    if (opened === null) return fail("ACCESS_DENIED");
    const basis = basisSnapshot(opened, request);
    const found = references(await this.#read(() => this.#authority.searchVisibleContext(basis, request.query)));
    if (found.length > basis.maxCandidates) return fail("AUTHORITY_PROTOCOL_ERROR");
    const domains = new Set(basis.partitions.map(p => p.sourceDomainId));
    if (found.some(ref => !domains.has(ref.sourceDomainId))) return fail("ACCESS_DENIED");
    const wanted = new Map<string, ContextVersionRef>();
    for (const ref of [...basis.requiredConstraints, ...found]) wanted.set(refKey(ref), ref);
    const loaded = ownArray(await this.#read(() => this.#authority.loadVisibleVersions(basis,
      Object.freeze([...wanted.values()])))).map(materialSnapshot);
    const materials = new Map<string, ResolvedContextVersion>();
    for (const item of loaded) {
      const key = refKey(versionOf(item));
      if (!wanted.has(key) || !domains.has(item.object.domainId) || materials.has(key)) return fail("AUTHORITY_PROTOCOL_ERROR");
      materials.set(key, item);
    }
    const budget = Math.min(request.maxContentBytes, basis.maxContentBytes);
    const required = new Set(basis.requiredConstraints.map(refKey));
    const included: ExpectedContextVersion[] = [];
    const excluded: JsonValue[] = [];
    const selectedObjects = new Set<string>();
    let usedBytes = 0;
    for (const [key, ref] of wanted) {
      const must = required.has(key);
      const item = materials.get(key);
      let reason: string | null = null;
      if (!item || item.state !== "ACTIVE" || hashText(item.content) !== item.object.contentHash) reason = "NEEDS_EVIDENCE";
      else if (selectedObjects.has(objectKey(ref))) reason = "VERSION_NOT_SELECTED";
      else if (Buffer.byteLength(item.content, "utf8") > budget - usedBytes) reason = "NEEDS_BUDGET";
      if (reason !== null) {
        if (must) return fail(reason === "NEEDS_BUDGET" ? "NEEDS_BUDGET" : "NEEDS_EVIDENCE");
        // These references already passed domain-scoped retrieval. Never list inaccessible hits.
        excluded.push(Object.freeze({ ...ref, reason }));
        continue;
      }
      if (!item) return fail("NEEDS_EVIDENCE");
      included.push(versionOf(item));
      selectedObjects.add(objectKey(ref));
      usedBytes += Buffer.byteLength(item.content, "utf8");
    }
    const requestDigest = hashData(request);
    const body: Omit<ContextManifest, "manifestHash"> = Object.freeze({
      manifestId: request.manifestId, taskId: request.taskId, seatId: request.seatId as SeatId,
      bindingGeneration: request.bindingGeneration as U64String, domainId: request.domainId,
      policyRevision: basis.policyRevision as U64String,
      sourceSnapshot: jsonData({ assemblySchema: "gogoke.context-assembly.v1", operationId: request.operationId,
        requestDigest, principalId: basis.principalId, sessionId: basis.sessionId, bindingId: basis.bindingId,
        sourceEpoch: basis.sourceEpoch, runtimeInstanceId: basis.runtimeInstanceId, taskRevision: basis.taskRevision,
        authRevision: basis.authRevision, revocationHead: basis.revocationHead,
        // Full grants/read/admission handles remain in the INTERNAL commit basis.
        partitions: [...new Set([...wanted.values()].map(ref => ref.sourceDomainId))].sort()
          .map(sourceDomainId => Object.freeze({ sourceDomainId })),
        mode: "FIXED_SOURCE_RULES", excluded }),
      requiredConstraints: Object.freeze(included.filter(ref => required.has(refKey(ref))).map(ref => jsonData(ref))),
      includedVersions: Object.freeze(included.map(ref => jsonData({ ...ref,
        reason: required.has(refKey(ref)) ? "MANDATORY_CONSTRAINT" : "AUTHORIZED_RETRIEVAL" }))),
      redactions: Object.freeze([]), selectionDecisionId: basis.selectionDecisionId,
    });
    const manifest: ContextManifest = Object.freeze({ ...body, manifestHash: hashData(body) });
    let result: Readonly<Record<string, unknown>>;
    try {
      const value = await this.#authority.commitManifest(Object.freeze({ operationId: request.operationId,
        requestDigest, basis, expectedVersions: Object.freeze(included), manifest }));
      result = ownRecord(value, ["kind"], ["operationId", "manifestId", "manifestHash"]);
      if (result.kind === "committed" || result.kind === "replayed") {
        ownRecord(result, ["kind", "operationId", "manifestId", "manifestHash"]);
        if (result.operationId !== request.operationId || result.manifestId !== manifest.manifestId ||
            result.manifestHash !== manifest.manifestHash) return fail("COMMIT_OUTCOME_UNKNOWN");
      } else if (result.kind === "denied" || result.kind === "stale" || result.kind === "conflict") {
        ownRecord(result, ["kind"]);
      } else return fail("COMMIT_OUTCOME_UNKNOWN");
    } catch { return fail("COMMIT_OUTCOME_UNKNOWN"); }
    if (result.kind === "denied") return fail("ACCESS_DENIED");
    if (result.kind === "stale") return fail("STALE_ASSEMBLY");
    if (result.kind === "conflict") return fail("OPERATION_CONFLICT");
    // Reauthorization after the commit await, not a local cached disclosure.
    const disclosed = await this.replay(Object.freeze({ ...identity(request), operationId: request.operationId }));
    if (disclosed.manifestHash !== manifest.manifestHash || disclosed.manifestId !== manifest.manifestId) {
      return fail("COMMIT_OUTCOME_UNKNOWN");
    }
    return disclosed;
  }
}
