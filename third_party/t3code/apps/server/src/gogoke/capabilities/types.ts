import type { JsonValue, RuntimeDriverId, RuntimeInstanceId } from "../contracts/model.ts";
import type { ReadyHost } from "../host/coordinator.ts";
import type { CapabilitySupport, RuntimeExecutionTarget } from "../runtimeCatalog/types.ts";

export const CAPABILITY_SOURCE_KINDS = [
  "manifest",
  "passive-observation",
  "qualification",
  "revocation",
  "policy",
] as const;
export type CapabilitySourceKind = (typeof CAPABILITY_SOURCE_KINDS)[number];

export interface CapabilitySource {
  readonly kind: CapabilitySourceKind;
  readonly ref: string;
  readonly revision: string;
}

export interface SourcedCapability {
  readonly name: string;
  readonly support: CapabilitySupport;
  readonly source: CapabilitySource;
  readonly constraints?: JsonValue;
}

export type ObservationLevel = "L1" | "L2";

/** Authority output supplied only by the service's captured authority port. */
export interface PassiveObservationGrant {
  readonly level: ObservationLevel;
  readonly grantRef: string;
  readonly grantRevision: string;
  readonly mode: "passive-only";
}

export interface QualificationEnvironment {
  readonly binaryDigest: string;
  readonly nativeDigest: string;
  readonly platform: string;
  readonly runtimeMode: string;
  readonly isolationProfile: string;
  readonly toolProfile: string;
}

export interface QualificationIdentity {
  readonly driverId: RuntimeDriverId;
  readonly binaryDigest: string;
  readonly adapterDigest: string;
  readonly nativeDigest: string;
  readonly platform: string;
  readonly profileRef: string;
  readonly accountRef: string | null;
  readonly authRevision: string;
  readonly runtimeInstanceId: RuntimeInstanceId;
  readonly nativeModelId: string;
  readonly resolvedModelVersion: string;
  readonly runtimeMode: string;
  readonly isolationProfile: string;
  readonly toolProfile: string;
  readonly generation: string;
  readonly sourceEpoch: string;
}

export interface RuntimeAuthoritySnapshot {
  readonly target: RuntimeExecutionTarget;
  readonly readyHost: ReadyHost;
  readonly environment: QualificationEnvironment;
}

export interface HostTransitionEvidence {
  readonly evidenceRef: string;
  readonly previous: RuntimeAuthoritySnapshot;
  readonly next: RuntimeAuthoritySnapshot;
}

export interface CapabilitySubjectRef {
  readonly driverId: RuntimeDriverId;
  readonly runtimeInstanceId: RuntimeInstanceId;
}

/** Captured once by CapabilityCatalogService; never accepted per call. */
export interface ReadyHostAuthorityPort {
  current(subject: CapabilitySubjectRef): RuntimeAuthoritySnapshot | null;
  transition(subject: CapabilitySubjectRef, evidenceRef: string): HostTransitionEvidence | null;
}

/** Captured once by CapabilityCatalogService; callers supply only a grant ref. */
export interface CapabilityGrantAuthorityPort {
  resolvePassiveGrant(
    subject: CapabilitySubjectRef,
    grantRef: string,
  ): PassiveObservationGrant | null;
}

/** Captured once; source objects are not accepted from request callers. */
export interface QualificationSourcePort {
  resolveSource(
    subject: CapabilitySubjectRef,
    sourceRef: string,
    expectedKind: CapabilitySourceKind,
  ): CapabilitySource | null;
}

export interface CapabilityAuthorityAdapters {
  readonly readyHost: ReadyHostAuthorityPort;
  readonly grants: CapabilityGrantAuthorityPort;
  readonly sources: QualificationSourcePort;
}

export interface CapabilityCatalogSeed {
  readonly driverId: RuntimeDriverId;
  readonly runtimeInstanceId: RuntimeInstanceId;
  readonly manifestRevision: string;
}

export interface CapabilityClaimInput {
  readonly name: string;
  readonly support: CapabilitySupport;
  readonly requiredLevel: ObservationLevel;
  readonly sourceRef: string;
  readonly constraints?: JsonValue;
}

export type CapacityInput =
  | {
      readonly status: "known";
      readonly available: string;
      readonly unit: string;
      readonly sourceRef: string;
    }
  | {
      readonly status: "unknown";
      readonly sourceRef: string;
    };

export type CapacityState =
  | {
      readonly status: "known";
      readonly available: string;
      readonly unit: string;
      readonly source: CapabilitySource;
    }
  | {
      readonly status: "unknown";
      readonly source: CapabilitySource;
    };

export interface PassiveObservationRequest {
  readonly observationRevision: string;
  readonly grantRef: string;
  readonly capabilities: ReadonlyArray<CapabilityClaimInput>;
  readonly capacity: CapacityInput;
}

export interface QualificationRequest {
  readonly observationRevision: string;
  readonly grantRef: string;
  readonly sourceRef: string;
  readonly validUntilEpochMs: number;
}

export interface ObservationRevocationRequest {
  readonly observationDigest: string;
  readonly sourceRef: string;
}

export interface QualificationRevocationRequest {
  readonly qualificationKey: string;
  readonly sourceRef: string;
}

export interface PassiveCapabilityObservation extends SourcedCapability {
  readonly requiredLevel: ObservationLevel;
}

export interface PassiveObservationBatch {
  readonly identity: QualificationIdentity;
  readonly observationRevision: string;
  readonly observationDigest: string;
  readonly capabilities: ReadonlyArray<PassiveCapabilityObservation>;
  readonly capacity: CapacityState;
}

export interface QualifiedCapability extends SourcedCapability {
  readonly qualificationKey: string;
  readonly observationRevision: string;
  readonly observationDigest: string;
  readonly authorization: PassiveObservationGrant;
  readonly validUntilEpochMs: number;
}

export interface QualifiedCapacity {
  readonly qualificationKey: string;
  readonly observationRevision: string;
  readonly observationDigest: string;
  readonly authorization: PassiveObservationGrant;
  readonly validUntilEpochMs: number;
  readonly value: CapacityState;
}

export interface CapabilityCatalogView {
  readonly subject: CapabilitySubjectRef;
  readonly currentIdentity: QualificationIdentity;
  readonly currentSourceEpoch: string;
  readonly declared: ReadonlyArray<SourcedCapability>;
  readonly observations: ReadonlyArray<PassiveObservationBatch>;
  readonly qualified: ReadonlyArray<QualifiedCapability>;
  readonly qualifiedCapacity: ReadonlyArray<QualifiedCapacity>;
  readonly retiredQualificationKeys: ReadonlyArray<string>;
  readonly retiredObservationDigests: ReadonlyArray<string>;
}

export type CapabilityResolutionReason =
  | "QUALIFIED"
  | "UNSUPPORTED"
  | "QUALIFIED_UNKNOWN"
  | "NOT_QUALIFIED"
  | "HOST_TRANSITION_REQUIRED"
  | "EXPIRED"
  | "REVOKED"
  | "OBSERVATION_REPLACED";

export interface CapabilityResolution {
  readonly name: string;
  readonly support: CapabilitySupport;
  readonly reason: CapabilityResolutionReason;
  readonly sources: ReadonlyArray<CapabilitySource>;
  readonly qualificationKey: string;
}

export interface InFlightCapabilityBinding {
  readonly bindingId: string;
  readonly qualificationKey: string;
  readonly observationDigest: string;
  readonly runtimeInstanceId: RuntimeInstanceId;
  readonly generation: string;
  readonly capabilities: ReadonlyArray<CapabilityResolution>;
  readonly capacity: CapacityState;
}

export type CapabilityCatalogErrorCode =
  | "INVALID_INPUT"
  | "AUTHORITY_UNAVAILABLE"
  | "SUBJECT_MISMATCH"
  | "AUTHORIZATION_REQUIRED"
  | "AUTHORIZATION_INSUFFICIENT"
  | "HOST_TRANSITION_REQUIRED"
  | "TRANSITION_EVIDENCE_INVALID"
  | "AUTHORITY_ROLLBACK"
  | "AUTHORITY_REPLAY"
  | "OBSERVATION_NOT_FOUND"
  | "OBSERVATION_REVISION_CONFLICT"
  | "OBSERVATION_REVOKED"
  | "CAPABILITY_NOT_SUPPORTED"
  | "UNTRUSTED_BINDING";

export class CapabilityCatalogError extends Error {
  override readonly name = "CapabilityCatalogError";
  readonly code: CapabilityCatalogErrorCode;

  constructor(code: CapabilityCatalogErrorCode, detail: string) {
    super(`${code}: ${detail}`);
    this.code = code;
  }
}
