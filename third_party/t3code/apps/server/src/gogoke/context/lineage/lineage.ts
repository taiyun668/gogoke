import { isDeepStrictEqual } from "node:util";
import { isProxy } from "node:util/types";
import type { U64String } from "../../contracts/model.ts";

export const EXPOSURE_EVIDENCE_LEVELS = [
  "HOST_PREPARED",
  "HOST_DELIVERED",
  "NATIVE_ACKED",
  "INHERITED",
  "POSSIBLE",
  "UNKNOWN",
] as const;
export type ExposureEvidenceLevel = (typeof EXPOSURE_EVIDENCE_LEVELS)[number];

export const EXPOSURE_CLASSIFICATIONS = ["CLEAN", "TAINTED", "UNKNOWN"] as const;
export type ExposureClassification = (typeof EXPOSURE_CLASSIFICATIONS)[number];

export const SESSION_OPERATIONS = [
  "NEW_CLEAN",
  "RESUME",
  "NATIVE_FORK",
  "REBUILD",
  "HANDOFF",
  "ARCHIVE",
] as const;
export type SessionOperation = (typeof SESSION_OPERATIONS)[number];

export type SessionLifecycle = "ACTIVE" | "ARCHIVED";
export type NativeProcessState = "RUNNING" | "IDLE" | "UNKNOWN";

export interface SourceObservation {
  readonly sourceRef: string;
  readonly status: "COMPLETE" | "PARTIAL" | "NOT_OBSERVED" | "UNKNOWN";
}

/**
 * Coverage is deliberately explicit. A host delivery is not a model-observation
 * proof, and `complete: true` is accepted only when every listed source is complete.
 */
export interface NativeSourceCoverage {
  readonly complete: boolean;
  readonly observations: ReadonlyArray<SourceObservation>;
  readonly unknownSources: ReadonlyArray<string>;
  readonly inheritedFromReceiptId?: string;
}

export interface ExposureReceipt {
  readonly receiptId: string;
  readonly manifestId: string;
  readonly bindingId: string;
  readonly generation: U64String;
  readonly evidenceLevel: ExposureEvidenceLevel;
  readonly nativeSourceCoverage: NativeSourceCoverage;
  readonly taintLabels: ReadonlyArray<string>;
  readonly evidenceRefs: ReadonlyArray<string>;
}

export interface ExposureAssessment {
  readonly classification: ExposureClassification;
  readonly evidenceLevel: ExposureEvidenceLevel | "NONE";
  readonly taintLabels: ReadonlyArray<string>;
  readonly unknownSources: ReadonlyArray<string>;
  readonly reason:
    | "NO_RECEIPT"
    | "HOST_DELIVERY_NOT_OBSERVATION"
    | "NATIVE_COVERAGE_INCOMPLETE"
    | "NATIVE_COVERAGE_UNKNOWN"
    | "EXPLICIT_TAINT"
    | "INHERITED_TAINT"
    | "NATIVE_COVERAGE_COMPLETE";
}

export interface InheritedExposureSummary {
  readonly evidenceLevels: ReadonlyArray<ExposureEvidenceLevel>;
  readonly taintLabels: ReadonlyArray<string>;
  readonly unknownSources: ReadonlyArray<string>;
  readonly sourceReceiptRefs: ReadonlyArray<string>;
}

export interface SessionLineage {
  readonly sessionId: string;
  readonly bindingId: string;
  readonly parentRefs: ReadonlyArray<string>;
  readonly operationKind: SessionOperation;
  readonly inheritedExposure: InheritedExposureSummary;
  readonly sourceEpoch: U64String;
}

export interface PendingActionRef {
  readonly actionId: string;
  readonly operationId: string;
  readonly bindingId: string;
  readonly generation: U64String;
  readonly state: "PENDING";
}

export interface NativeSessionIdentity {
  readonly nativeSessionId: string;
  readonly bindingId: string;
  readonly generation: U64String;
  readonly sourceEpoch: U64String;
  readonly domainId: string;
}

export interface MaterialHandoff {
  readonly materialIds: ReadonlyArray<string>;
  readonly sourceSessionId: string;
  readonly nativeResumeUsed: false;
}

export interface SessionSnapshot {
  readonly sessionId: string;
  readonly lineage: SessionLineage;
  readonly native: NativeSessionIdentity;
  readonly lifecycle: SessionLifecycle;
  readonly processState: NativeProcessState;
  readonly exposure: ExposureReceipt | null;
  readonly exposureAssessment: ExposureAssessment;
  readonly pendingActions: ReadonlyArray<PendingActionRef>;
  readonly materialHandoff: MaterialHandoff | null;
  readonly pendingActionDisposition: "NONE" | "RETAINED_ON_PARENT" | "RETAINED_NOT_REPLAYED";
}

export interface NewCleanInput extends NativeSessionIdentity {
  readonly sessionId: string;
}

export interface ResumeInput {
  readonly sessionId: string;
  readonly native: NativeSessionIdentity;
}

export interface ChildSessionInput extends NativeSessionIdentity {
  readonly sessionId: string;
  readonly operation: "NATIVE_FORK" | "REBUILD";
}

export interface HandoffInput extends NativeSessionIdentity {
  readonly sessionId: string;
  readonly materialIds: ReadonlyArray<string>;
}

export class LineageError extends Error {
  override readonly name = "LineageError";
  readonly code:
    | "INVALID_INPUT"
    | "DUPLICATE_SESSION"
    | "SESSION_NOT_FOUND"
    | "NATIVE_IDENTITY_MISMATCH"
    | "INVALID_TRANSITION"
    | "INVALID_EXPOSURE";

  constructor(code: LineageError["code"], detail: string) {
    super(`${code}: ${detail}`);
    this.code = code;
  }
}

const U64 = /^(?:0|[1-9]\d*)$/u;
const ID = /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,255}$/u;

const invalid = (detail: string): never => {
  throw new LineageError("INVALID_INPUT", detail);
};

type RecordSnapshot = Record<string, unknown>;

const passiveObject = (value: unknown, path: string): object => {
  if (typeof value !== "object" || value === null) return invalid(`${path} must be an object`);
  if (isProxy(value)) return invalid(`${path} must not be a Proxy`);
  return value;
};

const recordSnapshot = (
  value: unknown,
  path: string,
  requiredKeys: ReadonlyArray<string>,
  optionalKeys: ReadonlyArray<string> = [],
): RecordSnapshot => {
  const object = passiveObject(value, path);
  if (Array.isArray(object)) return invalid(`${path} must be a plain record`);
  const prototype = Object.getPrototypeOf(object);
  if (prototype !== Object.prototype && prototype !== null) {
    return invalid(`${path} must not have a custom prototype`);
  }

  const ownKeys = Reflect.ownKeys(object);
  if (ownKeys.some((key) => typeof key !== "string")) {
    return invalid(`${path} must not contain symbol properties`);
  }
  const keySet = new Set(ownKeys as string[]);
  const allowedKeys = new Set([...requiredKeys, ...optionalKeys]);
  for (const key of requiredKeys) {
    if (!keySet.has(key)) return invalid(`${path}.${key} is required`);
  }
  for (const key of keySet) {
    if (!allowedKeys.has(key)) return invalid(`${path} contains an extra property`);
  }

  const snapshot = Object.create(null) as RecordSnapshot;
  for (const key of keySet) {
    const descriptor = Object.getOwnPropertyDescriptor(object, key);
    if (descriptor === undefined || !Object.hasOwn(descriptor, "value")) {
      return invalid(`${path}.${key} must be an own data property`);
    }
    snapshot[key] = descriptor.value;
  }
  return snapshot;
};

const arraySnapshot = (value: unknown, path: string): ReadonlyArray<unknown> => {
  const object = passiveObject(value, path);
  if (!Array.isArray(object)) return invalid(`${path} must be an array`);
  if (Object.getPrototypeOf(object) !== Array.prototype) {
    return invalid(`${path} must not have a custom prototype`);
  }

  const lengthDescriptor = Object.getOwnPropertyDescriptor(object, "length");
  if (
    lengthDescriptor === undefined ||
    !Object.hasOwn(lengthDescriptor, "value") ||
    typeof lengthDescriptor.value !== "number" ||
    !Number.isInteger(lengthDescriptor.value)
  ) {
    return invalid(`${path} has an invalid length`);
  }
  const length = lengthDescriptor.value;
  const ownKeys = Reflect.ownKeys(object);
  if (ownKeys.length !== length + 1)
    return invalid(`${path} must be dense and have no extra properties`);

  const values = new Array<unknown>(length);
  const seen = new Set<number>();
  for (const key of ownKeys) {
    if (typeof key !== "string") return invalid(`${path} must not contain symbol properties`);
    if (key === "length") continue;
    if (!/^(?:0|[1-9]\d*)$/u.test(key)) {
      return invalid(`${path} must not contain extra properties`);
    }
    const index = Number(key);
    if (!Number.isSafeInteger(index) || index >= length || String(index) !== key) {
      return invalid(`${path} has an invalid array index`);
    }
    const descriptor = Object.getOwnPropertyDescriptor(object, key);
    if (descriptor === undefined || !Object.hasOwn(descriptor, "value")) {
      return invalid(`${path}[${index}] must be an own data property`);
    }
    values[index] = descriptor.value;
    seen.add(index);
  }
  if (seen.size !== length) return invalid(`${path} must not be sparse`);
  return values;
};

const id = (value: unknown, path: string): string => {
  if (typeof value !== "string" || !ID.test(value)) return invalid(`${path} is not canonical`);
  return value;
};

const u64 = (value: unknown, path: string): U64String => {
  if (typeof value !== "string" || !U64.test(value))
    return invalid(`${path} is not a canonical u64`);
  return value as U64String;
};

const uniqueStrings = (input: unknown, path: string): ReadonlyArray<string> => {
  const values = arraySnapshot(input, path).map((value) => id(value, path));
  if (new Set(values).size !== values.length) return invalid(`${path} contains duplicates`);
  return Object.freeze(values);
};

const freezeObservation = (value: SourceObservation): SourceObservation => {
  const fields = recordSnapshot(value, "source observation", ["sourceRef", "status"]);
  const sourceRef = id(fields.sourceRef, "sourceRef");
  const status = fields.status;
  if (
    !(
      "COMPLETE" === status ||
      "PARTIAL" === status ||
      "NOT_OBSERVED" === status ||
      "UNKNOWN" === status
    )
  ) {
    return invalid("source observation status is invalid");
  }
  return Object.freeze({ sourceRef, status });
};

const freezeCoverage = (value: NativeSourceCoverage): NativeSourceCoverage => {
  const fields = recordSnapshot(
    value,
    "nativeSourceCoverage",
    ["complete", "observations", "unknownSources"],
    ["inheritedFromReceiptId"],
  );
  const observations = Object.freeze(
    arraySnapshot(fields.observations, "observations").map((entry) =>
      freezeObservation(entry as SourceObservation),
    ),
  );
  const unknownSources = uniqueStrings(fields.unknownSources, "unknownSources");
  const hasInheritedFrom = Object.hasOwn(fields, "inheritedFromReceiptId");
  const inheritedFromReceiptId = hasInheritedFrom
    ? id(fields.inheritedFromReceiptId, "inheritedFromReceiptId")
    : undefined;
  const derivedComplete =
    !hasInheritedFrom &&
    observations.length > 0 &&
    observations.every((entry) => entry.status === "COMPLETE") &&
    unknownSources.length === 0;
  if (typeof fields.complete !== "boolean" || fields.complete !== derivedComplete) {
    return invalid("nativeSourceCoverage.complete does not match observations");
  }
  return Object.freeze({
    complete: fields.complete,
    observations,
    unknownSources,
    ...(!hasInheritedFrom ? {} : { inheritedFromReceiptId: inheritedFromReceiptId! }),
  });
};

const freezeReceipt = (value: ExposureReceipt): ExposureReceipt => {
  const fields = recordSnapshot(value, "exposure receipt", [
    "receiptId",
    "manifestId",
    "bindingId",
    "generation",
    "evidenceLevel",
    "nativeSourceCoverage",
    "taintLabels",
    "evidenceRefs",
  ]);
  const receiptId = id(fields.receiptId, "receiptId");
  const manifestId = id(fields.manifestId, "manifestId");
  const bindingId = id(fields.bindingId, "bindingId");
  const generation = u64(fields.generation, "generation");
  if (
    typeof fields.evidenceLevel !== "string" ||
    !(EXPOSURE_EVIDENCE_LEVELS as ReadonlyArray<string>).includes(fields.evidenceLevel)
  ) {
    return invalid("evidenceLevel is invalid");
  }
  return Object.freeze({
    receiptId,
    manifestId,
    bindingId,
    generation,
    evidenceLevel: fields.evidenceLevel as ExposureEvidenceLevel,
    nativeSourceCoverage: freezeCoverage(fields.nativeSourceCoverage as NativeSourceCoverage),
    taintLabels: uniqueStrings(fields.taintLabels, "taintLabels"),
    evidenceRefs: uniqueStrings(fields.evidenceRefs, "evidenceRefs"),
  });
};

const freezePendingAction = (value: PendingActionRef): PendingActionRef => {
  const fields = recordSnapshot(value, "pending action", [
    "actionId",
    "operationId",
    "bindingId",
    "generation",
    "state",
  ]);
  if (fields.state !== "PENDING") return invalid("pending action state is invalid");
  return Object.freeze({
    actionId: id(fields.actionId, "pending actionId"),
    operationId: id(fields.operationId, "pending operationId"),
    bindingId: id(fields.bindingId, "pending bindingId"),
    generation: u64(fields.generation, "pending generation"),
    state: "PENDING",
  });
};

const freezePendingActions = (input: unknown): ReadonlyArray<PendingActionRef> =>
  Object.freeze(
    arraySnapshot(input, "pendingActions").map((value) =>
      freezePendingAction(value as PendingActionRef),
    ),
  );

const assess = (receipt: ExposureReceipt | null): ExposureAssessment => {
  if (receipt === null) {
    return Object.freeze({
      classification: "UNKNOWN",
      evidenceLevel: "NONE",
      taintLabels: Object.freeze([]),
      unknownSources: Object.freeze([]),
      reason: "NO_RECEIPT",
    });
  }
  const coverage = receipt.nativeSourceCoverage;
  const taintLabels = receipt.taintLabels;
  const unknownSources = Object.freeze([
    ...coverage.unknownSources,
    ...coverage.observations
      .filter((entry) => entry.status === "UNKNOWN" || entry.status === "NOT_OBSERVED")
      .map((entry) => entry.sourceRef),
  ]);
  if (taintLabels.length > 0) {
    return Object.freeze({
      classification: "TAINTED",
      evidenceLevel: receipt.evidenceLevel,
      taintLabels,
      unknownSources,
      reason: receipt.evidenceLevel === "INHERITED" ? "INHERITED_TAINT" : "EXPLICIT_TAINT",
    });
  }
  if (receipt.evidenceLevel !== "NATIVE_ACKED" || !coverage.complete) {
    return Object.freeze({
      classification: "UNKNOWN",
      evidenceLevel: receipt.evidenceLevel,
      taintLabels,
      unknownSources,
      reason:
        receipt.evidenceLevel === "HOST_DELIVERED" || receipt.evidenceLevel === "HOST_PREPARED"
          ? "HOST_DELIVERY_NOT_OBSERVATION"
          : "NATIVE_COVERAGE_INCOMPLETE",
    });
  }
  if (unknownSources.length > 0) {
    return Object.freeze({
      classification: "UNKNOWN",
      evidenceLevel: receipt.evidenceLevel,
      taintLabels,
      unknownSources,
      reason: "NATIVE_COVERAGE_UNKNOWN",
    });
  }
  return Object.freeze({
    classification: "CLEAN",
    evidenceLevel: receipt.evidenceLevel,
    taintLabels,
    unknownSources,
    reason: "NATIVE_COVERAGE_COMPLETE",
  });
};

export const assessExposure = (input: ExposureReceipt | null): ExposureAssessment =>
  assess(input === null ? null : freezeReceipt(input));

export const snapshotExposureReceipt = (input: ExposureReceipt): ExposureReceipt =>
  freezeReceipt(input);

export const inheritExposure = (
  parent: ExposureReceipt,
  child: Pick<ExposureReceipt, "receiptId" | "manifestId" | "bindingId" | "generation">,
): ExposureReceipt => {
  const parentReceipt = freezeReceipt(parent);
  const childFields = recordSnapshot(child, "child exposure", [
    "receiptId",
    "manifestId",
    "bindingId",
    "generation",
  ]);
  const next = freezeReceipt({
    receiptId: id(childFields.receiptId, "receiptId"),
    manifestId: id(childFields.manifestId, "manifestId"),
    bindingId: id(childFields.bindingId, "bindingId"),
    generation: u64(childFields.generation, "generation"),
    evidenceLevel: "INHERITED",
    nativeSourceCoverage: {
      complete: false,
      observations: parentReceipt.nativeSourceCoverage.observations,
      unknownSources: parentReceipt.nativeSourceCoverage.unknownSources,
      inheritedFromReceiptId: parentReceipt.receiptId,
    },
    taintLabels: parentReceipt.taintLabels,
    evidenceRefs: [...new Set([...parentReceipt.evidenceRefs, parentReceipt.receiptId])],
  });
  return next;
};

const nativeFromFields = (fields: RecordSnapshot): NativeSessionIdentity =>
  Object.freeze({
    nativeSessionId: id(fields.nativeSessionId, "nativeSessionId"),
    bindingId: id(fields.bindingId, "bindingId"),
    generation: u64(fields.generation, "generation"),
    sourceEpoch: u64(fields.sourceEpoch, "sourceEpoch"),
    domainId: id(fields.domainId, "domainId"),
  });

const freezeNative = (value: NativeSessionIdentity): NativeSessionIdentity =>
  nativeFromFields(
    recordSnapshot(value, "native identity", [
      "nativeSessionId",
      "bindingId",
      "generation",
      "sourceEpoch",
      "domainId",
    ]),
  );

const freezeNewCleanInput = (value: NewCleanInput): NewCleanInput => {
  const fields = recordSnapshot(value, "new session", [
    "sessionId",
    "nativeSessionId",
    "bindingId",
    "generation",
    "sourceEpoch",
    "domainId",
  ]);
  return Object.freeze({
    sessionId: id(fields.sessionId, "sessionId"),
    ...nativeFromFields(fields),
  });
};

const freezeResumeInput = (value: ResumeInput): ResumeInput => {
  const fields = recordSnapshot(value, "resume input", ["sessionId", "native"]);
  return Object.freeze({
    sessionId: id(fields.sessionId, "sessionId"),
    native: freezeNative(fields.native as NativeSessionIdentity),
  });
};

const freezeChildInput = (value: ChildSessionInput): ChildSessionInput => {
  const fields = recordSnapshot(value, "child session", [
    "sessionId",
    "nativeSessionId",
    "bindingId",
    "generation",
    "sourceEpoch",
    "domainId",
    "operation",
  ]);
  if (fields.operation !== "NATIVE_FORK" && fields.operation !== "REBUILD") {
    return invalid("child operation is invalid");
  }
  return Object.freeze({
    sessionId: id(fields.sessionId, "sessionId"),
    ...nativeFromFields(fields),
    operation: fields.operation,
  });
};

const freezeHandoffInput = (value: HandoffInput): HandoffInput => {
  const fields = recordSnapshot(value, "handoff input", [
    "sessionId",
    "nativeSessionId",
    "bindingId",
    "generation",
    "sourceEpoch",
    "domainId",
    "materialIds",
  ]);
  return Object.freeze({
    sessionId: id(fields.sessionId, "sessionId"),
    ...nativeFromFields(fields),
    materialIds: uniqueStrings(fields.materialIds, "materialIds"),
  });
};

type ReceiptIds = Pick<ExposureReceipt, "receiptId" | "manifestId">;

const freezeReceiptIds = (value: ReceiptIds): ReceiptIds => {
  const fields = recordSnapshot(value, "receipt ids", ["receiptId", "manifestId"]);
  return Object.freeze({
    receiptId: id(fields.receiptId, "receiptId"),
    manifestId: id(fields.manifestId, "manifestId"),
  });
};

const sameNative = (left: NativeSessionIdentity, right: NativeSessionIdentity): boolean =>
  left.nativeSessionId === right.nativeSessionId &&
  left.bindingId === right.bindingId &&
  left.generation === right.generation &&
  left.sourceEpoch === right.sourceEpoch &&
  left.domainId === right.domainId;

const freezeLineage = (value: SessionLineage): SessionLineage =>
  Object.freeze({
    sessionId: id(value.sessionId, "sessionId"),
    bindingId: id(value.bindingId, "lineage.bindingId"),
    parentRefs: uniqueStrings(value.parentRefs, "parentRefs"),
    operationKind: value.operationKind,
    inheritedExposure: freezeInheritedExposure(value.inheritedExposure),
    sourceEpoch: u64(value.sourceEpoch, "lineage.sourceEpoch"),
  });

const emptyInheritedExposure = (): InheritedExposureSummary =>
  Object.freeze({
    evidenceLevels: Object.freeze([]),
    taintLabels: Object.freeze([]),
    unknownSources: Object.freeze([]),
    sourceReceiptRefs: Object.freeze([]),
  });

/** Carries accumulated provenance forward without claiming a material exposure receipt. */
const mergeInheritedExposure = (
  parent: InheritedExposureSummary,
  receipt: ExposureReceipt | null,
): InheritedExposureSummary => {
  const assessment = assess(receipt);
  const incompleteSources =
    receipt === null || receipt.nativeSourceCoverage.complete
      ? []
      : receipt.nativeSourceCoverage.observations
          .filter((entry) => entry.status !== "COMPLETE")
          .map((entry) => entry.sourceRef);
  return Object.freeze({
    evidenceLevels: Object.freeze([
      ...new Set([
        ...parent.evidenceLevels,
        ...(receipt === null ? [] : [receipt.evidenceLevel]),
        ...(assessment.classification === "UNKNOWN" ? ["UNKNOWN" as const] : []),
      ]),
    ]),
    taintLabels: Object.freeze([
      ...new Set([...parent.taintLabels, ...(receipt?.taintLabels ?? [])]),
    ]),
    unknownSources: uniqueStrings(
      [...new Set([...parent.unknownSources, ...assessment.unknownSources, ...incompleteSources])],
      "lineage.unknownSources",
    ),
    sourceReceiptRefs: Object.freeze([
      ...new Set([
        ...parent.sourceReceiptRefs,
        ...(receipt === null ? [] : [receipt.receiptId, ...receipt.evidenceRefs]),
      ]),
    ]),
  });
};

const assessWithInherited = (
  receipt: ExposureReceipt | null,
  inherited: InheritedExposureSummary,
): ExposureAssessment => {
  const current = assess(receipt);
  const taintLabels = Object.freeze([
    ...new Set([...current.taintLabels, ...inherited.taintLabels]),
  ]);
  const unknownSources = Object.freeze([
    ...new Set([...current.unknownSources, ...inherited.unknownSources]),
  ]);

  if (taintLabels.length > 0) {
    return Object.freeze({
      classification: "TAINTED",
      evidenceLevel: current.evidenceLevel,
      taintLabels,
      unknownSources,
      reason:
        current.taintLabels.length > 0 && inherited.taintLabels.length === 0
          ? current.reason
          : "INHERITED_TAINT",
    });
  }

  const inheritedUnknown =
    inherited.unknownSources.length > 0 ||
    inherited.evidenceLevels.some((level) => level !== "NATIVE_ACKED");
  if (current.classification === "UNKNOWN" || inheritedUnknown) {
    return Object.freeze({
      classification: "UNKNOWN",
      evidenceLevel: current.evidenceLevel,
      taintLabels,
      unknownSources,
      reason:
        current.classification === "UNKNOWN"
          ? current.reason
          : unknownSources.length > 0
            ? "NATIVE_COVERAGE_UNKNOWN"
            : "NATIVE_COVERAGE_INCOMPLETE",
    });
  }

  return Object.freeze({ ...current, taintLabels, unknownSources });
};

const freezeInheritedExposure = (value: InheritedExposureSummary): InheritedExposureSummary => {
  const evidenceLevels = arraySnapshot(value.evidenceLevels, "lineage.evidenceLevels");
  if (
    evidenceLevels.some(
      (entry) =>
        typeof entry !== "string" ||
        !(EXPOSURE_EVIDENCE_LEVELS as ReadonlyArray<string>).includes(entry),
    )
  ) {
    return invalid("lineage.inheritedExposure.evidenceLevels is invalid");
  }
  return Object.freeze({
    evidenceLevels: Object.freeze(evidenceLevels as ExposureEvidenceLevel[]),
    taintLabels: uniqueStrings(value.taintLabels, "lineage.taintLabels"),
    unknownSources: uniqueStrings(value.unknownSources, "lineage.unknownSources"),
    sourceReceiptRefs: uniqueStrings(value.sourceReceiptRefs, "lineage.sourceReceiptRefs"),
  });
};

const cloneSnapshot = (value: SessionSnapshot): SessionSnapshot => {
  const lineage = freezeLineage(value.lineage);
  const exposure = value.exposure === null ? null : freezeReceipt(value.exposure);
  return Object.freeze({
    ...value,
    lineage,
    native: freezeNative(value.native),
    exposure,
    exposureAssessment: assessWithInherited(exposure, lineage.inheritedExposure),
    pendingActions: freezePendingActions(value.pendingActions),
    materialHandoff:
      value.materialHandoff === null
        ? null
        : Object.freeze({
            materialIds: uniqueStrings(value.materialHandoff.materialIds, "materialIds"),
            sourceSessionId: id(value.materialHandoff.sourceSessionId, "sourceSessionId"),
            nativeResumeUsed: false as const,
          }),
  });
};

type CreationRequest =
  | { readonly kind: "NEW_CLEAN"; readonly input: NewCleanInput }
  | {
      readonly kind: "NATIVE_FORK" | "REBUILD";
      readonly parentSessionId: string;
      readonly input: ChildSessionInput;
      readonly receiptIds: ReceiptIds;
    }
  | {
      readonly kind: "HANDOFF";
      readonly parentSessionId: string;
      readonly input: HandoffInput;
    };

interface RetainedReceipt {
  readonly sessionId: string;
  readonly receipt: ExposureReceipt;
}

/**
 * An in-memory preparatory seam. It models the continuity contract without
 * pretending to own the product store, native host, grants, or dispatch port.
 */
export class SessionLineageLedger {
  readonly #sessions = new Map<string, SessionSnapshot>();
  readonly #creationRequests = new Map<string, CreationRequest>();
  readonly #receipts = new Map<string, RetainedReceipt>();

  createNewClean(input: NewCleanInput): SessionSnapshot {
    const normalizedInput = freezeNewCleanInput(input);
    const sessionId = normalizedInput.sessionId;
    const request: CreationRequest = Object.freeze({ kind: "NEW_CLEAN", input: normalizedInput });
    const replay = this.#creationReplay(sessionId, request);
    if (replay !== undefined) return replay;
    const native = nativeFromFields(normalizedInput as unknown as RecordSnapshot);
    this.#assertIdentityAvailable(sessionId, native);
    const snapshot = cloneSnapshot({
      sessionId,
      lineage: {
        sessionId,
        bindingId: native.bindingId,
        parentRefs: [],
        operationKind: "NEW_CLEAN",
        inheritedExposure: emptyInheritedExposure(),
        sourceEpoch: native.sourceEpoch,
      },
      native,
      lifecycle: "ACTIVE",
      processState: "RUNNING",
      exposure: null,
      exposureAssessment: assess(null),
      pendingActions: [],
      materialHandoff: null,
      pendingActionDisposition: "NONE",
    });
    this.#sessions.set(sessionId, snapshot);
    this.#creationRequests.set(sessionId, request);
    return snapshot;
  }

  recordExposure(sessionId: string, input: ExposureReceipt): SessionSnapshot {
    const receipt = freezeReceipt(input);
    const canonicalSessionId = id(sessionId, "sessionId");
    const current = this.#require(canonicalSessionId);
    if (
      receipt.bindingId !== current.native.bindingId ||
      receipt.generation !== current.native.generation
    ) {
      throw new LineageError(
        "INVALID_EXPOSURE",
        "exposure receipt binding/generation does not match the session",
      );
    }
    const retained = this.#receipts.get(receipt.receiptId);
    if (retained !== undefined) {
      if (
        retained.sessionId === canonicalSessionId &&
        isDeepStrictEqual(retained.receipt, receipt)
      ) {
        return current;
      }
      throw new LineageError(
        "INVALID_EXPOSURE",
        "receiptId is already retained with different content or ownership",
      );
    }
    const next = cloneSnapshot({ ...current, exposure: receipt });
    this.#sessions.set(canonicalSessionId, next);
    this.#receipts.set(
      receipt.receiptId,
      Object.freeze({ sessionId: canonicalSessionId, receipt }),
    );
    return next;
  }

  retainPendingAction(sessionId: string, action: PendingActionRef): SessionSnapshot {
    const current = this.#require(sessionId);
    const pendingAction = freezePendingAction(action);
    if (
      pendingAction.bindingId !== current.native.bindingId ||
      pendingAction.generation !== current.native.generation
    ) {
      throw new LineageError(
        "NATIVE_IDENTITY_MISMATCH",
        "pending action belongs to a different binding generation",
      );
    }
    const pending = freezePendingActions([...current.pendingActions, pendingAction]);
    if (new Set(pending.map((item) => item.actionId)).size !== pending.length) {
      throw new LineageError("INVALID_INPUT", "pending actionId already exists");
    }
    const next = cloneSnapshot({
      ...current,
      pendingActions: pending,
      pendingActionDisposition:
        current.lineage.operationKind === "RESUME"
          ? "RETAINED_NOT_REPLAYED"
          : current.pendingActionDisposition,
    });
    this.#sessions.set(sessionId, next);
    return next;
  }

  resume(input: ResumeInput): SessionSnapshot {
    const normalizedInput = freezeResumeInput(input);
    const current = this.#require(normalizedInput.sessionId);
    const native = normalizedInput.native;
    if (!sameNative(current.native, native)) {
      throw new LineageError(
        "NATIVE_IDENTITY_MISMATCH",
        "RESUME requires the exact persisted native session, binding, domain and source epoch",
      );
    }
    if (current.lifecycle === "ARCHIVED") {
      throw new LineageError("INVALID_TRANSITION", "ARCHIVE does not implicitly resume a session");
    }
    const pendingActionDisposition =
      current.pendingActions.length === 0 ? "NONE" : "RETAINED_NOT_REPLAYED";
    if (
      current.lineage.operationKind === "RESUME" &&
      current.pendingActionDisposition === pendingActionDisposition
    ) {
      return current;
    }
    const next = cloneSnapshot({
      ...current,
      lineage: { ...current.lineage, operationKind: "RESUME" },
      pendingActionDisposition,
    });
    this.#sessions.set(normalizedInput.sessionId, next);
    return next;
  }

  fork(parentSessionId: string, input: ChildSessionInput, receiptIds: ReceiptIds): SessionSnapshot {
    const normalizedInput = freezeChildInput(input);
    if (normalizedInput.operation !== "NATIVE_FORK") {
      throw new LineageError("INVALID_TRANSITION", "fork requires operation NATIVE_FORK");
    }
    return this.#createChild(parentSessionId, normalizedInput, freezeReceiptIds(receiptIds));
  }

  rebuild(
    parentSessionId: string,
    input: ChildSessionInput,
    receiptIds: ReceiptIds,
  ): SessionSnapshot {
    const normalizedInput = freezeChildInput(input);
    if (normalizedInput.operation !== "REBUILD") {
      throw new LineageError("INVALID_TRANSITION", "rebuild requires operation REBUILD");
    }
    return this.#createChild(parentSessionId, normalizedInput, freezeReceiptIds(receiptIds));
  }

  handoff(parentSessionId: string, input: HandoffInput): SessionSnapshot {
    const normalizedParentId = id(parentSessionId, "parentSessionId");
    const parent = this.#require(normalizedParentId);
    const normalizedInput = freezeHandoffInput(input);
    const sessionId = normalizedInput.sessionId;
    const request: CreationRequest = Object.freeze({
      kind: "HANDOFF",
      parentSessionId: normalizedParentId,
      input: normalizedInput,
    });
    const replay = this.#creationReplay(sessionId, request);
    if (replay !== undefined) return replay;
    const native = nativeFromFields(normalizedInput as unknown as RecordSnapshot);
    const materialIds = normalizedInput.materialIds;
    if (materialIds.length === 0) return invalid("materialIds must not be empty");
    this.#assertIdentityAvailable(sessionId, native);
    const snapshot = cloneSnapshot({
      sessionId,
      lineage: {
        sessionId,
        bindingId: native.bindingId,
        parentRefs: [parent.sessionId],
        operationKind: "HANDOFF",
        inheritedExposure: mergeInheritedExposure(
          parent.lineage.inheritedExposure,
          parent.exposure,
        ),
        sourceEpoch: native.sourceEpoch,
      },
      native,
      lifecycle: "ACTIVE",
      processState: "RUNNING",
      exposure: null,
      exposureAssessment: assess(null),
      pendingActions: [],
      materialHandoff: { materialIds, sourceSessionId: parent.sessionId, nativeResumeUsed: false },
      pendingActionDisposition: parent.pendingActions.length === 0 ? "NONE" : "RETAINED_ON_PARENT",
    });
    this.#sessions.set(sessionId, snapshot);
    this.#creationRequests.set(sessionId, request);
    return snapshot;
  }

  archive(sessionId: string): SessionSnapshot {
    const current = this.#require(sessionId);
    const next = cloneSnapshot({ ...current, lifecycle: "ARCHIVED" });
    this.#sessions.set(sessionId, next);
    return next;
  }

  get(sessionId: string): SessionSnapshot {
    return this.#require(sessionId);
  }

  #createChild(
    parentSessionId: string,
    input: ChildSessionInput,
    receiptIds: ReceiptIds,
  ): SessionSnapshot {
    const canonicalParentId = id(parentSessionId, "parentSessionId");
    const parent = this.#require(canonicalParentId);
    const sessionId = input.sessionId;
    const request: CreationRequest = Object.freeze({
      kind: input.operation,
      parentSessionId: canonicalParentId,
      input,
      receiptIds,
    });
    const replay = this.#creationReplay(sessionId, request);
    if (replay !== undefined) return replay;
    const native = nativeFromFields(input as unknown as RecordSnapshot);
    if (native.nativeSessionId === parent.native.nativeSessionId) {
      throw new LineageError(
        "NATIVE_IDENTITY_MISMATCH",
        "child must have a distinct native session",
      );
    }
    this.#assertIdentityAvailable(sessionId, native);
    const inherited =
      parent.exposure === null
        ? null
        : inheritExposure(parent.exposure, {
            receiptId: receiptIds.receiptId,
            manifestId: receiptIds.manifestId,
            bindingId: native.bindingId,
            generation: native.generation,
          });
    if (inherited !== null) this.#assertReceiptAvailable(inherited, sessionId);
    const snapshot = cloneSnapshot({
      sessionId,
      lineage: {
        sessionId,
        bindingId: native.bindingId,
        parentRefs: [parent.sessionId],
        operationKind: input.operation,
        inheritedExposure: mergeInheritedExposure(
          parent.lineage.inheritedExposure,
          parent.exposure,
        ),
        sourceEpoch: native.sourceEpoch,
      },
      native,
      lifecycle: "ACTIVE",
      processState: "RUNNING",
      exposure: inherited,
      exposureAssessment: assess(inherited),
      pendingActions: [],
      materialHandoff: null,
      pendingActionDisposition: parent.pendingActions.length === 0 ? "NONE" : "RETAINED_ON_PARENT",
    });
    this.#sessions.set(sessionId, snapshot);
    this.#creationRequests.set(sessionId, request);
    if (inherited !== null) this.#retainReceipt(inherited, sessionId);
    return snapshot;
  }

  #creationReplay(sessionId: string, request: CreationRequest): SessionSnapshot | undefined {
    const current = this.#sessions.get(sessionId);
    if (current === undefined) return undefined;
    if (isDeepStrictEqual(this.#creationRequests.get(sessionId), request)) return current;
    throw new LineageError("DUPLICATE_SESSION", `session ${sessionId} already exists`);
  }

  #assertIdentityAvailable(sessionId: string, native: NativeSessionIdentity): void {
    for (const [ownerSessionId, retained] of this.#sessions) {
      if (ownerSessionId !== sessionId && sameNative(retained.native, native)) {
        throw new LineageError(
          "NATIVE_IDENTITY_MISMATCH",
          `native identity tuple is already retained by session ${ownerSessionId}`,
        );
      }
    }
  }

  #assertReceiptAvailable(receipt: ExposureReceipt, sessionId: string): void {
    const retained = this.#receipts.get(receipt.receiptId);
    if (retained === undefined) return;
    if (retained.sessionId === sessionId && isDeepStrictEqual(retained.receipt, receipt)) return;
    throw new LineageError(
      "INVALID_EXPOSURE",
      "receiptId is already retained with different content or ownership",
    );
  }

  #retainReceipt(receipt: ExposureReceipt, sessionId: string): void {
    this.#assertReceiptAvailable(receipt, sessionId);
    this.#receipts.set(receipt.receiptId, Object.freeze({ sessionId, receipt }));
  }

  #require(sessionId: string): SessionSnapshot {
    const canonical = id(sessionId, "sessionId");
    const session = this.#sessions.get(canonical);
    if (session === undefined)
      throw new LineageError("SESSION_NOT_FOUND", `session ${canonical} not found`);
    return session;
  }
}
