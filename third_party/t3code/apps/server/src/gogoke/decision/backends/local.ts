import { Buffer } from "node:buffer";
import type { DecisionBackend, DecisionBackendResult } from "../engine/engine.ts";
import { snapshotBackendResult, snapshotEligibility, snapshotRequest } from "../engine/decoders.ts";
import { arrayData, recordData, DecisionEngineError } from "../engine/passive.ts";
import { decisionScenario, type DecisionScenarioDefinition } from "../families/registry.ts";

export type DecisionBackendInput = Parameters<DecisionBackend["evaluate"]>[0];
export interface LocalDecisionTranscript {
  readonly input: DecisionBackendInput;
  readonly result: DecisionBackendResult;
}
const SCENARIO_KEYS = Object.freeze(["id", "key", "family", "purpose", "ceiling", "s1Mode", "liveStatus"] as const);
const MAX_BYTES = 1024 * 1024;
const byteLength = Buffer.byteLength;
const reject = (): never => { throw new DecisionEngineError("BACKEND_PROTOCOL"); };
const data = (value: unknown, keys: readonly string[]) => recordData(value, keys, [], reject);

/** This codec checks identity/shape, not whether any candidate is authorized.
 * Only the existing current eligibility/commit authority may grant those rights.
 */
function inputSnapshot(value: unknown): DecisionBackendInput {
  const raw = data(value, ["scenario", "request", "candidates"]);
  const request = snapshotRequest(raw.request);
  const definition = decisionScenario(request.scenarioId);
  if (definition === null) return reject();
  const scenario = data(raw.scenario, SCENARIO_KEYS);
  for (const key of SCENARIO_KEYS) if (scenario[key] !== definition[key]) return reject();
  const decoded = snapshotEligibility({ taskRevision: request.taskRevision, policyRevision: request.policyRevision,
    capabilityRevision: request.capabilityRevision, bindingGeneration: request.bindingGeneration,
    candidates: raw.candidates });
  if (decoded === null) return reject();
  const requested = new Set(request.candidateRefs);
  if (decoded.candidates.some(candidate => !requested.has(candidate.candidateId))) return reject();
  return Object.freeze({ scenario: definition, request, candidates: decoded.candidates });
}

// Length-prefixed primitive fields avoid custom toJSON methods or ambiguous
// string concatenation. All fields come from fresh passive snapshots above.
function pack(values: readonly (string | number | null)[]): string {
  return values.map(value => {
    const text = value === null ? "" : String(value);
    const type = value === null ? "n" : typeof value === "number" ? "d" : "s";
    return `${type}${text.length}:${text}`;
  }).join("");
}
function keyOf(input: DecisionBackendInput): string {
  const r = input.request;
  const values: Array<string | number | null> = ["gogoke.local-decision-transcript.v1"];
  for (const key of SCENARIO_KEYS) values.push(input.scenario[key]);
  for (const key of ["operationId", "scenarioId", "stateViewHash", "candidateHash", "questionVersion", "rubricVersion",
    "modelRequested", "taskRevision", "policyRevision", "capabilityRevision", "bindingGeneration", "budgetUnits", "deadlineEpochMs"] as const) {
    values.push(r[key]);
  }
  values.push(r.candidateRefs.length);
  for (const ref of r.candidateRefs) values.push(ref);
  values.push(input.candidates.length);
  for (const c of input.candidates) {
    values.push(c.candidateId, c.authorization, c.capability, c.isolation, c.capacity.required, c.capacity.available,
      c.priorityClass, c.waitingMs, c.estimatedCost, c.recipeRef, c.resourceReservationRef, c.actionIntentRef);
  }
  const key = pack(values);
  if (byteLength(key, "utf8") > MAX_BYTES) return reject();
  return key;
}
function resultSize(result: DecisionBackendResult): number {
  const values: Array<string | number | null> = [result.kind, result.modelResolved];
  if (result.kind === "RANKED") for (const rank of result.ranks) values.push(rank.candidateId, rank.semanticRank);
  return byteLength(pack(values), "utf8");
}
function sealResult(value: unknown, input: DecisionBackendInput): DecisionBackendResult {
  const result = snapshotBackendResult(value);
  if (result.kind === "RANKED") {
    const expected = new Set(input.candidates.map(c => c.candidateId));
    if (result.ranks.length !== expected.size) return reject();
    for (const rank of result.ranks) if (!expected.delete(rank.candidateId)) return reject();
    if (expected.size !== 0) return reject();
  }
  // The Promise result itself must not inherit an unrelated then accessor.
  // Nested arrays/rows are already frozen by the existing result decoder.
  return Object.freeze(Object.assign(Object.create(null), result)) as DecisionBackendResult;
}
const MISSING: DecisionBackendResult = Object.freeze(Object.assign(Object.create(null),
  { kind: "NEEDS_EVIDENCE", modelResolved: null })) as DecisionBackendResult;

/** Deterministic no-model fallback. Equal semantic ranks leave priority, waiting,
 * fairness and cost ordering to the ONE existing DecisionEngine, not a scheduler here.
 */
export function createRulesBackend(): DecisionBackend {
  return Object.freeze({ kind: "RULES", async evaluate(value: DecisionBackendInput) {
    const input = inputSnapshot(value);
    keyOf(input); // Bound the full passive input before constructing output.
    if (input.candidates.length === 0) return sealResult({ kind: "NONE", modelResolved: null }, input);
    return sealResult({ kind: "RANKED", modelResolved: null,
      ranks: input.candidates.map(candidate => ({ candidateId: candidate.candidateId, semanticRank: 0 })) }, input);
  } });
}

/** Local controlled transcripts only. Not a grant cache, production calibration,
 * durable Decision receipt or permission to dispatch. Engine eligibility is read
 * anew before invocation; its native commit must recheck current authority again.
 * Exact input mismatch abstains, including changed capacity, recipe, action,
 * request order, revisions, model, question, budget or deadline. No fuzzy match.
 */
function transcriptBackend(kind: "FAKE" | "REPLAY", entries: readonly LocalDecisionTranscript[]): DecisionBackend {
  const snapshots = arrayData(entries, reject);
  const records = new Map<string, DecisionBackendResult>();
  let bytes = 0;
  for (const value of snapshots) {
    const entry = data(value, ["input", "result"]);
    const input = inputSnapshot(entry.input);
    const key = keyOf(input);
    const result = sealResult(entry.result, input);
    bytes += byteLength(key, "utf8") + resultSize(result);
    if (bytes > MAX_BYTES || records.has(key)) return reject();
    records.set(key, result);
  }
  return Object.freeze({ kind, async evaluate(value: DecisionBackendInput) {
    const input = inputSnapshot(value);
    return records.get(keyOf(input)) ?? MISSING;
  } });
}
export function createFixtureBackend(entries: readonly LocalDecisionTranscript[]): DecisionBackend {
  return transcriptBackend("FAKE", entries);
}
export function createReplayBackend(entries: readonly LocalDecisionTranscript[]): DecisionBackend {
  return transcriptBackend("REPLAY", entries);
}

// Compile-time check that every fixed registry field participates in identity.
type MissingScenarioField = Exclude<keyof DecisionScenarioDefinition, typeof SCENARIO_KEYS[number]>;
const completeScenarioKeySet: MissingScenarioField extends never ? true : never = true;
void completeScenarioKeySet;
