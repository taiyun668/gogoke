import type { DecisionRequest, DecisionRecord, EligibilityCandidate, EligibilitySnapshot,
  DecisionBackendKind, DecisionBackendResult, DecisionCommitResult } from "./engine.ts";
import { DecisionEngineError, arrayData, recordData } from "./passive.ts";

type Code = "INVALID_INPUT" | "BACKEND_PROTOCOL" | "COMMIT_UNKNOWN";
const reject = (code: Code): never => { throw new DecisionEngineError(code); };
const rec = (value: unknown, keys: readonly string[], code: Code, optional: readonly string[] = []) =>
  recordData(value, keys, optional, () => reject(code));
const arr = (value: unknown, code: Code) => arrayData(value, () => reject(code));
function text(value: unknown, code: Code): string {
  if (typeof value !== "string" || value.length === 0 || value.length > 4096 || value !== value.trim()) return reject(code);
  return value;
}
function u64(value: unknown, code: Code): string {
  const valueText = text(value, code);
  if (!/^(0|[1-9][0-9]*)$/.test(valueText) || valueText.length > 20 || BigInt(valueText) > 18446744073709551615n) return reject(code);
  return valueText;
}
function count(value: unknown, code: Code): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) return reject(code);
  return value;
}
function choice<const T extends string>(value: unknown, values: readonly T[], code: Code): T {
  for (const candidate of values) if (value === candidate) return candidate;
  return reject(code);
}
function model(value: unknown, code: Code): string | null {
  return value === null ? null : text(value, code);
}

export function snapshotRequest(value: unknown): DecisionRequest {
  const code = "INVALID_INPUT";
  const raw = rec(value, ["operationId","scenarioId","stateViewHash","candidateHash","questionVersion",
    "rubricVersion","modelRequested","taskRevision","policyRevision","capabilityRevision",
    "bindingGeneration","budgetUnits","deadlineEpochMs","candidateRefs"], code);
  const candidateRefs = arr(raw.candidateRefs, code).map(value => text(value, code));
  if (candidateRefs.length === 0 || new Set(candidateRefs).size !== candidateRefs.length) return reject(code);
  return Object.freeze({ operationId:text(raw.operationId,code), scenarioId:text(raw.scenarioId,code),
    stateViewHash:text(raw.stateViewHash,code), candidateHash:text(raw.candidateHash,code),
    questionVersion:text(raw.questionVersion,code), rubricVersion:text(raw.rubricVersion,code),
    modelRequested:model(raw.modelRequested,code), taskRevision:u64(raw.taskRevision,code),
    policyRevision:u64(raw.policyRevision,code), capabilityRevision:u64(raw.capabilityRevision,code),
    bindingGeneration:u64(raw.bindingGeneration,code), budgetUnits:count(raw.budgetUnits,code),
    deadlineEpochMs:count(raw.deadlineEpochMs,code), candidateRefs:Object.freeze(candidateRefs) });
}
function candidate(value: unknown): EligibilityCandidate {
  const code = "BACKEND_PROTOCOL";
  const raw = rec(value,["candidateId","authorization","capability","isolation","capacity","priorityClass",
    "waitingMs","estimatedCost","recipeRef","resourceReservationRef","actionIntentRef"],code);
  const capacity = rec(raw.capacity,["required","available"],code);
  const estimatedCost = raw.estimatedCost;
  if (estimatedCost !== null && (typeof estimatedCost !== "number" || !Number.isFinite(estimatedCost) ||
      estimatedCost < 0 || estimatedCost > Number.MAX_SAFE_INTEGER)) return reject(code);
  return Object.freeze({ candidateId:text(raw.candidateId,code),
    authorization:choice(raw.authorization,["ALLOWED","DENIED","UNKNOWN"],code),
    capability:choice(raw.capability,["QUALIFIED","UNQUALIFIED","UNKNOWN"],code),
    isolation:choice(raw.isolation,["QUALIFIED","UNQUALIFIED","UNKNOWN"],code),
    capacity:Object.freeze({ required:count(capacity.required,code),
      available:capacity.available === null ? null : count(capacity.available,code) }),
    priorityClass:count(raw.priorityClass,code), waitingMs:count(raw.waitingMs,code), estimatedCost,
    recipeRef:text(raw.recipeRef,code), resourceReservationRef:text(raw.resourceReservationRef,code),
    actionIntentRef:text(raw.actionIntentRef,code) });
}
export function snapshotEligibility(value: unknown): EligibilitySnapshot | null {
  if (value === null) return null;
  const code = "BACKEND_PROTOCOL";
  const raw = rec(value,["taskRevision","policyRevision","capabilityRevision","bindingGeneration","candidates"],code);
  const candidates = arr(raw.candidates,code).map(candidate);
  if (new Set(candidates.map(value => value.candidateId)).size !== candidates.length) return reject(code);
  return Object.freeze({ taskRevision:u64(raw.taskRevision,code), policyRevision:u64(raw.policyRevision,code),
    capabilityRevision:u64(raw.capabilityRevision,code), bindingGeneration:u64(raw.bindingGeneration,code),
    candidates:Object.freeze(candidates) });
}
export function snapshotBackendKind(value: unknown): DecisionBackendKind {
  return choice(value,["RULES","FAKE","REPLAY","JEV","GENERATIVE"],"INVALID_INPUT");
}
export function snapshotBackendResult(value: unknown): DecisionBackendResult {
  const code = "BACKEND_PROTOCOL";
  const raw = rec(value,["kind","modelResolved"],code,["ranks"]);
  const modelResolved = model(raw.modelResolved,code);
  if (raw.kind === "RANKED") {
    rec(value,["kind","modelResolved","ranks"],code);
    const ranks = arr(raw.ranks,code).map(value => {
      const rank = rec(value,["candidateId","semanticRank"],code);
      if (typeof rank.semanticRank !== "number" || !Number.isFinite(rank.semanticRank)) return reject(code);
      return Object.freeze({ candidateId:text(rank.candidateId,code), semanticRank:rank.semanticRank });
    });
    return Object.freeze({ kind:"RANKED", modelResolved, ranks:Object.freeze(ranks) });
  }
  rec(value,["kind","modelResolved"],code);
  return Object.freeze({ kind:choice(raw.kind,["NONE","WAIT","NEEDS_EVIDENCE","NEEDS_REASONING"],code), modelResolved });
}
function decisionRecord(value: unknown, code: Code): DecisionRecord {
  const raw = rec(value,["operationId","scenarioId","family","state","stateViewHash","candidateHash",
    "questionVersion","rubricVersion","modelRequested","modelResolved","taskRevision","policyRevision",
    "capabilityRevision","bindingGeneration","backendKind","choice","reason","budgetUnits","deadlineEpochMs"],code);
  const state = choice(raw.state,["ABSTAINED","COMMITTED"],code);
  return Object.freeze({
    operationId:text(raw.operationId,code), scenarioId:text(raw.scenarioId,code), family:text(raw.family,code),
    state, stateViewHash:text(raw.stateViewHash,code), candidateHash:text(raw.candidateHash,code),
    questionVersion:text(raw.questionVersion,code), rubricVersion:text(raw.rubricVersion,code),
    modelRequested:model(raw.modelRequested,code), modelResolved:model(raw.modelResolved,code),
    taskRevision:u64(raw.taskRevision,code), policyRevision:u64(raw.policyRevision,code),
    capabilityRevision:u64(raw.capabilityRevision,code), bindingGeneration:u64(raw.bindingGeneration,code),
    backendKind:choice(raw.backendKind,["RULES","FAKE","REPLAY","JEV","GENERATIVE"],code),
    choice:raw.choice === null ? null : text(raw.choice,code), reason:text(raw.reason,code),
    budgetUnits:count(raw.budgetUnits,code), deadlineEpochMs:count(raw.deadlineEpochMs,code),
  });
}
export function snapshotCommitResult(value: unknown): DecisionCommitResult {
  const code = "COMMIT_UNKNOWN";
  const raw = rec(value,["kind"],code,["operationId","decisionReceiptId","record"]);
  if (raw.kind === "committed") {
    rec(value,["kind","operationId","decisionReceiptId"],code);
    return Object.freeze({ kind:"committed" as const, operationId:text(raw.operationId,code),
      decisionReceiptId:text(raw.decisionReceiptId,code) });
  }
  if (raw.kind === "replayed") {
    rec(value,["kind","operationId","decisionReceiptId","record"],code);
    const record = decisionRecord(raw.record,code);
    if (record.state !== "COMMITTED" || record.choice === null) return reject(code);
    return Object.freeze({ kind:"replayed" as const, operationId:text(raw.operationId,code),
      decisionReceiptId:text(raw.decisionReceiptId,code), record });
  }
  rec(value,["kind"],code);
  return Object.freeze({ kind:choice(raw.kind,["denied","stale","conflict"],code) });
}
