export type JsonPrimitive = null | boolean | number | string;
export type JsonValue = JsonPrimitive | JsonObject | ReadonlyArray<JsonValue>;
export interface JsonObject {
  readonly [key: string]: JsonValue;
}

export const PUBLIC_OBJECT_SCHEMA = "gogoke.s1-r4.objects.v1" as const;

const freezeStrings = <const Values extends readonly string[]>(values: Values): Values =>
  Object.freeze(values) as Values;

export const EXPOSURE_STATES = freezeStrings([
  "HOST_PREPARED",
  "HOST_DELIVERED",
  "NATIVE_ACKED",
  "INHERITED",
  "POSSIBLE",
  "UNKNOWN",
] as const);
export type ExposureState = (typeof EXPOSURE_STATES)[number];

export const CONTEXT_STATES = freezeStrings([
  "ACTIVE",
  "SUPERSEDED",
  "CONFLICTED",
  "STALE",
  "REVOKED",
  "ARCHIVED",
] as const);
export type ContextState = (typeof CONTEXT_STATES)[number];

export const DECISION_STATES = freezeStrings([
  "REQUESTED",
  "EVALUATING",
  "VALIDATED",
  "COMMITTED",
  "STALE",
  "REJECTED",
  "ABSTAINED",
  "FAILED",
  "CANCELLED",
] as const);
export type DecisionState = (typeof DECISION_STATES)[number];

export const OUTCOME_STATES = freezeStrings([
  "PENDING",
  "OBSERVED",
  "CORRECTED",
  "CENSORED",
] as const);
export type OutcomeState = (typeof OUTCOME_STATES)[number];

export const DREAM_STATES = freezeStrings([
  "DRAFT",
  "DEV_VALIDATED",
  "CALIBRATED",
  "HOLDOUT_VALIDATED",
  "SHADOW",
  "CANARY",
  "ACTIVE",
  "REJECTED",
  "RETIRED",
  "REVOKED",
] as const);
export type DreamState = (typeof DREAM_STATES)[number];

export const CONTEXT_SCOPES = freezeStrings(["GLOBAL", "PROJECT", "SESSION"] as const);
export type ContextScope = (typeof CONTEXT_SCOPES)[number];

export type U64String = string & { readonly U64String: unique symbol };
export type RuntimeDriverId = string & { readonly RuntimeDriverId: unique symbol };
export type RuntimeInstanceId = string & { readonly RuntimeInstanceId: unique symbol };
export type NativeModelId = string & { readonly NativeModelId: unique symbol };
export type RoleId = string & { readonly RoleId: unique symbol };
export type SeatId = string & { readonly SeatId: unique symbol };
/** Account identity is a separate reference; it is intentionally not a v1 wire object. */
export type RuntimeAccountRef = string & { readonly RuntimeAccountRef: unique symbol };

export interface RuntimeDriver {
  readonly driverId: RuntimeDriverId;
  readonly adapterVersion: string;
  readonly artifactDigest: string;
  readonly configSchemaRef: string;
  readonly requiredHostServices: ReadonlyArray<string>;
  readonly admissionRef: string;
}

export interface RuntimeInstance {
  readonly instanceId: RuntimeInstanceId;
  readonly driverId: RuntimeDriverId;
  readonly binaryIdentity: string;
  readonly profileRef: string;
  readonly authRevision: U64String;
  readonly hostRef: string;
  readonly capacityPoolRef: string;
}

export interface ModelRef {
  readonly runtimeInstanceId: RuntimeInstanceId;
  readonly nativeModelId: NativeModelId;
  readonly resolvedVersion: string;
  readonly capabilityRevision: U64String;
}

export interface RoleSpec {
  readonly roleId: RoleId;
  readonly revision: U64String;
  readonly responsibility: string;
  readonly requiredCapabilities: ReadonlyArray<string>;
  readonly delegableCeiling: JsonValue;
  readonly reviewIndependence: boolean;
}

export interface Seat {
  readonly seatId: SeatId;
  readonly roleId: RoleId;
  readonly scope: string;
  readonly domainId: string;
  readonly lifecycle: string;
  readonly grantRef: string;
}

export interface ExecutionRecipe {
  readonly recipeId: string;
  readonly revision: U64String;
  readonly seatId: SeatId;
  readonly runtimeInstanceId: RuntimeInstanceId;
  readonly modelRef: JsonObject;
  readonly toolProfile: JsonValue;
  readonly isolationProfile: JsonValue;
  readonly contextManifestId: string;
  readonly budgetPolicy: JsonValue;
  readonly admissionRef: string;
}

export interface NativeBinding {
  readonly bindingId: string;
  readonly generation: U64String;
  readonly sourceEpoch: U64String;
  readonly instanceId: RuntimeInstanceId;
  readonly nativeIdentity: string;
  readonly domainId: string;
  readonly lineageRef: string;
  readonly custodyRef: string;
}

export interface ContextObject {
  readonly contextId: string;
  readonly version: U64String;
  readonly scope: ContextScope;
  readonly domainId: string;
  readonly kind: string;
  readonly contentHash: string;
  readonly sourceRef: JsonValue;
  readonly sourceAuthority: JsonValue;
  readonly derivedFrom: ReadonlyArray<string>;
  readonly validity: ContextState;
  readonly supersedes: ReadonlyArray<string>;
  readonly accessPolicyRevision: U64String;
}

export interface ContextManifest {
  readonly manifestId: string;
  readonly taskId: string;
  readonly seatId: SeatId;
  readonly bindingGeneration: U64String;
  readonly domainId: string;
  readonly policyRevision: U64String;
  readonly sourceSnapshot: JsonValue;
  readonly requiredConstraints: ReadonlyArray<JsonValue>;
  readonly includedVersions: ReadonlyArray<JsonValue>;
  readonly redactions: ReadonlyArray<JsonValue>;
  readonly selectionDecisionId: string;
  readonly manifestHash: string;
}

export interface ExposureReceipt {
  readonly receiptId: string;
  readonly manifestId: string;
  readonly bindingId: string;
  readonly generation: U64String;
  readonly evidenceLevel: ExposureState;
  readonly nativeSourceCoverage: JsonValue;
  readonly taintLabels: ReadonlyArray<string>;
  readonly evidenceRefs: ReadonlyArray<string>;
}

export interface SessionLineage {
  readonly sessionId: string;
  readonly bindingId: string;
  readonly parentRefs: ReadonlyArray<string>;
  readonly operationKind: string;
  readonly inheritedExposure: JsonValue;
  readonly sourceEpoch: U64String;
}

export interface DecisionRecord {
  readonly decisionId: string;
  readonly family: string;
  readonly stateViewHash: string;
  readonly candidateHash: string;
  readonly sourceRevisions: JsonObject;
  readonly backend: string;
  readonly modelRequested: string;
  readonly modelResolved: string;
  readonly questionVersion: string;
  readonly probabilities: JsonObject;
  readonly nativeConfidence: JsonValue;
  readonly calibrationRef: string;
  readonly mode: string;
  readonly state: DecisionState;
  readonly actionId: string;
}

export interface OutcomeRecord {
  readonly outcomeId: string;
  readonly decisionId: string;
  readonly actionId: string;
  readonly revision: U64String;
  readonly labelSource: string;
  readonly evidenceRefs: ReadonlyArray<string>;
  readonly observationWindow: JsonValue;
  readonly censorStatus: OutcomeState;
  readonly quality: JsonValue;
  readonly cost: JsonValue;
  readonly latency: JsonValue;
  readonly rework: JsonValue;
  readonly safetyEvents: ReadonlyArray<JsonValue>;
}

export interface CalibrationProfile {
  readonly profileId: string;
  readonly family: string;
  readonly modelVersion: string;
  readonly questionViewVersion: string;
  readonly locale: string;
  readonly taskDomain: string;
  readonly datasetSplitHash: string;
  readonly rubricHash: string;
  readonly sampleCounts: JsonObject;
  readonly uncertaintyIntervals: JsonObject;
  readonly coverageRisk: JsonValue;
  readonly testOnly: boolean;
  readonly qualificationGrant: JsonValue;
}

export interface DreamRun {
  readonly runId: string;
  readonly snapshotHash: string;
  readonly domainId: string;
  readonly budgetLease: JsonValue;
  readonly stepReceipts: ReadonlyArray<string>;
  readonly preemptionState: string;
  readonly datasetSplitHash: string;
  readonly recipeRef: string;
}

export interface DreamProposal {
  readonly proposalId: string;
  readonly runId: string;
  readonly kind: string;
  readonly basePolicyRevision: U64String;
  readonly candidateHash: string;
  readonly allowedChangeSet: JsonValue;
  readonly evaluationRefs: ReadonlyArray<string>;
  readonly heldoutReceipt: string;
  readonly state: DreamState;
  readonly activationGrant: JsonValue;
  readonly rollbackRef: string;
}

export interface PublicObjectMap {
  readonly RuntimeDriver: RuntimeDriver;
  readonly RuntimeInstance: RuntimeInstance;
  readonly ModelRef: ModelRef;
  readonly RoleSpec: RoleSpec;
  readonly Seat: Seat;
  readonly ExecutionRecipe: ExecutionRecipe;
  readonly NativeBinding: NativeBinding;
  readonly ContextObject: ContextObject;
  readonly ContextManifest: ContextManifest;
  readonly ExposureReceipt: ExposureReceipt;
  readonly SessionLineage: SessionLineage;
  readonly DecisionRecord: DecisionRecord;
  readonly OutcomeRecord: OutcomeRecord;
  readonly CalibrationProfile: CalibrationProfile;
  readonly DreamRun: DreamRun;
  readonly DreamProposal: DreamProposal;
}

export type PublicObjectType = keyof PublicObjectMap;

export interface PublicObjectEnvelope<K extends PublicObjectType = PublicObjectType> {
  readonly schema: string;
  readonly objectType: K;
  readonly object: PublicObjectMap[K];
}

export interface UnknownFields {
  readonly envelope: JsonObject;
  readonly object: JsonObject;
}

/**
 * Unknown minor-version fields round-trip here, outside the authority-bearing
 * `value`. Callers cannot gain grants, roles, seats, or lifecycle state by
 * placing lookalike properties in an extension field.
 */
export interface DecodedPublicObject<K extends PublicObjectType = PublicObjectType> {
  readonly value: PublicObjectEnvelope<K>;
  readonly unknownFields: UnknownFields;
}

export type FieldRule =
  | "boolean"
  | "driverId"
  | "instanceId"
  | "json"
  | "jsonArray"
  | "jsonObject"
  | "string"
  | "stringArray"
  | "u64"
  | readonly ["enum", ReadonlyArray<string>];

type ObjectDefinition = Readonly<Record<string, FieldRule>>;

const enumRule = (values: ReadonlyArray<string>): FieldRule =>
  Object.freeze(["enum", values] as const);

/** Runtime schema source of truth. Every listed field is required in v1. */
export const OBJECT_DEFINITIONS: Readonly<Record<PublicObjectType, ObjectDefinition>> = {
  RuntimeDriver: {
    driverId: "driverId",
    adapterVersion: "string",
    artifactDigest: "string",
    configSchemaRef: "string",
    requiredHostServices: "stringArray",
    admissionRef: "string",
  },
  RuntimeInstance: {
    instanceId: "instanceId",
    driverId: "driverId",
    binaryIdentity: "string",
    profileRef: "string",
    authRevision: "u64",
    hostRef: "string",
    capacityPoolRef: "string",
  },
  ModelRef: {
    runtimeInstanceId: "instanceId",
    nativeModelId: "string",
    resolvedVersion: "string",
    capabilityRevision: "u64",
  },
  RoleSpec: {
    roleId: "string",
    revision: "u64",
    responsibility: "string",
    requiredCapabilities: "stringArray",
    delegableCeiling: "json",
    reviewIndependence: "boolean",
  },
  Seat: {
    seatId: "string",
    roleId: "string",
    scope: "string",
    domainId: "string",
    lifecycle: "string",
    grantRef: "string",
  },
  ExecutionRecipe: {
    recipeId: "string",
    revision: "u64",
    seatId: "string",
    runtimeInstanceId: "instanceId",
    modelRef: "jsonObject",
    toolProfile: "json",
    isolationProfile: "json",
    contextManifestId: "string",
    budgetPolicy: "json",
    admissionRef: "string",
  },
  NativeBinding: {
    bindingId: "string",
    generation: "u64",
    sourceEpoch: "u64",
    instanceId: "instanceId",
    nativeIdentity: "string",
    domainId: "string",
    lineageRef: "string",
    custodyRef: "string",
  },
  ContextObject: {
    contextId: "string",
    version: "u64",
    scope: enumRule(CONTEXT_SCOPES),
    domainId: "string",
    kind: "string",
    contentHash: "string",
    sourceRef: "json",
    sourceAuthority: "json",
    derivedFrom: "stringArray",
    validity: enumRule(CONTEXT_STATES),
    supersedes: "stringArray",
    accessPolicyRevision: "u64",
  },
  ContextManifest: {
    manifestId: "string",
    taskId: "string",
    seatId: "string",
    bindingGeneration: "u64",
    domainId: "string",
    policyRevision: "u64",
    sourceSnapshot: "json",
    requiredConstraints: "jsonArray",
    includedVersions: "jsonArray",
    redactions: "jsonArray",
    selectionDecisionId: "string",
    manifestHash: "string",
  },
  ExposureReceipt: {
    receiptId: "string",
    manifestId: "string",
    bindingId: "string",
    generation: "u64",
    evidenceLevel: enumRule(EXPOSURE_STATES),
    nativeSourceCoverage: "json",
    taintLabels: "stringArray",
    evidenceRefs: "stringArray",
  },
  SessionLineage: {
    sessionId: "string",
    bindingId: "string",
    parentRefs: "stringArray",
    operationKind: "string",
    inheritedExposure: "json",
    sourceEpoch: "u64",
  },
  DecisionRecord: {
    decisionId: "string",
    family: "string",
    stateViewHash: "string",
    candidateHash: "string",
    sourceRevisions: "jsonObject",
    backend: "string",
    modelRequested: "string",
    modelResolved: "string",
    questionVersion: "string",
    probabilities: "jsonObject",
    nativeConfidence: "json",
    calibrationRef: "string",
    mode: "string",
    state: enumRule(DECISION_STATES),
    actionId: "string",
  },
  OutcomeRecord: {
    outcomeId: "string",
    decisionId: "string",
    actionId: "string",
    revision: "u64",
    labelSource: "string",
    evidenceRefs: "stringArray",
    observationWindow: "json",
    censorStatus: enumRule(OUTCOME_STATES),
    quality: "json",
    cost: "json",
    latency: "json",
    rework: "json",
    safetyEvents: "jsonArray",
  },
  CalibrationProfile: {
    profileId: "string",
    family: "string",
    modelVersion: "string",
    questionViewVersion: "string",
    locale: "string",
    taskDomain: "string",
    datasetSplitHash: "string",
    rubricHash: "string",
    sampleCounts: "jsonObject",
    uncertaintyIntervals: "jsonObject",
    coverageRisk: "json",
    testOnly: "boolean",
    qualificationGrant: "json",
  },
  DreamRun: {
    runId: "string",
    snapshotHash: "string",
    domainId: "string",
    budgetLease: "json",
    stepReceipts: "stringArray",
    preemptionState: "string",
    datasetSplitHash: "string",
    recipeRef: "string",
  },
  DreamProposal: {
    proposalId: "string",
    runId: "string",
    kind: "string",
    basePolicyRevision: "u64",
    candidateHash: "string",
    allowedChangeSet: "json",
    evaluationRefs: "stringArray",
    heldoutReceipt: "string",
    state: enumRule(DREAM_STATES),
    activationGrant: "json",
    rollbackRef: "string",
  },
};

for (const definition of Object.values(OBJECT_DEFINITIONS)) Object.freeze(definition);
Object.freeze(OBJECT_DEFINITIONS);
