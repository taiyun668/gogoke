import type {
  JsonObject,
  JsonValue,
  ModelRef,
  RuntimeAccountRef,
  RuntimeDriver,
  RuntimeDriverId,
  RuntimeInstance,
  RuntimeInstanceId,
  Seat,
} from "../contracts/model.ts";

export const ADAPTER_MANIFEST_SCHEMA = "gogoke.adapter-manifest.v1" as const;
export const RUNTIME_INSTANCE_CONFIG_SCHEMA = "gogoke.runtime-instance-config.v1" as const;

export const CAPABILITY_SUPPORT = ["supported", "unsupported", "unknown"] as const;
export type CapabilitySupport = (typeof CAPABILITY_SUPPORT)[number];

export const KNOWN_CAPABILITY_NAMES = [
  "prompt",
  "steer",
  "followUp",
  "interrupt",
  "close",
  "resume",
  "fork",
  "compaction",
  "image",
  "tools",
  "approval",
  "question",
  "usage",
  "eventReplay",
  "sandbox",
  "descendants",
] as const;

export interface CapabilityClaim {
  readonly name: string;
  readonly support: CapabilitySupport;
  readonly constraints?: JsonValue;
}

export interface AdapterManifestV1 {
  readonly schema: typeof ADAPTER_MANIFEST_SCHEMA;
  readonly packageId: string;
  readonly driverId: RuntimeDriverId;
  readonly adapterVersion: string;
  readonly artifactDigest: string;
  readonly hostApiRange: {
    readonly minInclusive: string;
    readonly maxExclusive: string;
  };
  readonly nativeProtocol: string;
  readonly platforms: ReadonlyArray<string>;
  readonly configSchemaRef: string;
  readonly declaredCapabilities: ReadonlyArray<CapabilityClaim>;
  readonly requiredHostServices: ReadonlyArray<string>;
  readonly requestedEffects: ReadonlyArray<string>;
  readonly source: {
    readonly kind: "bundled";
    readonly provenanceRef: string;
  };
  readonly license: {
    readonly spdxId: string;
    readonly noticeRef: string;
  };
  readonly admissionRef: string;
}

export interface AdapterRegistration {
  /**
   * Registrations are trusted declarations assembled by this build. They do
   * not contain executable code, download locations, or lifecycle hooks.
   */
  readonly manifest: AdapterManifestV1;
}

export interface RuntimeCatalogHostContext {
  readonly hostApiVersion: string;
  readonly hostPlatform: string;
  readonly availableHostServices: ReadonlySet<string>;
  readonly allowedEffects: ReadonlySet<string>;
  readonly admittedAdmissionRefs: ReadonlySet<string>;
}

export interface RuntimeDriverRecord {
  readonly driver: RuntimeDriver;
  readonly manifest: AdapterManifestV1;
}

export interface RuntimeInstanceRecord {
  readonly instance: RuntimeInstance;
  readonly adapterVersion: string;
  readonly accountRef?: RuntimeAccountRef;
  readonly enabled: boolean;
  readonly config: JsonValue;
}

export interface RuntimeExecutionTarget {
  readonly driver: RuntimeDriverRecord;
  readonly instance: RuntimeInstanceRecord;
  readonly model: ModelRef;
  readonly seat: Seat;
}

export interface RuntimeInstanceConfig {
  readonly instanceId: RuntimeInstanceId;
  readonly driverId: RuntimeDriverId;
  readonly adapterVersion: string;
  readonly enabled?: boolean;
  readonly accountRef?: RuntimeAccountRef;
  readonly config: JsonValue;
}

export interface DecodedRuntimeInstanceConfig {
  readonly value: RuntimeInstanceConfig;
  readonly unknownFields: JsonObject;
}

export type RuntimeAvailability =
  | {
      readonly status: "available";
      readonly registration: AdapterRegistration;
    }
  | {
      readonly status: "unavailable";
      readonly reason: "DRIVER_NOT_REGISTERED" | "ADAPTER_VERSION_NOT_REGISTERED";
    };

export interface ResolvedRuntimeInstanceConfig {
  readonly decoded: DecodedRuntimeInstanceConfig;
  readonly enabled: boolean;
  readonly availability: RuntimeAvailability;
}

export interface AdapterIdentity {
  readonly driverId: RuntimeDriverId;
  readonly adapterVersion: string;
  readonly artifactDigest: string;
}

export type RuntimeInstancePhase = "disabled" | "ready" | "draining" | "unavailable";

export interface RuntimeInstanceLeaseState {
  readonly instanceId: RuntimeInstanceId;
  readonly driverId: RuntimeDriverId;
  readonly enabled: boolean;
  readonly phase: RuntimeInstancePhase;
  readonly currentAdapter: AdapterIdentity | null;
  readonly pendingAdapter: AdapterIdentity | null;
  readonly activeBindings: ReadonlyArray<string>;
}

export interface QualificationKeyInput {
  readonly driverId: RuntimeDriverId;
  readonly adapterDigest: string;
  readonly nativeDigest: string;
  readonly platform: string;
  readonly profileRef: string;
  readonly authRevision: string;
  readonly runtimeInstanceId: RuntimeInstanceId;
  readonly nativeModelId: string;
  readonly resolvedModelVersion: string;
  readonly runtimeMode: string;
  readonly isolationProfile: string;
  readonly toolProfile: string;
  readonly generation: string;
}

export interface ObservedCapability extends CapabilityClaim {
  readonly evidenceRefs: ReadonlyArray<string>;
}

export interface QualifiedCapability extends ObservedCapability {
  readonly qualificationKey: string;
  readonly qualification: Readonly<QualificationKeyInput>;
}

export interface QualifiedCapabilityInput extends ObservedCapability {
  readonly qualification: QualificationKeyInput;
}

export interface CapabilityLayers {
  readonly declared: ReadonlyArray<CapabilityClaim>;
  readonly observed: ReadonlyArray<ObservedCapability>;
  readonly qualified: ReadonlyArray<QualifiedCapability>;
}

export interface CapabilityResolution {
  readonly support: CapabilitySupport;
  readonly layer: "qualified" | "none";
  readonly reason: "QUALIFIED" | "NOT_QUALIFIED" | "STALE_QUALIFICATION";
  readonly evidenceRefs: ReadonlyArray<string>;
}
