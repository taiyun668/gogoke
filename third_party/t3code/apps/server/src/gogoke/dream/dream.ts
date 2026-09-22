import { createHash } from "node:crypto";
import type {
  DreamProposal,
  DreamRun,
  DreamState,
  JsonObject,
  JsonValue,
} from "../contracts/model.ts";
import type { CalibrationKey } from "../evaluation/calibration.ts";
import { choice, integer, record, revision, text } from "../evaluation/boundary.ts";

export type DreamNamespace = "REAL" | "SYNTHETIC";
export type DreamCandidateKind =
  | "MEMORY_INDEX"
  | "CONTEXT_SUMMARY"
  | "DECISION_VIEW"
  | "QUESTION_SET"
  | "CANDIDATE_RULE"
  | "CALL_BATCH"
  | "CACHE_POLICY"
  | "THRESHOLD";

export interface DreamSchedulerFacts {
  readonly domainId: string;
  readonly namespace: DreamNamespace;
  readonly foregroundCritical: boolean;
  readonly urgentResumePending: boolean;
  readonly dangerousCustodyPending: boolean;
  readonly budgetAuthorized: boolean;
  readonly maintenanceWindowOpen: boolean;
  readonly dataAuthorized: boolean;
  readonly idleMillis: number;
  readonly minIdleMillis: number;
  readonly recentSamples: number;
  readonly minRecentSamples: number;
  readonly externalModelBudget: number;
  readonly maxSteps: number;
  readonly maxDurationMs: number;
  readonly preemptDeadlineMs: number;
  readonly budgetLease: JsonValue;
  readonly snapshotHash: string;
  readonly datasetSplitHash: string;
  readonly recipeRef: string;
}

export type DreamEligibility =
  | { readonly kind: "READY"; readonly task: DreamMaintenanceTask }
  | {
      readonly kind: "WAIT";
      readonly reason:
        | "FOREGROUND"
        | "URGENT_RESUME"
        | "DANGEROUS_CUSTODY"
        | "NO_BUDGET"
        | "WINDOW_CLOSED"
        | "DATA_DENIED"
        | "NOT_IDLE"
        | "INSUFFICIENT_SAMPLES"
        | "REAL_EXTERNAL_BUDGET_DISABLED"
        | "INVALID_LIMITS";
    };

export interface DreamMaintenanceTask {
  readonly kind: "maintenance";
  readonly concurrency: 1;
  readonly domainId: string;
  readonly namespace: DreamNamespace;
  readonly maxSteps: number;
  readonly maxDurationMs: number;
  readonly preemptDeadlineMs: number;
  readonly budgetLease: JsonValue;
  readonly snapshotHash: string;
  readonly datasetSplitHash: string;
  readonly recipeRef: string;
}

export interface DreamCandidateRequest {
  readonly namespace: DreamNamespace;
  readonly proposalId: string;
  readonly runId: string;
  readonly kind: DreamCandidateKind;
  readonly basePolicyRevision: string;
  readonly beforeHash: string;
  readonly afterHash: string;
  readonly evaluationRefs: ReadonlyArray<string>;
  readonly rollbackRef: string;
  readonly calibrationKey: CalibrationKey;
  readonly calibrationAutomaticUse: "NOT_AUTHORIZED";
  readonly safetyPrivacyIncidents: number;
  readonly safetyPrivacyUnknown: number;
  readonly heldoutReceipt: string | null;
}

const SHA = /^sha256:[0-9a-f]{64}$/u;
const CANDIDATE_KINDS: readonly DreamCandidateKind[] = Object.freeze([
  "MEMORY_INDEX",
  "CONTEXT_SUMMARY",
  "DECISION_VIEW",
  "QUESTION_SET",
  "CANDIDATE_RULE",
  "CALL_BATCH",
  "CACHE_POLICY",
  "THRESHOLD",
]);
const PROD_STATES: readonly DreamState[] = Object.freeze([
  "DRAFT",
  "DEV_VALIDATED",
  "CALIBRATED",
  "HOLDOUT_VALIDATED",
  "REJECTED",
  "RETIRED",
  "REVOKED",
]);
const TEST_STATES: readonly DreamState[] = Object.freeze([
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
]);
const ALLOWED_CHANGE_KEYS = Object.freeze(["kind", "beforeHash", "afterHash", "testOnly"] as const);

interface DreamCandidateIdentity {
  readonly namespace: DreamNamespace;
  readonly candidateHash: string;
  readonly kind: DreamCandidateKind;
  readonly beforeHash: string;
  readonly afterHash: string;
  readonly testOnly: boolean;
}

// Preparatory, process-local provenance only. This registry deliberately does
// not make DreamProposal serialization/restart durable and is not a signature,
// decoder, grant, or Product Authority. A future durable path must decode and
// bind authority independently rather than treating this registry as proof.
const dreamCandidateIdentities = new WeakMap<object, DreamCandidateIdentity>();

function exactAllowedChangeSet(value: JsonValue, expected: DreamCandidateIdentity): boolean {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const parent = Object.getPrototypeOf(value);
  if (parent !== null && parent !== Object.prototype) return false;
  const keys = Reflect.ownKeys(value);
  if (
    keys.length !== ALLOWED_CHANGE_KEYS.length ||
    keys.some(
      (key) =>
        typeof key !== "string" || !ALLOWED_CHANGE_KEYS.some((expectedKey) => expectedKey === key),
    )
  ) {
    return false;
  }
  const expectedValues: Readonly<Record<(typeof ALLOWED_CHANGE_KEYS)[number], JsonValue>> = {
    kind: expected.kind,
    beforeHash: expected.beforeHash,
    afterHash: expected.afterHash,
    testOnly: expected.testOnly,
  };
  for (const key of ALLOWED_CHANGE_KEYS) {
    const field = Object.getOwnPropertyDescriptor(value, key);
    if (!field || !("value" in field) || !field.enumerable || field.value !== expectedValues[key]) {
      return false;
    }
  }
  return true;
}

function registerDreamCandidate(
  proposal: DreamProposal,
  identity: DreamCandidateIdentity,
): DreamProposal {
  dreamCandidateIdentities.set(proposal, identity);
  return proposal;
}

const digest = (value: string): string =>
  `sha256:${createHash("sha256").update(value).digest("hex")}`;
const hash = (value: unknown): string => {
  const v = text(value);
  if (!SHA.test(v)) throw new Error("DREAM_INPUT_REJECTED");
  return v;
};
const boolean = (value: unknown): boolean => {
  if (typeof value !== "boolean") throw new Error("DREAM_INPUT_REJECTED");
  return value;
};
const json = (value: unknown): JsonValue => {
  const encoded = JSON.stringify(value);
  if (encoded === undefined) throw new Error("DREAM_INPUT_REJECTED");
  return JSON.parse(encoded) as JsonValue;
};

export function dreamEligibility(value: DreamSchedulerFacts): DreamEligibility {
  const raw = record(value, [
    "domainId",
    "namespace",
    "foregroundCritical",
    "urgentResumePending",
    "dangerousCustodyPending",
    "budgetAuthorized",
    "maintenanceWindowOpen",
    "dataAuthorized",
    "idleMillis",
    "minIdleMillis",
    "recentSamples",
    "minRecentSamples",
    "externalModelBudget",
    "maxSteps",
    "maxDurationMs",
    "preemptDeadlineMs",
    "budgetLease",
    "snapshotHash",
    "datasetSplitHash",
    "recipeRef",
  ]);
  const namespace = choice(raw.namespace, ["REAL", "SYNTHETIC"]);
  const foreground = boolean(raw.foregroundCritical),
    urgent = boolean(raw.urgentResumePending),
    custody = boolean(raw.dangerousCustodyPending),
    budget = boolean(raw.budgetAuthorized),
    windowOpen = boolean(raw.maintenanceWindowOpen),
    data = boolean(raw.dataAuthorized);
  const idle = integer(raw.idleMillis, 0, Number.MAX_SAFE_INTEGER),
    minIdle = integer(raw.minIdleMillis, 0, Number.MAX_SAFE_INTEGER);
  const samples = integer(raw.recentSamples, 0, Number.MAX_SAFE_INTEGER),
    minSamples = integer(raw.minRecentSamples, 0, Number.MAX_SAFE_INTEGER);
  const external = integer(raw.externalModelBudget, 0, Number.MAX_SAFE_INTEGER);
  const maxSteps = integer(raw.maxSteps, 1, 20),
    maxDurationMs = integer(raw.maxDurationMs, 1, 300_000),
    preemptDeadlineMs = integer(raw.preemptDeadlineMs, 1, 2_000);
  if (foreground) return Object.freeze({ kind: "WAIT" as const, reason: "FOREGROUND" as const });
  if (urgent) return Object.freeze({ kind: "WAIT" as const, reason: "URGENT_RESUME" as const });
  if (custody)
    return Object.freeze({ kind: "WAIT" as const, reason: "DANGEROUS_CUSTODY" as const });
  if (!budget) return Object.freeze({ kind: "WAIT" as const, reason: "NO_BUDGET" as const });
  if (!windowOpen)
    return Object.freeze({ kind: "WAIT" as const, reason: "WINDOW_CLOSED" as const });
  if (!data) return Object.freeze({ kind: "WAIT" as const, reason: "DATA_DENIED" as const });
  if (idle < minIdle) return Object.freeze({ kind: "WAIT" as const, reason: "NOT_IDLE" as const });
  if (samples < minSamples)
    return Object.freeze({ kind: "WAIT" as const, reason: "INSUFFICIENT_SAMPLES" as const });
  if (namespace === "REAL" && external !== 0) {
    return Object.freeze({
      kind: "WAIT" as const,
      reason: "REAL_EXTERNAL_BUDGET_DISABLED" as const,
    });
  }
  return Object.freeze({
    kind: "READY" as const,
    task: Object.freeze({
      kind: "maintenance" as const,
      concurrency: 1 as const,
      domainId: text(raw.domainId),
      namespace,
      maxSteps,
      maxDurationMs,
      preemptDeadlineMs,
      budgetLease: json(raw.budgetLease),
      snapshotHash: hash(raw.snapshotHash),
      datasetSplitHash: hash(raw.datasetSplitHash),
      recipeRef: text(raw.recipeRef),
    }),
  });
}

export function createDreamRun(input: {
  readonly runId: string;
  readonly task: DreamMaintenanceTask;
  readonly stepReceipts?: ReadonlyArray<string>;
  readonly preemptionState?: string;
}): DreamRun {
  const receipts = Object.freeze((input.stepReceipts ?? []).map(text));
  if (new Set(receipts).size !== receipts.length) throw new Error("DREAM_INPUT_REJECTED");
  return Object.freeze({
    runId: text(input.runId),
    snapshotHash: hash(input.task.snapshotHash),
    domainId: text(input.task.domainId),
    budgetLease: json(input.task.budgetLease),
    stepReceipts: receipts,
    preemptionState: text(input.preemptionState ?? "RUNNABLE"),
    datasetSplitHash: hash(input.task.datasetSplitHash),
    recipeRef: text(input.task.recipeRef),
  });
}

function calibrationKeyFrame(key: CalibrationKey): string {
  return [
    key.decisionFamily,
    key.modelVersion,
    key.questionVersion,
    key.viewVersion,
    key.criteriaVersion,
    key.candidateGeneratorVersion,
    key.candidateSetHash,
    key.locale,
    key.taskDomain,
    key.runtimeEnvironment,
  ]
    .map(text)
    .join("\u0000");
}

export function createDreamCandidate(input: DreamCandidateRequest): DreamProposal {
  if (input.calibrationAutomaticUse !== "NOT_AUTHORIZED") throw new Error("DREAM_INPUT_REJECTED");
  const namespace = choice(input.namespace, ["REAL", "SYNTHETIC"]);
  const kind = choice(input.kind, CANDIDATE_KINDS);
  const before = hash(input.beforeHash),
    after = hash(input.afterHash);
  if (before === after) throw new Error("DREAM_INPUT_REJECTED");
  const incidents = integer(input.safetyPrivacyIncidents, 0, Number.MAX_SAFE_INTEGER);
  const unknown = integer(input.safetyPrivacyUnknown, 0, Number.MAX_SAFE_INTEGER);
  if (incidents !== 0 || unknown !== 0) throw new Error("DREAM_INPUT_REJECTED");
  const refs = Object.freeze(input.evaluationRefs.map(text));
  if (refs.length === 0 || new Set(refs).size !== refs.length)
    throw new Error("DREAM_INPUT_REJECTED");
  const heldout = input.heldoutReceipt === null ? "PENDING" : text(input.heldoutReceipt);
  const base = revision(input.basePolicyRevision);
  const candidateHash = digest(
    [
      namespace,
      text(input.proposalId),
      text(input.runId),
      kind,
      base,
      before,
      after,
      calibrationKeyFrame(input.calibrationKey),
      ...refs,
    ].join("\u0000"),
  );
  const testOnly = namespace === "SYNTHETIC";
  const allowedChangeSet: JsonObject = Object.freeze({
    kind,
    beforeHash: before,
    afterHash: after,
    testOnly,
  });
  const proposal: DreamProposal = Object.freeze({
    proposalId: text(input.proposalId),
    runId: text(input.runId),
    kind,
    basePolicyRevision: base as DreamProposal["basePolicyRevision"],
    candidateHash,
    allowedChangeSet,
    evaluationRefs: refs,
    heldoutReceipt: heldout,
    state: "DRAFT",
    activationGrant: null,
    rollbackRef: text(input.rollbackRef),
  });
  return registerDreamCandidate(
    proposal,
    Object.freeze({
      namespace,
      candidateHash,
      kind,
      beforeHash: before,
      afterHash: after,
      testOnly,
    }),
  );
}

const nextState: Readonly<Record<string, readonly DreamState[]>> = Object.freeze({
  DRAFT: Object.freeze<DreamState[]>(["DEV_VALIDATED", "REJECTED"]),
  DEV_VALIDATED: Object.freeze<DreamState[]>(["CALIBRATED", "REJECTED"]),
  CALIBRATED: Object.freeze<DreamState[]>(["HOLDOUT_VALIDATED", "REJECTED"]),
  HOLDOUT_VALIDATED: Object.freeze<DreamState[]>(["SHADOW", "REJECTED"]),
  SHADOW: Object.freeze<DreamState[]>(["CANARY", "REJECTED", "REVOKED"]),
  CANARY: Object.freeze<DreamState[]>(["ACTIVE", "REJECTED", "REVOKED"]),
  ACTIVE: Object.freeze<DreamState[]>(["RETIRED", "REVOKED"]),
});

export function transitionDreamCandidate(input: {
  readonly namespace: DreamNamespace;
  readonly proposal: DreamProposal;
  readonly next: DreamState;
  readonly evidenceRef: string;
  readonly heldoutReceipt?: string;
  readonly activationGrant?: JsonValue;
}): DreamProposal {
  const namespace = choice(input.namespace, ["REAL", "SYNTHETIC"]);
  const identity = dreamCandidateIdentities.get(input.proposal);
  if (
    identity === undefined ||
    namespace !== identity.namespace ||
    input.proposal.candidateHash !== identity.candidateHash ||
    !exactAllowedChangeSet(input.proposal.allowedChangeSet, identity)
  ) {
    throw new Error("DREAM_TRANSITION_REJECTED");
  }
  const next = choice(input.next, namespace === "REAL" ? PROD_STATES : TEST_STATES);
  const allowed = nextState[input.proposal.state] ?? [];
  if (!allowed.includes(next)) throw new Error("DREAM_TRANSITION_REJECTED");
  if (namespace === "REAL" && ["SHADOW", "CANARY", "ACTIVE"].includes(next)) {
    throw new Error("DREAM_TRANSITION_REJECTED");
  }
  const evidence = text(input.evidenceRef);
  const refs = Object.freeze([...input.proposal.evaluationRefs, evidence]);
  if (new Set(refs).size !== refs.length) throw new Error("DREAM_TRANSITION_REJECTED");
  const heldout =
    next === "HOLDOUT_VALIDATED" ? text(input.heldoutReceipt) : input.proposal.heldoutReceipt;
  const activation = ["SHADOW", "CANARY", "ACTIVE"].includes(next)
    ? json(input.activationGrant)
    : null;
  if (namespace === "REAL" && activation !== null) throw new Error("DREAM_TRANSITION_REJECTED");
  const proposal: DreamProposal = Object.freeze({
    ...input.proposal,
    evaluationRefs: refs,
    heldoutReceipt: heldout,
    state: next,
    activationGrant: activation,
  });
  return registerDreamCandidate(proposal, identity);
}
