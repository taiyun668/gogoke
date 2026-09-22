import * as NodeCrypto from "node:crypto";

import type { JsonValue } from "../contracts/model.ts";
import { cloneAndFreezeJson } from "../runtimeCatalog/manifest.ts";
import {
  CAPABILITY_SOURCE_KINDS,
  CapabilityCatalogError,
  type CapabilityAuthorityAdapters,
  type CapabilityCatalogSeed,
  type CapabilityCatalogView,
  type CapabilityClaimInput,
  type CapabilityResolution,
  type CapabilitySource,
  type CapabilitySourceKind,
  type CapabilitySubjectRef,
  type CapacityInput,
  type CapacityState,
  type HostTransitionEvidence,
  type InFlightCapabilityBinding,
  type ObservationLevel,
  type ObservationRevocationRequest,
  type PassiveCapabilityObservation,
  type PassiveObservationBatch,
  type PassiveObservationGrant,
  type PassiveObservationRequest,
  type QualificationIdentity,
  type QualificationRequest,
  type QualificationRevocationRequest,
  type QualifiedCapability,
  type QualifiedCapacity,
  type RuntimeAuthoritySnapshot,
  type SourcedCapability,
} from "./types.ts";

const SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;
const U64_PATTERN = /^(?:0|[1-9]\d*)$/;
const LEVEL_RANK: Readonly<Record<ObservationLevel, number>> = Object.freeze({ L1: 1, L2: 2 });
const POLICY_SOURCE: CapabilitySource = Object.freeze({
  kind: "policy",
  ref: "policy://gogoke/capabilities/default-deny",
  revision: "1",
});
const IDENTITY_FIELDS: ReadonlyArray<keyof QualificationIdentity> = Object.freeze([
  "driverId",
  "binaryDigest",
  "adapterDigest",
  "nativeDigest",
  "platform",
  "profileRef",
  "accountRef",
  "authRevision",
  "runtimeInstanceId",
  "nativeModelId",
  "resolvedModelVersion",
  "runtimeMode",
  "isolationProfile",
  "toolProfile",
  "generation",
  "sourceEpoch",
]);

const authenticBindings = new WeakSet<object>();
const trustedSources = new WeakSet<object>();
const trustedGrants = new WeakSet<object>();
const trustedQualifications = new WeakSet<object>();

const invalid = (detail: string): never => {
  throw new CapabilityCatalogError("INVALID_INPUT", detail);
};

const passiveSnapshot = <T>(value: T, path: string): T => {
  try {
    return cloneAndFreezeJson(value as never, path) as T;
  } catch (error) {
    throw new CapabilityCatalogError(
      "INVALID_INPUT",
      `${path} must contain passive JSON data only: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
};

const exactRecord = (
  value: unknown,
  path: string,
  required: ReadonlyArray<string>,
  optional: ReadonlyArray<string> = [],
): Readonly<Record<string, unknown>> => {
  const record = passiveSnapshot(value, path);
  if (record === null || typeof record !== "object" || Array.isArray(record)) {
    return invalid(`${path} must be an object`);
  }
  const keys = Object.keys(record);
  const allowed = new Set([...required, ...optional]);
  const extra = keys.find((key) => !allowed.has(key));
  if (extra !== undefined) return invalid(`${path}.${extra} is not allowed`);
  const missing = required.find((key) => !keys.includes(key));
  if (missing !== undefined) return invalid(`${path}.${missing} is required`);
  return record as Readonly<Record<string, unknown>>;
};

const canonicalString = (value: unknown, path: string): string => {
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    return invalid(`${path} must be a canonical non-empty string`);
  }
  return value;
};

const canonicalU64 = (value: unknown, path: string): string => {
  const result = canonicalString(value, path);
  if (!U64_PATTERN.test(result) || BigInt(result) > 18_446_744_073_709_551_615n) {
    return invalid(`${path} must be a canonical u64`);
  }
  return result;
};

const digest = (value: unknown, path: string): string => {
  const result = canonicalString(value, path);
  if (!SHA256_PATTERN.test(result)) return invalid(`${path} must be a lowercase sha256 digest`);
  return result;
};

const epochMs = (value: unknown, path: string): number => {
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    return invalid(`${path} must be a non-negative safe integer`);
  }
  return value as number;
};

const canonicalJson = (value: JsonValue): string => {
  if (value === null || typeof value === "boolean" || typeof value === "string") {
    return JSON.stringify(value);
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value))
      return invalid("canonical JSON cannot contain a non-finite number");
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  const object = value as Readonly<Record<string, JsonValue>>;
  return `{${Object.keys(object)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${canonicalJson(object[key]!)}`)
    .join(",")}}`;
};

const contentDigest = (value: JsonValue): string =>
  `sha256:${NodeCrypto.createHash("sha256").update(canonicalJson(value)).digest("hex")}`;

const normalizeSource = (
  value: unknown,
  path: string,
  expectedKind: CapabilitySourceKind,
): CapabilitySource => {
  const raw = exactRecord(value, path, ["kind", "ref", "revision"]);
  if (!CAPABILITY_SOURCE_KINDS.includes(raw.kind as CapabilitySourceKind)) {
    return invalid(`${path}.kind is invalid`);
  }
  if (raw.kind !== expectedKind) return invalid(`${path}.kind must be ${expectedKind}`);
  const result = Object.freeze({
    kind: raw.kind as CapabilitySourceKind,
    ref: canonicalString(raw.ref, `${path}.ref`),
    revision: canonicalU64(raw.revision, `${path}.revision`),
  });
  trustedSources.add(result);
  return result;
};

const normalizeGrant = (value: unknown, expectedRef: string): PassiveObservationGrant => {
  const raw = exactRecord(value, "grant", ["level", "grantRef", "grantRevision", "mode"]);
  if (raw.level !== "L1" && raw.level !== "L2") return invalid("grant.level is invalid");
  if (raw.mode !== "passive-only") return invalid("grant.mode must be passive-only");
  const grantRef = canonicalString(raw.grantRef, "grant.grantRef");
  if (grantRef !== expectedRef) return invalid("authority returned the wrong grant");
  const grant = Object.freeze({
    level: raw.level,
    grantRef,
    grantRevision: canonicalU64(raw.grantRevision, "grant.grantRevision"),
    mode: "passive-only",
  });
  trustedGrants.add(grant);
  return grant;
};

const normalizeIdentity = (value: unknown, path: string): QualificationIdentity => {
  const raw = exactRecord(value, path, IDENTITY_FIELDS);
  const accountRef =
    raw.accountRef === null ? null : canonicalString(raw.accountRef, `${path}.accountRef`);
  return Object.freeze({
    driverId: canonicalString(
      raw.driverId,
      `${path}.driverId`,
    ) as QualificationIdentity["driverId"],
    binaryDigest: digest(raw.binaryDigest, `${path}.binaryDigest`),
    adapterDigest: digest(raw.adapterDigest, `${path}.adapterDigest`),
    nativeDigest: digest(raw.nativeDigest, `${path}.nativeDigest`),
    platform: canonicalString(raw.platform, `${path}.platform`),
    profileRef: canonicalString(raw.profileRef, `${path}.profileRef`),
    accountRef,
    authRevision: canonicalU64(raw.authRevision, `${path}.authRevision`),
    runtimeInstanceId: canonicalString(
      raw.runtimeInstanceId,
      `${path}.runtimeInstanceId`,
    ) as QualificationIdentity["runtimeInstanceId"],
    nativeModelId: canonicalString(raw.nativeModelId, `${path}.nativeModelId`),
    resolvedModelVersion: canonicalString(raw.resolvedModelVersion, `${path}.resolvedModelVersion`),
    runtimeMode: canonicalString(raw.runtimeMode, `${path}.runtimeMode`),
    isolationProfile: canonicalString(raw.isolationProfile, `${path}.isolationProfile`),
    toolProfile: canonicalString(raw.toolProfile, `${path}.toolProfile`),
    generation: canonicalU64(raw.generation, `${path}.generation`),
    sourceEpoch: canonicalU64(raw.sourceEpoch, `${path}.sourceEpoch`),
  });
};

export const qualificationKey = (value: QualificationIdentity): string => {
  const identity = normalizeIdentity(value, "identity");
  return IDENTITY_FIELDS.map((field) => {
    const text = identity[field] === null ? "<null>" : String(identity[field]);
    return `${field.length}:${field}=${text.length}:${text}`;
  }).join("|");
};

const authorityFacts = (
  value: RuntimeAuthoritySnapshot,
  subject: CapabilitySubjectRef,
): {
  readonly snapshot: RuntimeAuthoritySnapshot;
  readonly identity: QualificationIdentity;
  readonly sourceEpoch: string;
  readonly authorityDigest: string;
} => {
  const snapshot = passiveSnapshot(value, "authority");
  const { target, readyHost, environment } = snapshot;
  const instance = target.instance.instance;
  const manifest = target.driver.manifest;
  if (
    subject.driverId !== manifest.driverId ||
    subject.runtimeInstanceId !== instance.instanceId ||
    target.driver.driver.driverId !== manifest.driverId ||
    instance.driverId !== manifest.driverId ||
    target.instance.adapterVersion !== manifest.adapterVersion
  ) {
    throw new CapabilityCatalogError("SUBJECT_MISMATCH", "runtime catalog subject disagrees");
  }
  if (
    target.model.runtimeInstanceId !== instance.instanceId ||
    readyHost.binding.runtimeInstanceId !== instance.instanceId ||
    readyHost.binding.profileId !== instance.profileRef ||
    readyHost.binding.authRevision !== instance.authRevision
  ) {
    throw new CapabilityCatalogError(
      "SUBJECT_MISMATCH",
      "model, host binding, and runtime instance identities disagree",
    );
  }
  const identity = normalizeIdentity(
    {
      driverId: manifest.driverId,
      binaryDigest: environment.binaryDigest,
      adapterDigest: manifest.artifactDigest,
      nativeDigest: environment.nativeDigest,
      platform: environment.platform,
      profileRef: instance.profileRef,
      accountRef: target.instance.accountRef ?? null,
      authRevision: instance.authRevision,
      runtimeInstanceId: instance.instanceId,
      nativeModelId: target.model.nativeModelId,
      resolvedModelVersion: target.model.resolvedVersion,
      runtimeMode: environment.runtimeMode,
      isolationProfile: environment.isolationProfile,
      toolProfile: environment.toolProfile,
      generation: readyHost.binding.generation,
      sourceEpoch: readyHost.sourceEpoch,
    },
    "authority.identity",
  );
  const sourceEpoch = canonicalU64(readyHost.sourceEpoch, "authority.readyHost.sourceEpoch");
  return Object.freeze({
    snapshot,
    identity,
    sourceEpoch,
    authorityDigest: contentDigest(snapshot as unknown as JsonValue),
  });
};

const sameIdentity = (left: QualificationIdentity, right: QualificationIdentity): boolean =>
  qualificationKey(left) === qualificationKey(right);

const frozenSources = (
  ...values: ReadonlyArray<CapabilitySource>
): ReadonlyArray<CapabilitySource> =>
  Object.freeze(values.map((value) => Object.freeze({ ...value })));

interface CatalogState {
  currentAuthority: RuntimeAuthoritySnapshot;
  currentAuthorityDigest: string;
  currentIdentity: QualificationIdentity;
  currentSourceEpoch: string;
  sourceEpochHighWater: bigint;
  generationHighWater: bigint;
  authRevisionHighWater: bigint;
  declared: ReadonlyArray<SourcedCapability>;
  observations: ReadonlyArray<PassiveObservationBatch>;
  qualified: ReadonlyArray<QualifiedCapability>;
  qualifiedCapacity: ReadonlyArray<QualifiedCapacity>;
  readonly retiredQualificationKeys: Set<string>;
  readonly retiredObservationDigests: Set<string>;
  readonly retiredProfiles: Set<string>;
  readonly retiredAccounts: Set<string>;
  readonly usedTransitionEvidence: Set<string>;
}

/**
 * Provider-neutral capability authority. The only authority-bearing inputs are
 * resolved by ports captured in the constructor. Request methods accept IDs
 * and passive claims, never a ReadyHost, grant, source, or qualification.
 *
 * No production authority adapter is provided by this module. Until the
 * composition root wires reviewed ports, production use remains blocked.
 */
export class CapabilityCatalogService {
  readonly #subject: CapabilitySubjectRef;
  readonly #currentAuthority: (subject: CapabilitySubjectRef) => RuntimeAuthoritySnapshot | null;
  readonly #transitionAuthority: (
    subject: CapabilitySubjectRef,
    evidenceRef: string,
  ) => HostTransitionEvidence | null;
  readonly #grantAuthority: (
    subject: CapabilitySubjectRef,
    grantRef: string,
  ) => PassiveObservationGrant | null;
  readonly #sourceAuthority: (
    subject: CapabilitySubjectRef,
    sourceRef: string,
    expectedKind: CapabilitySourceKind,
  ) => CapabilitySource | null;
  readonly #retiredQualificationSources = new Map<string, CapabilitySource>();
  readonly #retiredObservationSources = new Map<string, CapabilitySource>();
  #state!: CatalogState;

  constructor(adapters: CapabilityAuthorityAdapters, seedInput: CapabilityCatalogSeed) {
    if (
      typeof adapters?.readyHost?.current !== "function" ||
      typeof adapters.readyHost.transition !== "function" ||
      typeof adapters?.grants?.resolvePassiveGrant !== "function" ||
      typeof adapters?.sources?.resolveSource !== "function"
    ) {
      throw new CapabilityCatalogError(
        "AUTHORITY_UNAVAILABLE",
        "all captured authority adapters are required",
      );
    }
    this.#currentAuthority = adapters.readyHost.current.bind(adapters.readyHost);
    this.#transitionAuthority = adapters.readyHost.transition.bind(adapters.readyHost);
    this.#grantAuthority = adapters.grants.resolvePassiveGrant.bind(adapters.grants);
    this.#sourceAuthority = adapters.sources.resolveSource.bind(adapters.sources);

    const seed = exactRecord(seedInput, "seed", [
      "driverId",
      "runtimeInstanceId",
      "manifestRevision",
    ]);
    this.#subject = Object.freeze({
      driverId: canonicalString(seed.driverId, "seed.driverId") as CapabilitySubjectRef["driverId"],
      runtimeInstanceId: canonicalString(
        seed.runtimeInstanceId,
        "seed.runtimeInstanceId",
      ) as CapabilitySubjectRef["runtimeInstanceId"],
    });
    const authority = this.#requireCurrentAuthority();
    const manifestRevision = canonicalU64(seed.manifestRevision, "seed.manifestRevision");
    const manifestSource = this.#mintSource(
      {
        kind: "manifest",
        ref: authority.snapshot.target.driver.manifest.source.provenanceRef,
        revision: manifestRevision,
      },
      "manifest",
    );
    const declared = authority.snapshot.target.driver.manifest.declaredCapabilities.map((claim) =>
      Object.freeze({
        name: canonicalString(claim.name, "manifest.capability.name"),
        support: claim.support,
        source: manifestSource,
        ...(claim.constraints === undefined
          ? {}
          : { constraints: passiveSnapshot(claim.constraints, "manifest.constraints") }),
      }),
    );
    if (new Set(declared.map((entry) => entry.name)).size !== declared.length) {
      return invalid("manifest capabilities must be unique");
    }
    this.#state = {
      currentAuthority: authority.snapshot,
      currentAuthorityDigest: authority.authorityDigest,
      currentIdentity: authority.identity,
      currentSourceEpoch: authority.sourceEpoch,
      sourceEpochHighWater: BigInt(authority.sourceEpoch),
      generationHighWater: BigInt(authority.identity.generation),
      authRevisionHighWater: BigInt(authority.identity.authRevision),
      declared: Object.freeze(declared),
      observations: Object.freeze([]),
      qualified: Object.freeze([]),
      qualifiedCapacity: Object.freeze([]),
      retiredQualificationKeys: new Set(),
      retiredObservationDigests: new Set(),
      retiredProfiles: new Set(),
      retiredAccounts: new Set(),
      usedTransitionEvidence: new Set(),
    };
  }

  view(): CapabilityCatalogView {
    return Object.freeze({
      subject: this.#subject,
      currentIdentity: this.#state.currentIdentity,
      currentSourceEpoch: this.#state.currentSourceEpoch,
      declared: Object.freeze([...this.#state.declared]),
      observations: Object.freeze([...this.#state.observations]),
      qualified: Object.freeze([...this.#state.qualified]),
      qualifiedCapacity: Object.freeze([...this.#state.qualifiedCapacity]),
      retiredQualificationKeys: Object.freeze([...this.#state.retiredQualificationKeys]),
      retiredObservationDigests: Object.freeze([...this.#state.retiredObservationDigests]),
    });
  }

  transition(evidenceRefInput: string): CapabilityCatalogView {
    const evidenceRef = canonicalString(evidenceRefInput, "evidenceRef");
    if (this.#state.usedTransitionEvidence.has(evidenceRef)) {
      throw new CapabilityCatalogError("AUTHORITY_REPLAY", "transition evidence was already used");
    }
    const evidenceRaw = this.#transitionAuthority(this.#subject, evidenceRef);
    if (evidenceRaw === null) {
      throw new CapabilityCatalogError(
        "TRANSITION_EVIDENCE_INVALID",
        "trusted host authority did not resolve transition evidence",
      );
    }
    const evidence = passiveSnapshot(evidenceRaw, "transitionEvidence");
    if (evidence.evidenceRef !== evidenceRef) {
      throw new CapabilityCatalogError(
        "TRANSITION_EVIDENCE_INVALID",
        "authority resolved a different transition evidence ref",
      );
    }
    const previous = authorityFacts(evidence.previous, this.#subject);
    const next = authorityFacts(evidence.next, this.#subject);
    if (previous.authorityDigest !== this.#state.currentAuthorityDigest) {
      throw new CapabilityCatalogError(
        "TRANSITION_EVIDENCE_INVALID",
        "transition previous state is not the service's current exact authority",
      );
    }
    const observedCurrent = this.#requireCurrentAuthority();
    if (observedCurrent.authorityDigest !== next.authorityDigest) {
      throw new CapabilityCatalogError(
        "TRANSITION_EVIDENCE_INVALID",
        "transition next state is not the current exact ReadyHost authority",
      );
    }
    const nextSourceEpoch = BigInt(next.sourceEpoch);
    const nextGeneration = BigInt(next.identity.generation);
    const nextAuthRevision = BigInt(next.identity.authRevision);
    if (
      nextSourceEpoch <= this.#state.sourceEpochHighWater ||
      nextGeneration < this.#state.generationHighWater ||
      nextAuthRevision < this.#state.authRevisionHighWater
    ) {
      throw new CapabilityCatalogError(
        "AUTHORITY_ROLLBACK",
        "sourceEpoch must advance and generation/authRevision must not fall below high-water",
      );
    }
    const oldKey = qualificationKey(this.#state.currentIdentity);
    const nextKey = qualificationKey(next.identity);
    if (this.#state.retiredQualificationKeys.has(nextKey)) {
      throw new CapabilityCatalogError(
        "AUTHORITY_ROLLBACK",
        "a retired qualification identity cannot become current again",
      );
    }
    const oldProfile = this.#state.currentIdentity.profileRef;
    const nextProfile = next.identity.profileRef;
    if (oldProfile !== nextProfile && this.#state.retiredProfiles.has(nextProfile)) {
      throw new CapabilityCatalogError("AUTHORITY_ROLLBACK", "retired profile cannot revive");
    }
    const oldAccount = this.#state.currentIdentity.accountRef ?? "<null>";
    const nextAccount = next.identity.accountRef ?? "<null>";
    if (oldAccount !== nextAccount && this.#state.retiredAccounts.has(nextAccount)) {
      throw new CapabilityCatalogError("AUTHORITY_ROLLBACK", "retired account cannot revive");
    }
    this.#state.retiredQualificationKeys.add(oldKey);
    if (oldProfile !== nextProfile) this.#state.retiredProfiles.add(oldProfile);
    if (oldAccount !== nextAccount) this.#state.retiredAccounts.add(oldAccount);
    this.#state.usedTransitionEvidence.add(evidenceRef);
    this.#state.currentAuthority = next.snapshot;
    this.#state.currentAuthorityDigest = next.authorityDigest;
    this.#state.currentIdentity = next.identity;
    this.#state.currentSourceEpoch = next.sourceEpoch;
    this.#state.sourceEpochHighWater = nextSourceEpoch;
    this.#state.generationHighWater =
      nextGeneration > this.#state.generationHighWater
        ? nextGeneration
        : this.#state.generationHighWater;
    this.#state.authRevisionHighWater =
      nextAuthRevision > this.#state.authRevisionHighWater
        ? nextAuthRevision
        : this.#state.authRevisionHighWater;
    return this.view();
  }

  observe(requestInput: PassiveObservationRequest): CapabilityCatalogView {
    this.#assertExactCurrentAuthority();
    const request = exactRecord(requestInput, "observation", [
      "observationRevision",
      "grantRef",
      "capabilities",
      "capacity",
    ]);
    const observationRevision = canonicalU64(
      request.observationRevision,
      "observation.observationRevision",
    );
    const grantRef = canonicalString(request.grantRef, "observation.grantRef");
    if (!Array.isArray(request.capabilities)) {
      return invalid("observation.capabilities must be an array");
    }
    const capabilities = request.capabilities.map((entry, index) =>
      this.#observationCapability(entry, `observation.capabilities[${index}]`),
    );
    if (new Set(capabilities.map((entry) => entry.name)).size !== capabilities.length) {
      return invalid("observation capabilities must be unique");
    }
    const requiredLevel = capabilities.some((entry) => entry.requiredLevel === "L2") ? "L2" : "L1";
    this.#grant(grantRef, requiredLevel);
    const capacity = this.#capacity(request.capacity as CapacityInput, "observation.capacity");
    const digestInput = passiveSnapshot(
      {
        identity: this.#state.currentIdentity,
        observationRevision,
        capabilities,
        capacity,
      } as unknown as JsonValue,
      "observation.digestInput",
    );
    const observationDigest = contentDigest(digestInput);
    const sameRevision = this.#state.observations.find(
      (entry) =>
        sameIdentity(entry.identity, this.#state.currentIdentity) &&
        entry.observationRevision === observationRevision,
    );
    if (sameRevision !== undefined) {
      if (sameRevision.observationDigest === observationDigest) return this.view();
      throw new CapabilityCatalogError(
        "OBSERVATION_REVISION_CONFLICT",
        "same identity and observation revision have different content digests",
      );
    }
    const prior = this.#state.observations.filter((entry) =>
      sameIdentity(entry.identity, this.#state.currentIdentity),
    );
    if (prior.some((entry) => BigInt(entry.observationRevision) > BigInt(observationRevision))) {
      throw new CapabilityCatalogError(
        "OBSERVATION_REVISION_CONFLICT",
        "observation revision cannot move backward",
      );
    }
    const replacedDigests = prior.map((entry) => entry.observationDigest);
    for (const replaced of replacedDigests) this.#state.retiredObservationDigests.add(replaced);
    const batch: PassiveObservationBatch = Object.freeze({
      identity: this.#state.currentIdentity,
      observationRevision,
      observationDigest,
      capabilities: Object.freeze(capabilities),
      capacity,
    });
    this.#state.observations = Object.freeze([...this.#state.observations, batch]);
    this.#invalidateObservationDigests(replacedDigests);
    return this.view();
  }

  qualify(requestInput: QualificationRequest): CapabilityCatalogView {
    this.#assertExactCurrentAuthority();
    const request = exactRecord(requestInput, "qualification", [
      "observationRevision",
      "grantRef",
      "sourceRef",
      "validUntilEpochMs",
    ]);
    const observationRevision = canonicalU64(
      request.observationRevision,
      "qualification.observationRevision",
    );
    const observation = this.#state.observations.find(
      (entry) =>
        sameIdentity(entry.identity, this.#state.currentIdentity) &&
        entry.observationRevision === observationRevision &&
        !this.#state.retiredObservationDigests.has(entry.observationDigest),
    );
    if (observation === undefined) {
      throw new CapabilityCatalogError(
        "OBSERVATION_NOT_FOUND",
        "qualification must consume an exact current passive observation",
      );
    }
    this.#assertObservationDigest(observation);
    const requiredLevel = observation.capabilities.some((entry) => entry.requiredLevel === "L2")
      ? "L2"
      : "L1";
    const grant = this.#grant(
      canonicalString(request.grantRef, "qualification.grantRef"),
      requiredLevel,
    );
    const qualificationSource = this.#source(
      canonicalString(request.sourceRef, "qualification.sourceRef"),
      "qualification",
    );
    const validUntilEpochMs = epochMs(request.validUntilEpochMs, "qualification.validUntilEpochMs");
    const key = qualificationKey(this.#state.currentIdentity);
    if (this.#state.retiredQualificationKeys.has(key)) {
      throw new CapabilityCatalogError(
        "AUTHORITY_ROLLBACK",
        "retired identity cannot receive a new qualification",
      );
    }
    const qualified = observation.capabilities.map((entry) => {
      const record: QualifiedCapability = Object.freeze({
        name: entry.name,
        support: entry.support,
        source: qualificationSource,
        ...(entry.constraints === undefined ? {} : { constraints: entry.constraints }),
        qualificationKey: key,
        observationRevision,
        observationDigest: observation.observationDigest,
        authorization: grant,
        validUntilEpochMs,
      });
      trustedQualifications.add(record);
      return record;
    });
    const capacity: QualifiedCapacity = Object.freeze({
      qualificationKey: key,
      observationRevision,
      observationDigest: observation.observationDigest,
      authorization: grant,
      validUntilEpochMs,
      value: Object.freeze({ ...observation.capacity, source: qualificationSource }),
    });
    trustedQualifications.add(capacity);
    this.#state.qualified = Object.freeze([
      ...this.#state.qualified.filter((entry) => entry.qualificationKey !== key),
      ...qualified,
    ]);
    this.#state.qualifiedCapacity = Object.freeze([
      ...this.#state.qualifiedCapacity.filter((entry) => entry.qualificationKey !== key),
      capacity,
    ]);
    return this.view();
  }

  revokeObservation(requestInput: ObservationRevocationRequest): CapabilityCatalogView {
    const request = exactRecord(requestInput, "observationRevocation", [
      "observationDigest",
      "sourceRef",
    ]);
    const observationDigest = digest(
      request.observationDigest,
      "observationRevocation.observationDigest",
    );
    const revocationSource = this.#source(
      canonicalString(request.sourceRef, "observationRevocation.sourceRef"),
      "revocation",
    );
    this.#state.retiredObservationDigests.add(observationDigest);
    this.#retiredObservationSources.set(observationDigest, revocationSource);
    this.#invalidateObservationDigests([observationDigest]);
    return this.view();
  }

  revokeQualification(requestInput: QualificationRevocationRequest): CapabilityCatalogView {
    const request = exactRecord(requestInput, "qualificationRevocation", [
      "qualificationKey",
      "sourceRef",
    ]);
    const key = canonicalString(request.qualificationKey, "qualificationRevocation.key");
    const revocationSource = this.#source(
      canonicalString(request.sourceRef, "qualificationRevocation.sourceRef"),
      "revocation",
    );
    this.#state.retiredQualificationKeys.add(key);
    this.#retiredQualificationSources.set(key, revocationSource);
    this.#state.qualified = Object.freeze(
      this.#state.qualified.filter((entry) => entry.qualificationKey !== key),
    );
    this.#state.qualifiedCapacity = Object.freeze(
      this.#state.qualifiedCapacity.filter((entry) => entry.qualificationKey !== key),
    );
    return this.view();
  }

  resolve(nameInput: string, nowEpochMsInput: number): CapabilityResolution {
    const name = canonicalString(nameInput, "name");
    const nowEpochMs = epochMs(nowEpochMsInput, "nowEpochMs");
    const key = qualificationKey(this.#state.currentIdentity);
    if (!this.#isExactCurrentAuthority()) {
      return this.#unknown(name, key, "HOST_TRANSITION_REQUIRED", POLICY_SOURCE);
    }
    const retiredSource = this.#retiredQualificationSources.get(key);
    if (this.#state.retiredQualificationKeys.has(key)) {
      return this.#unknown(name, key, "REVOKED", retiredSource ?? POLICY_SOURCE);
    }
    const qualification = this.#state.qualified.find(
      (entry) => entry.qualificationKey === key && entry.name === name,
    );
    if (
      qualification === undefined ||
      !trustedQualifications.has(qualification) ||
      !trustedSources.has(qualification.source)
    ) {
      const declared = this.#state.declared.find((entry) => entry.name === name);
      return Object.freeze({
        name,
        support: "unknown",
        reason: "NOT_QUALIFIED",
        sources: frozenSources(...(declared === undefined ? [] : [declared.source]), POLICY_SOURCE),
        qualificationKey: key,
      });
    }
    if (!this.#qualificationObservationCurrent(qualification)) {
      return this.#unknown(
        name,
        key,
        "OBSERVATION_REPLACED",
        qualification.source,
        this.#retiredObservationSources.get(qualification.observationDigest) ?? POLICY_SOURCE,
      );
    }
    if (nowEpochMs > qualification.validUntilEpochMs) {
      return this.#unknown(name, key, "EXPIRED", qualification.source, POLICY_SOURCE);
    }
    return Object.freeze({
      name,
      support: qualification.support,
      reason:
        qualification.support === "supported"
          ? "QUALIFIED"
          : qualification.support === "unsupported"
            ? "UNSUPPORTED"
            : "QUALIFIED_UNKNOWN",
      sources: frozenSources(qualification.source),
      qualificationKey: key,
    });
  }

  capacity(nowEpochMsInput: number): CapacityState {
    const nowEpochMs = epochMs(nowEpochMsInput, "nowEpochMs");
    const key = qualificationKey(this.#state.currentIdentity);
    if (!this.#isExactCurrentAuthority() || this.#state.retiredQualificationKeys.has(key)) {
      return Object.freeze({ status: "unknown", source: POLICY_SOURCE });
    }
    const capacity = this.#state.qualifiedCapacity.find((entry) => entry.qualificationKey === key);
    if (
      capacity === undefined ||
      !trustedQualifications.has(capacity) ||
      !trustedSources.has(capacity.value.source) ||
      nowEpochMs > capacity.validUntilEpochMs ||
      !this.#qualificationObservationCurrent(capacity)
    ) {
      return Object.freeze({ status: "unknown", source: POLICY_SOURCE });
    }
    return capacity.value;
  }

  begin(
    bindingIdInput: string,
    requiredCapabilitiesInput: ReadonlyArray<string>,
    nowEpochMs: number,
  ): InFlightCapabilityBinding {
    this.#assertExactCurrentAuthority();
    const bindingId = canonicalString(bindingIdInput, "bindingId");
    const names = requiredCapabilitiesInput.map((entry, index) =>
      canonicalString(entry, `requiredCapabilities[${index}]`),
    );
    if (new Set(names).size !== names.length) return invalid("requiredCapabilities has duplicates");
    const capabilities = Object.freeze(names.map((name) => this.resolve(name, nowEpochMs)));
    const denied = capabilities.find((entry) => entry.support !== "supported");
    if (denied !== undefined) {
      throw new CapabilityCatalogError(
        "CAPABILITY_NOT_SUPPORTED",
        `${denied.name} is ${denied.support} (${denied.reason})`,
      );
    }
    const key = qualificationKey(this.#state.currentIdentity);
    const qualification = this.#state.qualified.find(
      (entry) => entry.qualificationKey === key && entry.name === names[0],
    );
    if (qualification === undefined || !this.#qualificationObservationCurrent(qualification)) {
      throw new CapabilityCatalogError(
        "CAPABILITY_NOT_SUPPORTED",
        "binding has no current observation digest",
      );
    }
    const binding = Object.freeze({
      bindingId,
      qualificationKey: key,
      observationDigest: qualification.observationDigest,
      runtimeInstanceId: this.#state.currentIdentity.runtimeInstanceId,
      generation: this.#state.currentIdentity.generation,
      capabilities,
      capacity: this.capacity(nowEpochMs),
    });
    authenticBindings.add(binding);
    return binding;
  }

  snapshotBinding(binding: InFlightCapabilityBinding): InFlightCapabilityBinding {
    if (!authenticBindings.has(binding)) {
      throw new CapabilityCatalogError("UNTRUSTED_BINDING", "binding must originate here");
    }
    return binding;
  }

  #requireCurrentAuthority() {
    const value = this.#currentAuthority(this.#subject);
    if (value === null) {
      throw new CapabilityCatalogError(
        "AUTHORITY_UNAVAILABLE",
        "trusted ReadyHost authority has no current subject",
      );
    }
    return authorityFacts(value, this.#subject);
  }

  #isExactCurrentAuthority(): boolean {
    try {
      return this.#requireCurrentAuthority().authorityDigest === this.#state.currentAuthorityDigest;
    } catch {
      return false;
    }
  }

  #assertExactCurrentAuthority(): void {
    if (!this.#isExactCurrentAuthority()) {
      throw new CapabilityCatalogError(
        "HOST_TRANSITION_REQUIRED",
        "trusted ReadyHost changed; apply trusted transition evidence before new authority work",
      );
    }
  }

  #mintSource(value: CapabilitySource, expectedKind: CapabilitySourceKind): CapabilitySource {
    return normalizeSource(value, "source", expectedKind);
  }

  #source(sourceRef: string, expectedKind: CapabilitySourceKind): CapabilitySource {
    const raw = this.#sourceAuthority(this.#subject, sourceRef, expectedKind);
    if (raw === null) {
      throw new CapabilityCatalogError(
        "AUTHORIZATION_REQUIRED",
        `trusted source authority did not resolve ${sourceRef}`,
      );
    }
    const result = normalizeSource(raw, "source", expectedKind);
    if (result.ref !== sourceRef) return invalid("source authority returned the wrong source ref");
    return result;
  }

  #grant(grantRef: string, requiredLevel: ObservationLevel): PassiveObservationGrant {
    const raw = this.#grantAuthority(this.#subject, grantRef);
    if (raw === null) {
      throw new CapabilityCatalogError(
        "AUTHORIZATION_REQUIRED",
        `trusted grant authority did not resolve ${grantRef}`,
      );
    }
    const grant = normalizeGrant(raw, grantRef);
    if (LEVEL_RANK[grant.level] < LEVEL_RANK[requiredLevel]) {
      throw new CapabilityCatalogError(
        "AUTHORIZATION_INSUFFICIENT",
        `${grant.level} cannot accept ${requiredLevel} evidence`,
      );
    }
    return grant;
  }

  #observationCapability(value: CapabilityClaimInput, path: string): PassiveCapabilityObservation {
    const raw = exactRecord(
      value,
      path,
      ["name", "support", "requiredLevel", "sourceRef"],
      ["constraints"],
    );
    if (raw.support !== "supported" && raw.support !== "unsupported" && raw.support !== "unknown") {
      return invalid(`${path}.support is invalid`);
    }
    if (raw.requiredLevel !== "L1" && raw.requiredLevel !== "L2") {
      return invalid(`${path}.requiredLevel is invalid`);
    }
    return Object.freeze({
      name: canonicalString(raw.name, `${path}.name`),
      support: raw.support,
      requiredLevel: raw.requiredLevel,
      source: this.#source(
        canonicalString(raw.sourceRef, `${path}.sourceRef`),
        "passive-observation",
      ),
      ...(raw.constraints === undefined
        ? {}
        : {
            constraints: passiveSnapshot(raw.constraints as JsonValue, `${path}.constraints`),
          }),
    });
  }

  #capacity(value: CapacityInput, path: string): CapacityState {
    const raw = exactRecord(value, path, ["status", "sourceRef"], ["available", "unit"]);
    const source = this.#source(
      canonicalString(raw.sourceRef, `${path}.sourceRef`),
      "passive-observation",
    );
    if (raw.status === "unknown") {
      if (raw.available !== undefined || raw.unit !== undefined) {
        return invalid(`${path} unknown capacity cannot carry a numeric value`);
      }
      return Object.freeze({ status: "unknown", source });
    }
    if (raw.status !== "known") return invalid(`${path}.status is invalid`);
    return Object.freeze({
      status: "known",
      available: canonicalU64(raw.available, `${path}.available`),
      unit: canonicalString(raw.unit, `${path}.unit`),
      source,
    });
  }

  #assertObservationDigest(observation: PassiveObservationBatch): void {
    if (
      observation.capabilities.some((entry) => !trustedSources.has(entry.source)) ||
      !trustedSources.has(observation.capacity.source)
    ) {
      throw new CapabilityCatalogError(
        "OBSERVATION_REVISION_CONFLICT",
        "observation contains a source not minted by the captured authority port",
      );
    }
    const recalculated = contentDigest({
      identity: observation.identity,
      observationRevision: observation.observationRevision,
      capabilities: observation.capabilities,
      capacity: observation.capacity,
    } as unknown as JsonValue);
    if (recalculated !== observation.observationDigest) {
      throw new CapabilityCatalogError(
        "OBSERVATION_REVISION_CONFLICT",
        "observation content no longer matches its canonical digest",
      );
    }
  }

  #qualificationObservationCurrent(
    qualification: Pick<
      QualifiedCapability,
      "authorization" | "observationDigest" | "observationRevision"
    >,
  ): boolean {
    if (!trustedGrants.has(qualification.authorization)) return false;
    if (this.#state.retiredObservationDigests.has(qualification.observationDigest)) return false;
    const observation = this.#state.observations.find(
      (entry) =>
        entry.observationDigest === qualification.observationDigest &&
        entry.observationRevision === qualification.observationRevision &&
        sameIdentity(entry.identity, this.#state.currentIdentity),
    );
    if (observation === undefined) return false;
    this.#assertObservationDigest(observation);
    return true;
  }

  #invalidateObservationDigests(digests: ReadonlyArray<string>): void {
    const replaced = new Set(digests);
    if (replaced.size === 0) return;
    this.#state.qualified = Object.freeze(
      this.#state.qualified.filter((entry) => !replaced.has(entry.observationDigest)),
    );
    this.#state.qualifiedCapacity = Object.freeze(
      this.#state.qualifiedCapacity.filter((entry) => !replaced.has(entry.observationDigest)),
    );
  }

  #unknown(
    name: string,
    qualificationKeyValue: string,
    reason: CapabilityResolution["reason"],
    ...sources: ReadonlyArray<CapabilitySource>
  ): CapabilityResolution {
    return Object.freeze({
      name,
      support: "unknown",
      reason,
      sources: frozenSources(...sources),
      qualificationKey: qualificationKeyValue,
    });
  }
}
