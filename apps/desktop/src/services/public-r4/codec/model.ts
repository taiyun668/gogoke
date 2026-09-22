import objectModel from "../../../../contracts/s1-r4/object-model.v1.json";
import fieldRules from "../../../../contracts/s1-r4/field-rules.v1.json";

export type JsonPrimitive = null | boolean | number | string;
export type JsonValue = JsonPrimitive | JsonObject | ReadonlyArray<JsonValue>;
export interface JsonObject {
  readonly [key: string]: JsonValue;
}

type ObjectModelSource = {
  readonly schema: string;
  readonly objects: Readonly<Record<string, ReadonlyArray<string>>>;
  readonly states: {
    readonly exposure: ReadonlyArray<string>;
    readonly context: ReadonlyArray<string>;
    readonly decision: ReadonlyArray<string>;
    readonly outcome: ReadonlyArray<string>;
    readonly dream: ReadonlyArray<string>;
  };
};

const deepFreezeImport = <Value>(value: Value): Value => {
  if (Array.isArray(value)) {
    return Object.freeze(value.map((item) => deepFreezeImport(item))) as Value;
  }
  if (typeof value === "object" && value !== null) {
    const clone: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) {
      Object.defineProperty(clone, key, {
        value: deepFreezeImport(item),
        enumerable: true,
        configurable: false,
        writable: false,
      });
    }
    return Object.freeze(clone) as Value;
  }
  return value;
};

const CHECKED_IN_OBJECT_MODEL = deepFreezeImport(objectModel as ObjectModelSource);
const CHECKED_IN_FIELD_RULES = deepFreezeImport(fieldRules as {
  readonly schema: string;
  readonly objects: Readonly<Record<string, Readonly<Record<string, unknown>>>>;
});

export const PUBLIC_OBJECT_SCHEMA = CHECKED_IN_OBJECT_MODEL.schema as "gogoke.s1-r4.objects.v1";

export const EXPOSURE_STATES = Object.freeze([...CHECKED_IN_OBJECT_MODEL.states.exposure]) as unknown as readonly [
  "HOST_PREPARED",
  "HOST_DELIVERED",
  "NATIVE_ACKED",
  "INHERITED",
  "POSSIBLE",
  "UNKNOWN",
];
export type ExposureState = (typeof EXPOSURE_STATES)[number];

export const CONTEXT_STATES = Object.freeze([...CHECKED_IN_OBJECT_MODEL.states.context]) as unknown as readonly [
  "ACTIVE",
  "SUPERSEDED",
  "CONFLICTED",
  "STALE",
  "REVOKED",
  "ARCHIVED",
];
export type ContextState = (typeof CONTEXT_STATES)[number];

export const DECISION_STATES = Object.freeze([...CHECKED_IN_OBJECT_MODEL.states.decision]) as unknown as readonly [
  "REQUESTED",
  "EVALUATING",
  "VALIDATED",
  "COMMITTED",
  "STALE",
  "REJECTED",
  "ABSTAINED",
  "FAILED",
  "CANCELLED",
];
export type DecisionState = (typeof DECISION_STATES)[number];

export const OUTCOME_STATES = Object.freeze([...CHECKED_IN_OBJECT_MODEL.states.outcome]) as unknown as readonly [
  "PENDING",
  "OBSERVED",
  "CORRECTED",
  "CENSORED",
];
export type OutcomeState = (typeof OUTCOME_STATES)[number];

export const DREAM_STATES = Object.freeze([...CHECKED_IN_OBJECT_MODEL.states.dream]) as unknown as readonly [
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
];
export type DreamState = (typeof DREAM_STATES)[number];

export const CONTEXT_SCOPES = Object.freeze(["GLOBAL", "PROJECT", "SESSION"] as const);
export type ContextScope = (typeof CONTEXT_SCOPES)[number];

export const PUBLIC_OBJECT_TYPES = Object.freeze(
  Object.keys(CHECKED_IN_OBJECT_MODEL.objects),
) as readonly PublicObjectType[];

export const PUBLIC_CONTRACT_SOURCES = {
  objectModel: CHECKED_IN_OBJECT_MODEL,
  fieldRules: CHECKED_IN_FIELD_RULES,
} as const;

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

const normalizeFieldRule = (value: unknown): FieldRule => {
  if (Array.isArray(value) && value.length === 2 && value[0] === "enum" && Array.isArray(value[1])) {
    return Object.freeze([
      "enum",
      Object.freeze(value[1].map((entry) => String(entry))),
    ] as const);
  }
  if (
    value === "boolean" ||
    value === "driverId" ||
    value === "instanceId" ||
    value === "json" ||
    value === "jsonArray" ||
    value === "jsonObject" ||
    value === "string" ||
    value === "stringArray" ||
    value === "u64"
  ) {
    return value;
  }
  throw new Error("invalid checked-in field rule");
};

const objectDefinitions = Object.fromEntries(
  Object.entries(CHECKED_IN_FIELD_RULES.objects).map(([objectType, fields]) => [
    objectType,
    Object.freeze(Object.fromEntries(
      Object.entries(fields).map(([field, rule]) => [field, normalizeFieldRule(rule)]),
    )),
  ]),
) as Readonly<Record<PublicObjectType, ObjectDefinition>>;

/** Runtime schema source of truth. Every listed field is required in v1. */
export const OBJECT_DEFINITIONS = Object.freeze(objectDefinitions);
