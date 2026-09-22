import { decisionScenario, type DecisionScenarioDefinition } from "../families/registry.ts";
import { DecisionEngineError, captureMethod, captureValue } from "./passive.ts";
import { snapshotRequest, snapshotEligibility, snapshotBackendKind, snapshotBackendResult, snapshotCommitResult } from "./decoders.ts";
export { DecisionEngineError } from "./passive.ts";

export type DecisionBackendKind = "RULES" | "FAKE" | "REPLAY" | "JEV" | "GENERATIVE";
export type DecisionControl = "NONE" | "WAIT" | "NEEDS_EVIDENCE" | "NEEDS_REASONING";

export interface DecisionRequest {
  readonly operationId: string;
  readonly scenarioId: string;
  readonly stateViewHash: string;
  readonly candidateHash: string;
  readonly questionVersion: string;
  readonly rubricVersion: string;
  readonly modelRequested: string | null;
  readonly taskRevision: string;
  readonly policyRevision: string;
  readonly capabilityRevision: string;
  readonly bindingGeneration: string;
  readonly budgetUnits: number;
  readonly deadlineEpochMs: number;
  readonly candidateRefs: ReadonlyArray<string>;
}

export interface CapacitySnapshot {
  readonly required: number;
  readonly available: number | null;
}

export interface EligibilityCandidate {
  readonly candidateId: string;
  readonly authorization: "ALLOWED" | "DENIED" | "UNKNOWN";
  readonly capability: "QUALIFIED" | "UNQUALIFIED" | "UNKNOWN";
  readonly isolation: "QUALIFIED" | "UNQUALIFIED" | "UNKNOWN";
  readonly capacity: CapacitySnapshot;
  readonly priorityClass: number;
  readonly waitingMs: number;
  readonly estimatedCost: number | null;
  readonly recipeRef: string;
  readonly resourceReservationRef: string;
  readonly actionIntentRef: string;
}

export interface EligibilitySnapshot {
  readonly taskRevision: string;
  readonly policyRevision: string;
  readonly capabilityRevision: string;
  readonly bindingGeneration: string;
  readonly candidates: ReadonlyArray<EligibilityCandidate>;
}

export interface DecisionEligibilityPort {
  resolveEligibility(request: DecisionRequest): Promise<EligibilitySnapshot | null>;
}

export interface RankedCandidate {
  readonly candidateId: string;
  readonly semanticRank: number;
}

export type DecisionBackendResult =
  | { readonly kind: "RANKED"; readonly modelResolved: string | null; readonly ranks: ReadonlyArray<RankedCandidate> }
  | { readonly kind: DecisionControl; readonly modelResolved: string | null };

export interface DecisionBackend {
  readonly kind: DecisionBackendKind;
  evaluate(input: {
    readonly scenario: DecisionScenarioDefinition;
    readonly request: DecisionRequest;
    readonly candidates: ReadonlyArray<EligibilityCandidate>;
  }): Promise<DecisionBackendResult>;
}

export interface DecisionRecord {
  readonly operationId: string;
  readonly scenarioId: string;
  readonly family: string;
  readonly state: "ABSTAINED" | "COMMITTED";
  readonly stateViewHash: string;
  readonly candidateHash: string;
  readonly questionVersion: string;
  readonly rubricVersion: string;
  readonly modelRequested: string | null;
  readonly modelResolved: string | null;
  readonly taskRevision: string;
  readonly policyRevision: string;
  readonly capabilityRevision: string;
  readonly bindingGeneration: string;
  readonly backendKind: DecisionBackendKind;
  readonly choice: string | null;
  readonly reason: string;
  readonly budgetUnits: number;
  readonly deadlineEpochMs: number;
}

export interface DecisionCommitCommand {
  readonly record: DecisionRecord;
  readonly reservation: {
    readonly candidateId: string;
    readonly resourceReservationRef: string;
  };
  readonly action: {
    readonly actionIntentRef: string;
  };
}

export type DecisionCommitResult =
  | { readonly kind: "committed"; readonly operationId: string; readonly decisionReceiptId: string }
  | { readonly kind: "replayed"; readonly operationId: string; readonly decisionReceiptId: string; readonly record: DecisionRecord }
  | { readonly kind: "denied" | "stale" | "conflict" };

export interface DecisionCommitPort {
  commitDecisionReservation(command: DecisionCommitCommand): Promise<DecisionCommitResult>;
}

export type DecisionResult =
  | { readonly kind: DecisionControl; readonly record: DecisionRecord }
  | { readonly kind: "COMMITTED" | "REPLAYED"; readonly record: DecisionRecord; readonly decisionReceiptId: string };

function sameBasis(request: DecisionRequest, snapshot: EligibilitySnapshot): boolean {
  return request.taskRevision === snapshot.taskRevision
    && request.policyRevision === snapshot.policyRevision
    && request.capabilityRevision === snapshot.capabilityRevision
    && request.bindingGeneration === snapshot.bindingGeneration;
}

function hardStatus(candidate: EligibilityCandidate): "ELIGIBLE" | "INELIGIBLE" | "UNKNOWN" {
  if (candidate.authorization === "DENIED" || candidate.capability === "UNQUALIFIED" || candidate.isolation === "UNQUALIFIED") return "INELIGIBLE";
  if (candidate.authorization === "UNKNOWN" || candidate.capability === "UNKNOWN" || candidate.isolation === "UNKNOWN" || candidate.capacity.available === null) return "UNKNOWN";
  return candidate.capacity.available < candidate.capacity.required ? "INELIGIBLE" : "ELIGIBLE";
}

function finiteRankMap(result: Extract<DecisionBackendResult,{kind:"RANKED"}>, candidates: ReadonlyArray<EligibilityCandidate>): Map<string,number> {
  const allowed = new Set(candidates.map((candidate) => candidate.candidateId));
  const ranks = new Map<string,number>();
  for (const entry of result.ranks) {
    if (!allowed.has(entry.candidateId) || ranks.has(entry.candidateId) || !Number.isFinite(entry.semanticRank)) {
      throw new DecisionEngineError("BACKEND_PROTOCOL");
    }
    ranks.set(entry.candidateId, entry.semanticRank);
  }
  if (ranks.size !== candidates.length) throw new DecisionEngineError("BACKEND_PROTOCOL");
  return ranks;
}

function compareCandidates(left: EligibilityCandidate, right: EligibilityCandidate, ranks: ReadonlyMap<string,number>): number {
  if (left.priorityClass !== right.priorityClass) return left.priorityClass - right.priorityClass;
  if (left.waitingMs !== right.waitingMs) return right.waitingMs - left.waitingMs;
  const semantic = (ranks.get(right.candidateId) ?? Number.NEGATIVE_INFINITY) - (ranks.get(left.candidateId) ?? Number.NEGATIVE_INFINITY);
  if (semantic !== 0) return semantic;
  if (left.estimatedCost !== null && right.estimatedCost !== null && left.estimatedCost !== right.estimatedCost) return left.estimatedCost - right.estimatedCost;
  if (left.estimatedCost === null && right.estimatedCost !== null) return 1;
  if (left.estimatedCost !== null && right.estimatedCost === null) return -1;
  return left.candidateId < right.candidateId ? -1 : left.candidateId > right.candidateId ? 1 : 0;
}

function record(
  request: DecisionRequest,
  scenario: DecisionScenarioDefinition,
  backend: DecisionBackend,
  state: "ABSTAINED" | "COMMITTED",
  choice: string | null,
  reason: string,
  modelResolved: string | null,
): DecisionRecord {
  return Object.freeze({
    operationId: request.operationId,
    scenarioId: scenario.id,
    family: scenario.family,
    state,
    stateViewHash: request.stateViewHash,
    candidateHash: request.candidateHash,
    questionVersion: request.questionVersion,
    rubricVersion: request.rubricVersion,
    modelRequested: request.modelRequested,
    modelResolved,
    taskRevision: request.taskRevision,
    policyRevision: request.policyRevision,
    capabilityRevision: request.capabilityRevision,
    bindingGeneration: request.bindingGeneration,
    backendKind: backend.kind,
    choice,
    reason,
    budgetUnits: request.budgetUnits,
    deadlineEpochMs: request.deadlineEpochMs,
  });
}

/** Preparatory coordination, not a production grant or atomic reservation authority.
 * Collaborators must deliver parsed passive data; Promise resolution itself can
 * inspect a top-level then property before this consumer obtains the value.
 */
export class DecisionEngine {
  readonly #eligibility: DecisionEligibilityPort;
  readonly #backend: DecisionBackend;
  readonly #commit: DecisionCommitPort;

  constructor(eligibility: DecisionEligibilityPort, backend: DecisionBackend, commit: DecisionCommitPort) {
    this.#eligibility = Object.freeze({ resolveEligibility:
      captureMethod<DecisionEligibilityPort["resolveEligibility"]>(eligibility,"resolveEligibility") });
    this.#backend = Object.freeze({ kind:snapshotBackendKind(captureValue(backend,"kind")),
      evaluate:captureMethod<DecisionBackend["evaluate"]>(backend,"evaluate") });
    this.#commit = Object.freeze({ commitDecisionReservation:
      captureMethod<DecisionCommitPort["commitDecisionReservation"]>(commit,"commitDecisionReservation") });
  }

  async decide(input: DecisionRequest): Promise<DecisionResult> {
    const request = snapshotRequest(input);
    const scenario = decisionScenario(request.scenarioId);
    if (scenario === null) throw new DecisionEngineError("INVALID_INPUT");
    const abstain = (kind: DecisionControl, reason = kind, modelResolved: string | null = null): DecisionResult =>
      Object.freeze({ kind, record:record(request,scenario,this.#backend,"ABSTAINED",null,reason,modelResolved) });
    // GN remains closed: injection or a model name never establishes egress rights.
    if (this.#backend.kind === "JEV" || this.#backend.kind === "GENERATIVE") return abstain("NEEDS_EVIDENCE");

    let eligibilityValue: unknown;
    try { eligibilityValue = await this.#eligibility.resolveEligibility(request); }
    catch { return abstain("NEEDS_EVIDENCE"); }
    const snapshot = snapshotEligibility(eligibilityValue);
    if (snapshot === null) return abstain("NEEDS_EVIDENCE");
    if (!sameBasis(request,snapshot)) throw new DecisionEngineError("STALE_VIEW");
    const requested = new Set(request.candidateRefs);
    if (snapshot.candidates.length !== requested.size ||
        snapshot.candidates.some(candidate => !requested.has(candidate.candidateId))) {
      throw new DecisionEngineError("BACKEND_PROTOCOL");
    }
    const statuses = snapshot.candidates.map(candidate => [candidate,hardStatus(candidate)] as const);
    const eligible = Object.freeze(statuses.filter(([,status]) => status === "ELIGIBLE").map(([candidate]) => candidate));
    if (eligible.length === 0) return abstain(statuses.some(([,status]) => status === "UNKNOWN") ? "NEEDS_EVIDENCE" : "NONE");

    let backendValue: unknown;
    try { backendValue = await this.#backend.evaluate(Object.freeze({ scenario, request, candidates:eligible })); }
    catch { return abstain("NEEDS_EVIDENCE"); }
    const backend = snapshotBackendResult(backendValue);
    if (backend.kind !== "RANKED") return abstain(backend.kind,backend.kind,backend.modelResolved);
    const ranks = finiteRankMap(backend,eligible);
    // Ranking evidence cannot expand a scenario's fixed execution ceiling.
    // Proposal/replay-only scenarios use non-dispatch proposal/read paths; this
    // action-creating kernel must not manufacture an intent for them.
    if (scenario.s1Mode !== "fixture_bounded_auto") {
      return abstain("NEEDS_REASONING", "NEEDS_REASONING", backend.modelResolved);
    }
    const selected = [...eligible].sort((left,right) => compareCandidates(left,right,ranks))[0];
    if (!selected) throw new DecisionEngineError("BACKEND_PROTOCOL");
    const committed = record(request,scenario,this.#backend,"COMMITTED",selected.candidateId,"QUALIFIED_BOUNDED_SELECTION",backend.modelResolved);
    let commitValue: unknown;
    try {
      commitValue = await this.#commit.commitDecisionReservation(Object.freeze({ record:committed,
        reservation:Object.freeze({ candidateId:selected.candidateId, resourceReservationRef:selected.resourceReservationRef }),
        action:Object.freeze({ actionIntentRef:selected.actionIntentRef }) }));
    } catch { throw new DecisionEngineError("COMMIT_UNKNOWN"); }
    const outcome = snapshotCommitResult(commitValue);
    if (outcome.kind === "committed") {
      if (outcome.operationId !== request.operationId) throw new DecisionEngineError("COMMIT_UNKNOWN");
      return Object.freeze({ kind:"COMMITTED" as const, record:committed,
        decisionReceiptId:outcome.decisionReceiptId });
    }
    if (outcome.kind === "replayed") {
      if (outcome.operationId !== request.operationId || outcome.record.operationId !== request.operationId) {
        throw new DecisionEngineError("COMMIT_UNKNOWN");
      }
      // Replay is authoritative history, not this call's recomputed semantic choice.
      return Object.freeze({ kind:"REPLAYED" as const, record:outcome.record,
        decisionReceiptId:outcome.decisionReceiptId });
    }
    if (outcome.kind === "denied") throw new DecisionEngineError("COMMIT_DENIED");
    if (outcome.kind === "stale") throw new DecisionEngineError("STALE_VIEW");
    throw new DecisionEngineError("COMMIT_CONFLICT");
  }
}
