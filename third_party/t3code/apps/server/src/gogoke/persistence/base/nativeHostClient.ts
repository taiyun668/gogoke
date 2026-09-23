import * as NodeChildProcess from "node:child_process";
import * as NodeFS from "node:fs";
import * as NodeReadline from "node:readline";

import type {
  CommitContextVersionRequest,
  ContextCommitReceipt,
} from "../../context/repository/repository.ts";
import type {
  BeginActionResult,
  DurableActionReservation,
  DurableDispatchOutcome,
  ReserveActionResult,
} from "../../actions/typedAction.ts";
import { parseStrictJsonBytes } from "../../contracts/strictJson.ts";

export class NativeHostClientError extends Error {
  override readonly name = "NativeHostClientError";
  readonly code: string;

  constructor(code: string, detail: string) {
    super(`${code}: ${detail}`);
    this.code = code;
  }
}

export type NativeHostReply = {
  readonly ok: boolean;
  readonly body: string;
  readonly elapsedMicros: number;
};

export interface NativeProductIdentitySnapshot {
  readonly policyRevision: string;
  readonly principalId: string;
  readonly profileId: string;
  readonly revocationHead: string;
  readonly rootIdentity: string;
  readonly seatId: string;
}

const decodeProductIdentity = (body: string): NativeProductIdentitySnapshot => {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("PRODUCT_IDENTITY_REPLY", "identity reply must be an object");
  }
  const record = value as Record<string, unknown>;
  const keys = [
    "policyRevision",
    "principalId",
    "profileId",
    "revocationHead",
    "rootIdentity",
    "seatId",
  ] as const;
  const ownKeys = Reflect.ownKeys(record);
  if (
    ownKeys.length !== keys.length ||
    ownKeys.some((key) => typeof key !== "string" || !keys.includes(key as (typeof keys)[number])) ||
    keys.some(
      (key) =>
        typeof record[key] !== "string" ||
        (record[key] as string).length === 0 ||
        record[key] !== (record[key] as string).trim(),
    )
  ) {
    throw new NativeHostClientError("PRODUCT_IDENTITY_REPLY", "identity reply fields are invalid");
  }
  if (
    !/^(?:0|[1-9][0-9]*)$/.test(record.policyRevision as string) ||
    !/^(?:0|[1-9][0-9]*)$/.test(record.revocationHead as string)
  ) {
    throw new NativeHostClientError("PRODUCT_IDENTITY_REPLY", "identity revisions are not canonical");
  }
  return Object.freeze({
    policyRevision: record.policyRevision as string,
    principalId: record.principalId as string,
    profileId: record.profileId as string,
    revocationHead: record.revocationHead as string,
    rootIdentity: record.rootIdentity as string,
    seatId: record.seatId as string,
  });
};

export interface NativeDecisionAuthoritySnapshot {
  readonly operationId: string;
  readonly candidateId: string;
  readonly stateViewHash: string;
  readonly candidateHash: string;
  readonly taskRevision: string;
  readonly policyRevision: string;
  readonly capabilityRevision: string;
  readonly bindingId: string;
  readonly bindingGeneration: string;
  readonly authRevision: string;
  readonly resourceRef: string;
  readonly resourceRevision: string;
  readonly capacityTotal: number;
  readonly actionOperationId: string;
  readonly actionDigest: string;
}

export interface NativeDecisionRecord {
  readonly operationId: string;
  readonly scenarioId: string;
  readonly family: string;
  readonly state: "COMMITTED";
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
  readonly backendKind: "RULES" | "FAKE" | "REPLAY";
  readonly choice: string;
  readonly reason: "QUALIFIED_BOUNDED_SELECTION";
  readonly budgetUnits: number;
  readonly deadlineEpochMs: number;
}

export interface NativeDecisionCommitRequest {
  readonly domainId: string;
  readonly decisionId: string;
  readonly eventId: string;
  readonly receiptId: string;
  readonly recordedAt: string;
  readonly record: NativeDecisionRecord;
  readonly resourceReservationRef: string;
  readonly actionIntentRef: string;
  readonly requiredCapacityUnits: number;
}

export type NativeDecisionCommitReply =
  | { readonly kind: "committed"; readonly operationId: string; readonly decisionReceiptId: string }
  | {
      readonly kind: "replayed";
      readonly operationId: string;
      readonly decisionReceiptId: string;
      readonly record: NativeDecisionRecord;
    };

export interface NativeContextAssemblySnapshot {
  readonly operationId: string;
  readonly principalId: string;
  readonly seatId: string;
  readonly taskId: string;
  readonly sessionId: string;
  readonly domainId: string;
  readonly bindingId: string;
  readonly bindingGeneration: string;
  readonly sourceEpoch: string;
  readonly runtimeInstanceId: string;
  readonly taskRevision: string;
  readonly policyRevision: string;
  readonly authRevision: string;
  readonly revocationHead: string;
  readonly selectionDecisionId: string;
  readonly manifestId: string;
  readonly admissionActionOperationId: string;
  readonly admissionDigest: string;
  readonly maxContentBytes: number;
  readonly maxCandidates: number;
  readonly partitionBindings: ReadonlyArray<NativeContextPartitionBinding>;
}

export interface NativeContextPartitionBinding {
  readonly sourceDomainId: string;
  readonly destinationScope: "GLOBAL" | "PROJECT" | "SESSION";
  readonly promotionKind: string;
  readonly grant: NativeGrantRef;
}

export interface NativeContextAssemblyBasis {
  readonly operationId: string;
  readonly bindingGeneration: string;
  readonly sourceEpoch: string;
  readonly taskRevision: string;
  readonly policyRevision: string;
  readonly authRevision: string;
  readonly revocationHead: string;
  readonly maxContentBytes: number;
  readonly maxCandidates: number;
  readonly partitionBindings: ReadonlyArray<NativeContextPartitionBinding>;
  readonly mandatoryRefs: ReadonlyArray<TaskMandatoryContextRef>;
}

export interface TaskMandatoryContextRef {
  readonly sourceDomainId: string;
  readonly contextId: string;
  readonly version: string;
}

export interface CommitTaskContextRequirements {
  readonly operationId: string;
  readonly domainId: string;
  readonly taskId: string;
  readonly expectedPreviousTaskRevision: string | null;
  readonly mandatoryRefs: ReadonlyArray<TaskMandatoryContextRef>;
  readonly eventId: string;
  readonly receiptId: string;
  readonly recordedAt: string;
}

export interface ReadTaskContextRequirements {
  readonly domainId: string;
  readonly taskId: string;
}

export interface TaskContextRequirements extends ReadTaskContextRequirements {
  readonly taskRevision: string;
  readonly contentHash: string;
  readonly mandatoryRefs: ReadonlyArray<TaskMandatoryContextRef>;
}

export interface TaskContextRequirementsReceipt extends TaskContextRequirements {
  readonly disposition: "COMMITTED" | "RECONCILED";
  readonly operationId: string;
}

export interface NativeContextAssemblySource {
  readonly sourceDomainId: string;
  readonly contextId: string;
  readonly version: string;
  readonly scope: string;
  readonly kind: string;
  readonly contentHash: string;
  readonly sourceRef: string;
  readonly sourceHash: string;
  readonly sourceAuthorityKind: string;
  readonly sourceAuthorityRef: string;
  readonly accessPolicyRevision: string;
  readonly stateRevision: string;
  readonly grant: NativeGrantRef;
}

export interface NativeGrantRef {
  readonly grantId: string;
  readonly revision: string;
  readonly revocationHead: string;
}

export interface NativeDelegationGrantSnapshot {
  readonly grantRef: string;
  readonly revision: string;
  readonly revocationHead: string;
  readonly policyRevision: string;
  readonly seatId: string;
  readonly issuerId: string;
  readonly parentGrant: { readonly grantRef: string; readonly revision: string } | null;
  readonly principal: {
    readonly principalId: string;
    readonly projectId: string;
    readonly domainId: string;
    readonly role: string;
    readonly seatId: string;
  };
  readonly binding: {
    readonly sessionId: string;
    readonly executionId: string;
    readonly generation: string;
  };
  readonly expiresAtEpochMs: string;
  readonly ceiling: {
    readonly allowedActions: ReadonlyArray<string>;
    readonly allowedTargetPrincipalIds: ReadonlyArray<string>;
    readonly allowedTargetDomainIds: ReadonlyArray<string>;
    readonly allowedSinks: ReadonlyArray<string>;
    readonly allowedMaterialClasses: ReadonlyArray<string>;
    readonly explicitPrivateMaterialIds: ReadonlyArray<string>;
    readonly allowedContinuationResponses: ReadonlyArray<string>;
    readonly maxMaterialItems: string;
    readonly maxMaterialBytes: string;
    readonly maxResponseBytes: string;
  };
}

export interface NativeContextReadRequest {
  readonly sourceDomainId: string;
  readonly contextId: string;
  readonly version: string;
  readonly expectedScope: string;
  readonly expectedContentHash: string;
  readonly expectedAccessPolicyRevision: string;
  readonly destinationDomainId: string;
  readonly destinationScope: string;
  readonly promotionKind: string;
  readonly policyRevision: string;
  readonly grant: NativeGrantRef;
}

export interface NativeGranteeContextReadRequest {
  readonly principalId: string;
  readonly seatId: string;
  readonly source: NativeContextReadRequest;
}

export interface NativeAuthorizedContextReadSource {
  readonly grantRevision: string;
  readonly revocationHead: string;
  readonly state: string;
  readonly stateRevision: string;
  readonly sourceDomainId: string;
  readonly contextId: string;
  readonly version: string;
  readonly scope: string;
  readonly kind: string;
  readonly contentHash: string;
  readonly sourceRef: string;
  readonly sourceHash: string;
  readonly sourceAuthorityKind: string;
  readonly sourceAuthorityRef: string;
  readonly accessPolicyRevision: string;
}

export interface NativeAuthorizedContextReadSet {
  readonly principalId: string;
  readonly seatId: string;
  readonly policyRevision: string;
  readonly revocationHead: string;
  readonly destinationDomainId: string;
  readonly destinationScope: string;
  readonly promotionKind: string;
  readonly sources: ReadonlyArray<NativeAuthorizedContextReadSource>;
}

export interface NativeManifestExpectedVersion {
  readonly sourceDomainId: string;
  readonly contextId: string;
  readonly version: string;
  readonly contentHash: string;
  readonly stateRevision: string;
  readonly accessPolicyRevision: string;
}

export interface NativeContextManifestCommitRequest {
  readonly operationId: string;
  readonly requestDigest: string;
  readonly eventId: string;
  readonly receiptId: string;
  readonly recordedAt: string;
  readonly readRequests: ReadonlyArray<NativeGranteeContextReadRequest>;
  readonly expectedVersions: ReadonlyArray<NativeManifestExpectedVersion>;
  readonly canonicalManifest: string;
}

export interface NativeContextManifestReplayIdentity {
  readonly operationId: string;
  readonly principalId: string;
  readonly seatId: string;
  readonly taskId: string;
  readonly sessionId: string;
  readonly domainId: string;
  readonly bindingId: string;
  readonly bindingGeneration: string;
  readonly sourceEpoch: string;
  readonly runtimeInstanceId: string;
}

export interface NativeContextManifestReceipt {
  readonly disposition: "COMMITTED" | "REPLAYED";
  readonly operationId: string;
  readonly manifestId: string;
  readonly manifestHash: string;
  readonly canonicalManifest: string;
}

export interface NativeExecutionRecipeAppendRequest {
  readonly operationId: string;
  readonly domainId: string;
  readonly expectedPreviousRevision: string | null;
  readonly recipeId: string;
  readonly seatId: string;
  readonly runtimeInstanceId: string;
  readonly modelRef: Readonly<Record<string, unknown>>;
  readonly toolProfile: unknown;
  readonly isolationProfile: unknown;
  readonly contextManifestId: string;
  readonly budgetPolicy: unknown;
  readonly admissionRef: string;
  readonly eventId: string;
  readonly receiptId: string;
  readonly recordedAt: string;
}

export interface NativeExecutionRecipeReceipt {
  readonly disposition: "COMMITTED" | "REPLAYED";
  readonly operationId: string;
  readonly currentnessStatus: string;
  readonly recipe: {
    readonly contentHash: string;
    readonly domainId: string;
    readonly recipeId: string;
    readonly revision: string;
    readonly seatId: string;
    readonly runtimeInstanceId: string;
    readonly contextManifestId: string;
    readonly admissionRef: string;
  };
}

export const encodeExecutionRecipeFrame = (input: NativeExecutionRecipeAppendRequest): string =>
  JSON.stringify({
    admissionRef: input.admissionRef,
    budgetPolicy: JSON.stringify(input.budgetPolicy),
    contextManifestId: input.contextManifestId,
    domainId: input.domainId,
    eventId: input.eventId,
    expectedPreviousRevision: input.expectedPreviousRevision ?? "",
    isolationProfile: JSON.stringify(input.isolationProfile),
    modelRef: JSON.stringify(input.modelRef),
    operation: "AppendExecutionRecipe",
    operationId: input.operationId,
    recordedAt: input.recordedAt,
    receiptId: input.receiptId,
    recipeId: input.recipeId,
    runtimeInstanceId: input.runtimeInstanceId,
    seatId: input.seatId,
    toolProfile: JSON.stringify(input.toolProfile),
  });

export const decodeExecutionRecipeReceipt = (body: string): NativeExecutionRecipeReceipt => {
  const parsed: unknown = parseStrictJsonBytes(new TextEncoder().encode(body));
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) throw new NativeHostClientError("RECIPE_REPLY", "recipe reply must be an object");
  const value = parsed as Record<string, unknown>;
  const fields = ["currentnessStatus", "disposition", "operationId", "recipe"];
  if (Reflect.ownKeys(value).length !== fields.length || fields.some((field) => !Object.hasOwn(value, field))) throw new NativeHostClientError("RECIPE_REPLY", "recipe reply fields are invalid");
  if (!["COMMITTED", "REPLAYED"].includes(String(value.disposition)) || typeof value.operationId !== "string" || typeof value.currentnessStatus !== "string") throw new NativeHostClientError("RECIPE_REPLY", "recipe disposition is invalid");
  if (typeof value.recipe !== "object" || value.recipe === null || Array.isArray(value.recipe)) throw new NativeHostClientError("RECIPE_REPLY", "recipe body is invalid");
  const recipe = value.recipe as Record<string, unknown>;
  const recipeFields = ["admissionRef", "contentHash", "contextManifestId", "domainId", "recipeId", "revision", "runtimeInstanceId", "seatId"];
  if (Reflect.ownKeys(recipe).length !== recipeFields.length || recipeFields.some((field) => typeof recipe[field] !== "string" || (recipe[field] as string).length === 0)) throw new NativeHostClientError("RECIPE_REPLY", "recipe body fields are invalid");
  return Object.freeze({ disposition: value.disposition as "COMMITTED" | "REPLAYED", operationId: value.operationId as string, currentnessStatus: value.currentnessStatus as string, recipe: Object.freeze({ admissionRef: recipe.admissionRef as string, contentHash: recipe.contentHash as string, contextManifestId: recipe.contextManifestId as string, domainId: recipe.domainId as string, recipeId: recipe.recipeId as string, revision: recipe.revision as string, runtimeInstanceId: recipe.runtimeInstanceId as string, seatId: recipe.seatId as string }) });
};

export interface NativeObjectiveOutcomeRequest { readonly domainId:string; readonly outcomeId:string; readonly revision:string; readonly expectedPreviousRevision:string|null; readonly expectedPreviousContentHash:string|null; readonly operationId:string; readonly eventId:string; readonly receiptId:string; readonly recordedAt:string; readonly manifestId:string; readonly manifestVersion:string; readonly manifestHash:string; readonly decisionId:string; readonly decisionVersion:string; readonly decisionHash:string; readonly actionOperationId:string; readonly actionCompletionRef:string; readonly resultRefs:ReadonlyArray<NativeContextRecord>; readonly evidenceRefs:ReadonlyArray<NativeContextRecord>; readonly observationStartsAt:string; readonly observationEndsAt:string; readonly observationStatus:string; }
export interface NativeEvaluationRequest { readonly domainId:string; readonly evaluationId:string; readonly revision:string; readonly expectedPreviousRevision:string|null; readonly expectedPreviousContentHash:string|null; readonly operationId:string; readonly eventId:string; readonly receiptId:string; readonly recordedAt:string; readonly sourceIdentity:string; readonly outcomeRefs:ReadonlyArray<NativeContextRecord>; readonly decisionFamily:string; readonly scorerVersion:string; readonly rubricVersion:string; readonly calibrationKey:string; readonly calibrationVersion:string; readonly datasetNamespace:string; readonly datasetSplit:string; readonly evidenceRefs:ReadonlyArray<NativeContextRecord>; readonly metricsHash:string; readonly safetyStatus:string; readonly privacyStatus:string; }
export interface NativeContextRecord { readonly objectType:string; readonly objectId:string; readonly revision:string; readonly contentHash:string; }
export interface NativeAuthorityRecordReceipt { readonly disposition:"COMMITTED"|"REPLAYED"; readonly operationId:string; readonly objectId:string; readonly revision:string; readonly contentHash:string; readonly receiptId:string; }
export interface NativeAuthorityObjectSnapshot { readonly contentHash:string; readonly domainId:string; readonly objectId:string; readonly revision:string; }
export interface NativeSessionLineageSnapshot { readonly contentHash:string; readonly domainId:string; readonly lifecycle:string; readonly revision:string; readonly sessionId:string; }
export interface NativeExposureReceiptSnapshot { readonly domainId:string; readonly receiptId:string; readonly sessionRevision:string; readonly sessionId:string; }
const encodeAuthorityRecords=(tag:string,records:ReadonlyArray<NativeContextRecord>,width=4):string=>encodeStringRecords(tag,records.map(r=>[r.objectType,r.objectId,r.revision,r.contentHash].slice(0,width)),width,false);
const encodeEvaluationOutcomes=(records:ReadonlyArray<NativeContextRecord>):string=>encodeStringRecords("gogoke.evaluation-outcomes.v1",records.map(r=>[r.objectId,r.revision,r.contentHash]),3,false);
const decodeAuthorityReceipt=(body:string,operation:string):NativeAuthorityRecordReceipt=>{const v=JSON.parse(body) as Record<string,unknown>;const keys=["contentHash","disposition","objectId","operationId","receiptId","revision"];if(Reflect.ownKeys(v).length!==keys.length||keys.some(k=>typeof v[k]!=="string")||v.operation!==undefined||!["COMMITTED","REPLAYED"].includes(String(v.disposition)))throw new NativeHostClientError("AUTHORITY_REPLY",`${operation} reply invalid`);return Object.freeze({contentHash:v.contentHash as string,disposition:v.disposition as "COMMITTED"|"REPLAYED",objectId:v.objectId as string,operationId:v.operationId as string,receiptId:v.receiptId as string,revision:v.revision as string});};
const decodeAuthoritySnapshot=(body:string):NativeAuthorityObjectSnapshot=>{const v=JSON.parse(body) as Record<string,unknown>;const keys=["contentHash","domainId","objectId","revision"];if(Reflect.ownKeys(v).length!==keys.length||keys.some(k=>typeof v[k]!=="string"||(v[k] as string).length===0))throw new NativeHostClientError("AUTHORITY_REPLY","snapshot reply invalid");return Object.freeze({contentHash:v.contentHash as string,domainId:v.domainId as string,objectId:v.objectId as string,revision:v.revision as string});};
const decodeSessionLineageSnapshot=(body:string):NativeSessionLineageSnapshot=>{const v=JSON.parse(body) as Record<string,unknown>;const keys=["contentHash","domainId","lifecycle","revision","sessionId"];if(Reflect.ownKeys(v).length!==keys.length||keys.some(k=>typeof v[k]!=="string"||(v[k] as string).length===0))throw new NativeHostClientError("SESSION_LINEAGE_REPLY","lineage reply invalid");return Object.freeze(v as unknown as NativeSessionLineageSnapshot);};
const decodeExposureReceiptSnapshot=(body:string):NativeExposureReceiptSnapshot=>{const v=JSON.parse(body) as Record<string,unknown>;const keys=["domainId","receiptId","sessionRevision","sessionId"];if(Reflect.ownKeys(v).length!==keys.length||keys.some(k=>typeof v[k]!=="string"||(v[k] as string).length===0))throw new NativeHostClientError("EXPOSURE_REPLY","exposure reply invalid");return Object.freeze(v as unknown as NativeExposureReceiptSnapshot);};
export const encodeObjectiveOutcomeFrame=(i:NativeObjectiveOutcomeRequest):string=>JSON.stringify({actionCompletionRef:i.actionCompletionRef,actionOperationId:i.actionOperationId,decisionHash:i.decisionHash,decisionId:i.decisionId,decisionVersion:i.decisionVersion,domainId:i.domainId,evidenceRefs:encodeAuthorityRecords("gogoke.objective-evidence.v1",i.evidenceRefs),eventId:i.eventId,expectedPreviousContentHash:i.expectedPreviousContentHash??"",expectedPreviousRevision:i.expectedPreviousRevision??"",manifestHash:i.manifestHash,manifestId:i.manifestId,manifestVersion:i.manifestVersion,observationEndsAt:i.observationEndsAt,observationStartsAt:i.observationStartsAt,observationStatus:i.observationStatus,operation:"AppendObjectiveOutcome",operationId:i.operationId,outcomeId:i.outcomeId,receiptId:i.receiptId,recordedAt:i.recordedAt,resultRefs:encodeAuthorityRecords("gogoke.objective-evidence.v1",i.resultRefs),revision:i.revision});
export const encodeEvaluationFrame=(i:NativeEvaluationRequest):string=>JSON.stringify({calibrationKey:i.calibrationKey,calibrationVersion:i.calibrationVersion,datasetNamespace:i.datasetNamespace,datasetSplit:i.datasetSplit,decisionFamily:i.decisionFamily,domainId:i.domainId,evidenceRefs:encodeAuthorityRecords("gogoke.evaluation-evidence.v1",i.evidenceRefs),eventId:i.eventId,expectedPreviousContentHash:i.expectedPreviousContentHash??"",expectedPreviousRevision:i.expectedPreviousRevision??"",metricsHash:i.metricsHash,operation:"AppendEvaluation",operationId:i.operationId,outcomeRefs:encodeEvaluationOutcomes(i.outcomeRefs),privacyStatus:i.privacyStatus,receiptId:i.receiptId,recordedAt:i.recordedAt,revision:i.revision,rubricVersion:i.rubricVersion,safetyStatus:i.safetyStatus,scorerVersion:i.scorerVersion,sourceIdentity:i.sourceIdentity,evaluationId:i.evaluationId});
export interface NativeDreamRunRequest { readonly domainId:string; readonly runId:string; readonly revision:string; readonly expectedPreviousRevision:string|null; readonly expectedPreviousContentHash:string|null; readonly operationId:string; readonly eventId:string; readonly receiptId:string; readonly recordedAt:string; readonly sourceIdentity:string; readonly inputSnapshot:NativeContextRecord; readonly datasetNamespace:string; readonly datasetSplit:string; readonly datasetSplitHash:string; readonly recipeRef:NativeContextRecord; readonly budgetLease:Readonly<{leaseRef:string;operationId:string;resourceRef:string;resourceRevision:string;units:string}>; readonly evaluationRefs:ReadonlyArray<NativeContextRecord>; }
export interface NativeDreamProposalRequest { readonly domainId:string; readonly proposalId:string; readonly revision:string; readonly expectedPreviousRevision:string|null; readonly expectedPreviousContentHash:string|null; readonly operationId:string; readonly eventId:string; readonly receiptId:string; readonly recordedAt:string; readonly sourceIdentity:string; readonly runRef:NativeContextRecord; readonly candidateKind:string; readonly beforeHash:string; readonly afterHash:string; readonly allowedChangeSet:ReadonlyArray<Readonly<{key:string;beforeHash:string;afterHash:string}>>; readonly heldoutReceipt:NativeContextRecord|null; readonly rollbackRef:NativeContextRecord; readonly basePolicyRevision:string; readonly namespace:string; readonly testOnly:boolean; }
const encodeDreamObject=(tag:string,r:NativeContextRecord)=>encodeStringRecords(tag,[[r.objectType,r.objectId,r.revision,r.contentHash]],4,false);
const encodeDreamEvaluations=(records:ReadonlyArray<NativeContextRecord>):string=>encodeStringRecords("gogoke.dream-evaluations.v1",records.map(r=>[r.objectId,r.revision,r.contentHash]),3,true);
export const encodeDreamRunFrame=(i:NativeDreamRunRequest):string=>JSON.stringify({budgetLeaseRef:i.budgetLease.leaseRef,budgetOperationId:i.budgetLease.operationId,budgetResourceRef:i.budgetLease.resourceRef,budgetResourceRevision:i.budgetLease.resourceRevision,budgetUnits:i.budgetLease.units,datasetNamespace:i.datasetNamespace,datasetSplit:i.datasetSplit,datasetSplitHash:i.datasetSplitHash,domainId:i.domainId,evaluationRefs:encodeDreamEvaluations(i.evaluationRefs),eventId:i.eventId,expectedPreviousContentHash:i.expectedPreviousContentHash??"",expectedPreviousRevision:i.expectedPreviousRevision??"",inputSnapshot:encodeDreamObject("gogoke.dream-object.v1",i.inputSnapshot),operation:"AppendDreamRun",operationId:i.operationId,receiptId:i.receiptId,recordedAt:i.recordedAt,recipeRef:encodeDreamObject("gogoke.dream-object.v1",i.recipeRef),revision:i.revision,runId:i.runId,sourceIdentity:i.sourceIdentity});
export const encodeDreamProposalFrame=(i:NativeDreamProposalRequest):string=>JSON.stringify({afterHash:i.afterHash,allowedChangeSet:encodeStringRecords("gogoke.dream-changes.v1",i.allowedChangeSet.map(c=>[c.key,c.beforeHash,c.afterHash]),3,false),basePolicyRevision:i.basePolicyRevision,beforeHash:i.beforeHash,candidateKind:i.candidateKind,domainId:i.domainId,eventId:i.eventId,expectedPreviousContentHash:i.expectedPreviousContentHash??"",expectedPreviousRevision:i.expectedPreviousRevision??"",heldoutEvaluation:i.heldoutReceipt?encodeDreamEvaluations([i.heldoutReceipt]):"",namespace:i.namespace,operation:"AppendDreamProposal",operationId:i.operationId,proposalId:i.proposalId,receiptId:i.receiptId,recordedAt:i.recordedAt,revision:i.revision,rollbackRef:encodeDreamObject("gogoke.dream-object.v1",i.rollbackRef),runRef:encodeDreamObject("gogoke.dream-object.v1",i.runRef),sourceIdentity:i.sourceIdentity,testOnly:i.testOnly?"true":"false"});

const safeInteger = (value: number, path: string): string => {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new NativeHostClientError(
      "DECISION_FRAME",
      `${path} must be a non-negative safe integer`,
    );
  }
  return String(value);
};

export function encodeDecisionSnapshotFrame(input: NativeDecisionAuthoritySnapshot): string {
  return JSON.stringify({
    actionDigest: input.actionDigest,
    actionOperationId: input.actionOperationId,
    authRevision: input.authRevision,
    bindingGeneration: input.bindingGeneration,
    bindingId: input.bindingId,
    candidateHash: input.candidateHash,
    candidateId: input.candidateId,
    capabilityRevision: input.capabilityRevision,
    capacityTotal: safeInteger(input.capacityTotal, "capacityTotal"),
    operation: "PublishDecisionSnapshot",
    operationId: input.operationId,
    policyRevision: input.policyRevision,
    resourceRef: input.resourceRef,
    resourceRevision: input.resourceRevision,
    stateViewHash: input.stateViewHash,
    taskRevision: input.taskRevision,
  });
}

export function encodeDecisionCommitFrame(input: NativeDecisionCommitRequest): string {
  const record = input.record;
  if (
    record.state !== "COMMITTED" ||
    record.reason !== "QUALIFIED_BOUNDED_SELECTION" ||
    record.choice.length === 0
  ) {
    throw new NativeHostClientError(
      "DECISION_FRAME",
      "only an admitted committed Decision can cross the native commit seam",
    );
  }
  return JSON.stringify({
    actionIntentRef: input.actionIntentRef,
    backendKind: record.backendKind,
    bindingGeneration: record.bindingGeneration,
    budgetUnits: safeInteger(record.budgetUnits, "budgetUnits"),
    candidateHash: record.candidateHash,
    capabilityRevision: record.capabilityRevision,
    choice: record.choice,
    deadlineEpochMs: safeInteger(record.deadlineEpochMs, "deadlineEpochMs"),
    decisionId: input.decisionId,
    domainId: input.domainId,
    eventId: input.eventId,
    family: record.family,
    modelRequested: record.modelRequested ?? "",
    modelResolved: record.modelResolved ?? "",
    operation: "CommitDecision",
    operationId: record.operationId,
    policyRevision: record.policyRevision,
    questionVersion: record.questionVersion,
    reason: record.reason,
    receiptId: input.receiptId,
    recordedAt: input.recordedAt,
    requiredCapacityUnits: safeInteger(input.requiredCapacityUnits, "requiredCapacityUnits"),
    resourceReservationRef: input.resourceReservationRef,
    rubricVersion: record.rubricVersion,
    scenarioId: record.scenarioId,
    stateViewHash: record.stateViewHash,
    taskRevision: record.taskRevision,
  });
}

export const encodeDecisionReplayFrame = (domainId: string, operationId: string): string =>
  JSON.stringify({ domainId, operation: "ReadDecisionReplay", operationId });

const decisionReplyRecord = (value: unknown): NativeDecisionRecord => {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("DECISION_REPLY", "durable Decision record must be an object");
  }
  const record = value as Record<string, unknown>;
  const keys = [
    "operationId",
    "scenarioId",
    "family",
    "state",
    "stateViewHash",
    "candidateHash",
    "questionVersion",
    "rubricVersion",
    "modelRequested",
    "modelResolved",
    "taskRevision",
    "policyRevision",
    "capabilityRevision",
    "bindingGeneration",
    "backendKind",
    "choice",
    "reason",
    "budgetUnits",
    "deadlineEpochMs",
  ];
  if (
    Reflect.ownKeys(record).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(record, key))
  ) {
    throw new NativeHostClientError("DECISION_REPLY", "durable Decision record fields are invalid");
  }
  const strings = [
    "operationId",
    "scenarioId",
    "family",
    "stateViewHash",
    "candidateHash",
    "questionVersion",
    "rubricVersion",
    "taskRevision",
    "policyRevision",
    "capabilityRevision",
    "bindingGeneration",
    "choice",
  ] as const;
  if (
    strings.some((key) => typeof record[key] !== "string" || (record[key] as string).length === 0)
  ) {
    throw new NativeHostClientError("DECISION_REPLY", "durable Decision identity is invalid");
  }
  if (
    record.state !== "COMMITTED" ||
    record.reason !== "QUALIFIED_BOUNDED_SELECTION" ||
    !["RULES", "FAKE", "REPLAY"].includes(String(record.backendKind)) ||
    (record.modelRequested !== null && typeof record.modelRequested !== "string") ||
    (record.modelResolved !== null && typeof record.modelResolved !== "string") ||
    typeof record.budgetUnits !== "number" ||
    !Number.isSafeInteger(record.budgetUnits) ||
    record.budgetUnits < 0 ||
    typeof record.deadlineEpochMs !== "number" ||
    !Number.isSafeInteger(record.deadlineEpochMs) ||
    record.deadlineEpochMs < 0
  ) {
    throw new NativeHostClientError("DECISION_REPLY", "durable Decision semantics are invalid");
  }
  return Object.freeze({
    operationId: record.operationId as string,
    scenarioId: record.scenarioId as string,
    family: record.family as string,
    state: "COMMITTED",
    stateViewHash: record.stateViewHash as string,
    candidateHash: record.candidateHash as string,
    questionVersion: record.questionVersion as string,
    rubricVersion: record.rubricVersion as string,
    modelRequested: record.modelRequested as string | null,
    modelResolved: record.modelResolved as string | null,
    taskRevision: record.taskRevision as string,
    policyRevision: record.policyRevision as string,
    capabilityRevision: record.capabilityRevision as string,
    bindingGeneration: record.bindingGeneration as string,
    backendKind: record.backendKind as NativeDecisionRecord["backendKind"],
    choice: record.choice as string,
    reason: "QUALIFIED_BOUNDED_SELECTION",
    budgetUnits: record.budgetUnits,
    deadlineEpochMs: record.deadlineEpochMs,
  });
};

export function decodeDecisionCommitReply(body: string): NativeDecisionCommitReply {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("DECISION_REPLY", "Decision reply must be an object");
  }
  const record = value as Record<string, unknown>;
  if (record.kind === "committed") {
    const keys = ["kind", "operationId", "decisionReceiptId"];
    if (
      Reflect.ownKeys(record).length !== keys.length ||
      keys.some((key) => !Object.hasOwn(record, key)) ||
      typeof record.operationId !== "string" ||
      typeof record.decisionReceiptId !== "string"
    ) {
      throw new NativeHostClientError("DECISION_REPLY", "committed Decision reply is invalid");
    }
    return Object.freeze({
      kind: "committed",
      operationId: record.operationId,
      decisionReceiptId: record.decisionReceiptId,
    });
  }
  if (record.kind === "replayed") {
    const keys = ["kind", "operationId", "decisionReceiptId", "record"];
    if (
      Reflect.ownKeys(record).length !== keys.length ||
      keys.some((key) => !Object.hasOwn(record, key)) ||
      typeof record.operationId !== "string" ||
      typeof record.decisionReceiptId !== "string"
    ) {
      throw new NativeHostClientError("DECISION_REPLY", "replayed Decision reply is invalid");
    }
    const durable = decisionReplyRecord(record.record);
    if (durable.operationId !== record.operationId) {
      throw new NativeHostClientError("DECISION_REPLY", "replay operation identity mismatch");
    }
    return Object.freeze({
      kind: "replayed",
      operationId: record.operationId,
      decisionReceiptId: record.decisionReceiptId,
      record: durable,
    });
  }
  throw new NativeHostClientError("DECISION_REPLY", "Decision reply kind is invalid");
}

const contextCount = (value: number, path: string): string => {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new NativeHostClientError("CONTEXT_FRAME", `${path} must be a non-negative safe integer`);
  }
  return String(value);
};

const encodeStringRecords = (
  tag: string,
  records: ReadonlyArray<ReadonlyArray<string>>,
  width: number,
  allowEmpty: boolean,
): string => {
  if (records.length > 64 || (!allowEmpty && records.length === 0)) {
    throw new NativeHostClientError("CONTEXT_FRAME", "record count is outside the native bound");
  }
  let encoded = `${tag}|${records.length}|`;
  for (const record of records) {
    if (record.length !== width || record.some((field) => typeof field !== "string")) {
      throw new NativeHostClientError("CONTEXT_FRAME", "record width is invalid");
    }
    for (const field of record) encoded += `${Buffer.byteLength(field, "utf8")}:${field}`;
  }
  return encoded;
};

const decodeStringRecords = (
  value: string,
  tag: string,
  width: number,
  allowEmpty: boolean,
): ReadonlyArray<ReadonlyArray<string>> => {
  const bytes = Buffer.from(value, "utf8");
  const prefix = Buffer.from(`${tag}|`, "ascii");
  if (bytes.subarray(0, prefix.length).compare(prefix) !== 0) {
    throw new NativeHostClientError("CONTEXT_REPLY", "record schema is invalid");
  }
  let offset = prefix.length;
  const delimiter = bytes.indexOf(0x7c, offset);
  if (delimiter < 0) throw new NativeHostClientError("CONTEXT_REPLY", "record count is missing");
  const countText = bytes.subarray(offset, delimiter).toString("ascii");
  if (!/^(?:0|[1-9][0-9]*)$/.test(countText)) {
    throw new NativeHostClientError("CONTEXT_REPLY", "record count is not canonical");
  }
  const count = Number(countText);
  if (!Number.isSafeInteger(count) || count > 64 || (!allowEmpty && count === 0)) {
    throw new NativeHostClientError("CONTEXT_REPLY", "record count is outside the native bound");
  }
  offset = delimiter + 1;
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const records: string[][] = [];
  for (let recordIndex = 0; recordIndex < count; recordIndex += 1) {
    const record: string[] = [];
    for (let fieldIndex = 0; fieldIndex < width; fieldIndex += 1) {
      const colon = bytes.indexOf(0x3a, offset);
      if (colon < 0) throw new NativeHostClientError("CONTEXT_REPLY", "record length is missing");
      const lengthText = bytes.subarray(offset, colon).toString("ascii");
      if (!/^(?:0|[1-9][0-9]*)$/.test(lengthText)) {
        throw new NativeHostClientError("CONTEXT_REPLY", "record length is not canonical");
      }
      const length = Number(lengthText);
      const start = colon + 1;
      const end = start + length;
      if (!Number.isSafeInteger(length) || end > bytes.length) {
        throw new NativeHostClientError("CONTEXT_REPLY", "record length exceeds the payload");
      }
      try {
        record.push(decoder.decode(bytes.subarray(start, end)));
      } catch {
        throw new NativeHostClientError("CONTEXT_REPLY", "record field is not UTF-8");
      }
      offset = end;
    }
    records.push(record);
  }
  if (offset !== bytes.length) {
    throw new NativeHostClientError("CONTEXT_REPLY", "record payload has trailing bytes");
  }
  return Object.freeze(records.map((record) => Object.freeze(record)));
};

const granteeReadRecords = (
  requests: ReadonlyArray<NativeGranteeContextReadRequest>,
): ReadonlyArray<ReadonlyArray<string>> =>
  requests.map((request) => [
    request.principalId,
    request.seatId,
    request.source.sourceDomainId,
    request.source.contextId,
    request.source.version,
    request.source.expectedScope,
    request.source.expectedContentHash,
    request.source.expectedAccessPolicyRevision,
    request.source.destinationDomainId,
    request.source.destinationScope,
    request.source.promotionKind,
    request.source.policyRevision,
    request.source.grant.grantId,
    request.source.grant.revision,
    request.source.grant.revocationHead,
  ]);

export function encodeContextAssemblySnapshotFrame(input: NativeContextAssemblySnapshot): string {
  const partitions = input.partitionBindings.map((binding) => [
    binding.sourceDomainId,
    binding.destinationScope,
    binding.promotionKind,
    binding.grant.grantId,
    binding.grant.revision,
    binding.grant.revocationHead,
  ]);
  return JSON.stringify({
    admissionActionOperationId: input.admissionActionOperationId,
    admissionDigest: input.admissionDigest,
    authRevision: input.authRevision,
    bindingGeneration: input.bindingGeneration,
    bindingId: input.bindingId,
    domainId: input.domainId,
    manifestId: input.manifestId,
    maxCandidates: contextCount(input.maxCandidates, "maxCandidates"),
    maxContentBytes: contextCount(input.maxContentBytes, "maxContentBytes"),
    operation: "PublishContextAssemblySnapshot",
    operationId: input.operationId,
    partitionBindings: encodeStringRecords(
      "gogoke.context-assembly-partition-bindings.v1",
      partitions,
      6,
      false,
    ),
    policyRevision: input.policyRevision,
    principalId: input.principalId,
    revocationHead: input.revocationHead,
    runtimeInstanceId: input.runtimeInstanceId,
    seatId: input.seatId,
    selectionDecisionId: input.selectionDecisionId,
    sessionId: input.sessionId,
    sourceEpoch: input.sourceEpoch,
    taskId: input.taskId,
    taskRevision: input.taskRevision,
  });
}

export function encodeGranteeContextSetFrame(
  requests: ReadonlyArray<NativeGranteeContextReadRequest>,
): string {
  return JSON.stringify({
    operation: "ReadGranteeContextSet",
    readRequests: encodeStringRecords(
      "gogoke.grantee-context-read-requests.v1",
      granteeReadRecords(requests),
      15,
      false,
    ),
  });
}

export function decodeAuthorizedContextReadSet(body: string): NativeAuthorizedContextReadSet {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("CONTEXT_REPLY", "authorized read set must be an object");
  }
  const record = value as Record<string, unknown>;
  const keys = [
    "destinationDomainId",
    "destinationScope",
    "policyRevision",
    "principalId",
    "promotionKind",
    "revocationHead",
    "seatId",
    "sources",
  ];
  if (
    Reflect.ownKeys(record).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(record, key)) ||
    keys.some((key) => typeof record[key] !== "string")
  ) {
    throw new NativeHostClientError("CONTEXT_REPLY", "authorized read set fields are invalid");
  }
  const rows = decodeStringRecords(
    record.sources as string,
    "gogoke.authorized-context-read-sources.v1",
    15,
    false,
  );
  const sources = rows.map((row) =>
    Object.freeze({
      grantRevision: row[0]!,
      revocationHead: row[1]!,
      state: row[2]!,
      stateRevision: row[3]!,
      sourceDomainId: row[4]!,
      contextId: row[5]!,
      version: row[6]!,
      scope: row[7]!,
      kind: row[8]!,
      contentHash: row[9]!,
      sourceRef: row[10]!,
      sourceHash: row[11]!,
      sourceAuthorityKind: row[12]!,
      sourceAuthorityRef: row[13]!,
      accessPolicyRevision: row[14]!,
    }),
  );
  return Object.freeze({
    destinationDomainId: record.destinationDomainId as string,
    destinationScope: record.destinationScope as string,
    policyRevision: record.policyRevision as string,
    principalId: record.principalId as string,
    promotionKind: record.promotionKind as string,
    revocationHead: record.revocationHead as string,
    seatId: record.seatId as string,
    sources: Object.freeze(sources),
  });
}

export function encodeContextManifestCommitFrame(
  input: NativeContextManifestCommitRequest,
): string {
  const expected = input.expectedVersions.map((version) => [
    version.sourceDomainId,
    version.contextId,
    version.version,
    version.contentHash,
    version.stateRevision,
    version.accessPolicyRevision,
  ]);
  return JSON.stringify({
    canonicalManifest: input.canonicalManifest,
    eventId: input.eventId,
    expectedVersions: encodeStringRecords(
      "gogoke.manifest-expected-versions.v1",
      expected,
      6,
      true,
    ),
    operation: "CommitContextManifest",
    operationId: input.operationId,
    readRequests: encodeStringRecords(
      "gogoke.grantee-context-read-requests.v1",
      granteeReadRecords(input.readRequests),
      15,
      false,
    ),
    receiptId: input.receiptId,
    recordedAt: input.recordedAt,
    requestDigest: input.requestDigest,
  });
}

export function encodeContextManifestReplayFrame(
  input: NativeContextManifestReplayIdentity,
): string {
  return JSON.stringify({ ...input, operation: "ReadContextManifest" });
}

export function encodeContextAssemblyReadFrame(
  operation: "ReadContextAssemblyBasis" | "ListContextAssemblySources",
  input: NativeContextManifestReplayIdentity,
): string {
  return JSON.stringify({ ...input, operation });
}

const decodeContextCount = (value: string, path: string): number => {
  if (!/^(?:0|[1-9][0-9]*)$/.test(value)) {
    throw new NativeHostClientError("CONTEXT_REPLY", `${path} is not canonical`);
  }
  const count = Number(value);
  if (!Number.isSafeInteger(count)) {
    throw new NativeHostClientError("CONTEXT_REPLY", `${path} is outside the safe integer range`);
  }
  return count;
};

const decodeContextPartitionBindings = (
  value: string,
): ReadonlyArray<NativeContextPartitionBinding> => {
  const rows = decodeStringRecords(
    value,
    "gogoke.context-assembly-partition-bindings.v1",
    6,
    false,
  );
  return Object.freeze(
    rows.map((row) => {
      const destinationScope = row[1];
      if (
        destinationScope !== "GLOBAL" &&
        destinationScope !== "PROJECT" &&
        destinationScope !== "SESSION"
      ) {
        throw new NativeHostClientError("CONTEXT_REPLY", "partition scope is invalid");
      }
      return Object.freeze({
        sourceDomainId: row[0]!,
        destinationScope,
        promotionKind: row[2]!,
        grant: Object.freeze({
          grantId: row[3]!,
          revision: row[4]!,
          revocationHead: row[5]!,
        }),
      });
    }),
  );
};

const taskMandatoryRefRecords = (
  values: ReadonlyArray<TaskMandatoryContextRef>,
): ReadonlyArray<ReadonlyArray<string>> =>
  values.map((value) => [value.sourceDomainId, value.contextId, value.version]);

const decodeTaskMandatoryRefs = (value: string): ReadonlyArray<TaskMandatoryContextRef> => {
  const rows = decodeStringRecords(
    value,
    "gogoke.task-mandatory-context-refs.v1",
    3,
    true,
  );
  const refs = rows.map((row) =>
    Object.freeze({ sourceDomainId: row[0]!, contextId: row[1]!, version: row[2]! }),
  );
  if (
    refs.some(
      (ref) =>
        ref.sourceDomainId.length === 0 ||
        ref.contextId.length === 0 ||
        !/^(?:0|[1-9][0-9]*)$/u.test(ref.version) ||
        ref.version.length > 20 ||
        BigInt(ref.version) > 18_446_744_073_709_551_615n,
    ) ||
    new Set(refs.map((ref) => `${ref.sourceDomainId}\u0000${ref.contextId}\u0000${ref.version}`))
      .size !== refs.length
  ) {
    throw new NativeHostClientError("CONTEXT_REPLY", "mandatory Context refs are invalid");
  }
  return Object.freeze(refs);
};

export function encodeTaskContextCommitFrame(input: CommitTaskContextRequirements): string {
  return JSON.stringify({
    domainId: input.domainId,
    eventId: input.eventId,
    expectedPreviousTaskRevision: input.expectedPreviousTaskRevision ?? "",
    mandatoryRefs: encodeStringRecords(
      "gogoke.task-mandatory-context-refs.v1",
      taskMandatoryRefRecords(input.mandatoryRefs),
      3,
      true,
    ),
    operation: "CommitTaskContextRequirements",
    operationId: input.operationId,
    receiptId: input.receiptId,
    recordedAt: input.recordedAt,
    taskId: input.taskId,
  });
}

export function encodeTaskContextReadFrame(input: ReadTaskContextRequirements): string {
  return JSON.stringify({
    domainId: input.domainId,
    operation: "ReadTaskContextRequirements",
    taskId: input.taskId,
  });
}

function decodeTaskContextCurrent(
  body: string,
  expected: ReadTaskContextRequirements,
  expectedOperationId?: string,
): TaskContextRequirements | TaskContextRequirementsReceipt {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("CONTEXT_REPLY", "Task requirements must be an object");
  }
  const record = value as Record<string, unknown>;
  const receipt = expectedOperationId !== undefined;
  const keys = receipt
    ? [
        "contentHash",
        "disposition",
        "domainId",
        "mandatoryRefs",
        "operationId",
        "taskId",
        "taskRevision",
      ]
    : ["contentHash", "domainId", "mandatoryRefs", "taskId", "taskRevision"];
  if (
    Reflect.ownKeys(record).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(record, key)) ||
    keys.some((key) => typeof record[key] !== "string") ||
    record.domainId !== expected.domainId ||
    record.taskId !== expected.taskId ||
    !/^[1-9][0-9]*$/u.test(record.taskRevision as string) ||
    !/^sha256:[0-9a-f]{64}$/u.test(record.contentHash as string) ||
    (receipt &&
      (record.operationId !== expectedOperationId ||
        (record.disposition !== "COMMITTED" && record.disposition !== "RECONCILED")))
  ) {
    throw new NativeHostClientError("CONTEXT_REPLY", "Task requirements identity is invalid");
  }
  const current = {
    contentHash: record.contentHash as string,
    domainId: record.domainId as string,
    mandatoryRefs: decodeTaskMandatoryRefs(record.mandatoryRefs as string),
    taskId: record.taskId as string,
    taskRevision: record.taskRevision as string,
  };
  return receipt
    ? Object.freeze({
        ...current,
        disposition: record.disposition as TaskContextRequirementsReceipt["disposition"],
        operationId: record.operationId as string,
      })
    : Object.freeze(current);
}

export function decodeTaskContextRequirements(
  body: string,
  expected: ReadTaskContextRequirements,
): TaskContextRequirements {
  return decodeTaskContextCurrent(body, expected) as TaskContextRequirements;
}

export function decodeTaskContextRequirementsReceipt(
  body: string,
  expected: CommitTaskContextRequirements,
): TaskContextRequirementsReceipt {
  return decodeTaskContextCurrent(body, expected, expected.operationId) as TaskContextRequirementsReceipt;
}

export function decodeContextAssemblyBasis(
  body: string,
  expectedOperationId: string,
): NativeContextAssemblyBasis {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("CONTEXT_REPLY", "assembly basis must be an object");
  }
  const record = value as Record<string, unknown>;
  const keys = [
    "authRevision",
    "bindingGeneration",
    "mandatoryRefs",
    "maxCandidates",
    "maxContentBytes",
    "operationId",
    "partitionBindings",
    "policyRevision",
    "revocationHead",
    "sourceEpoch",
    "taskRevision",
  ];
  if (
    Reflect.ownKeys(record).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(record, key)) ||
    keys.some((key) => typeof record[key] !== "string") ||
    record.operationId !== expectedOperationId
  ) {
    throw new NativeHostClientError("CONTEXT_REPLY", "assembly basis fields are invalid");
  }
  const maxCandidates = decodeContextCount(record.maxCandidates as string, "maxCandidates");
  if (maxCandidates === 0 || maxCandidates > 64) {
    throw new NativeHostClientError("CONTEXT_REPLY", "maxCandidates is outside the native bound");
  }
  return Object.freeze({
    operationId: record.operationId as string,
    bindingGeneration: record.bindingGeneration as string,
    sourceEpoch: record.sourceEpoch as string,
    taskRevision: record.taskRevision as string,
    policyRevision: record.policyRevision as string,
    authRevision: record.authRevision as string,
    revocationHead: record.revocationHead as string,
    maxContentBytes: decodeContextCount(record.maxContentBytes as string, "maxContentBytes"),
    maxCandidates,
    partitionBindings: decodeContextPartitionBindings(record.partitionBindings as string),
    mandatoryRefs: decodeTaskMandatoryRefs(record.mandatoryRefs as string),
  });
}

export function decodeContextAssemblySources(
  body: string,
  expectedOperationId: string,
): ReadonlyArray<NativeContextAssemblySource> {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("CONTEXT_REPLY", "assembly sources must be an object");
  }
  const record = value as Record<string, unknown>;
  const keys = ["operationId", "sources"];
  if (
    Reflect.ownKeys(record).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(record, key)) ||
    keys.some((key) => typeof record[key] !== "string") ||
    record.operationId !== expectedOperationId
  ) {
    throw new NativeHostClientError("CONTEXT_REPLY", "assembly source identity is invalid");
  }
  const rows = decodeStringRecords(
    record.sources as string,
    "gogoke.context-assembly-sources.v1",
    15,
    true,
  );
  return Object.freeze(
    rows.map((row) =>
      Object.freeze({
        sourceDomainId: row[0]!,
        contextId: row[1]!,
        version: row[2]!,
        scope: row[3]!,
        kind: row[4]!,
        contentHash: row[5]!,
        sourceRef: row[6]!,
        sourceHash: row[7]!,
        sourceAuthorityKind: row[8]!,
        sourceAuthorityRef: row[9]!,
        accessPolicyRevision: row[10]!,
        stateRevision: row[11]!,
        grant: Object.freeze({
          grantId: row[12]!,
          revision: row[13]!,
          revocationHead: row[14]!,
        }),
      }),
    ),
  );
}

export function decodeContextManifestReceipt(body: string): NativeContextManifestReceipt {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("CONTEXT_REPLY", "Manifest receipt must be an object");
  }
  const record = value as Record<string, unknown>;
  const keys = ["canonicalManifest", "disposition", "manifestHash", "manifestId", "operationId"];
  if (
    Reflect.ownKeys(record).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(record, key)) ||
    keys.some((key) => typeof record[key] !== "string") ||
    (record.disposition !== "COMMITTED" && record.disposition !== "REPLAYED")
  ) {
    throw new NativeHostClientError("CONTEXT_REPLY", "Manifest receipt fields are invalid");
  }
  return Object.freeze({
    canonicalManifest: record.canonicalManifest as string,
    disposition: record.disposition,
    manifestHash: record.manifestHash as string,
    manifestId: record.manifestId as string,
    operationId: record.operationId as string,
  });
}

const contextSource = (
  value: CommitContextVersionRequest["object"]["sourceRef"],
): { readonly ref: string; readonly hash: string } => {
  const record = value as Record<string, unknown>;
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    typeof record.ref !== "string" ||
    typeof record.hash !== "string"
  ) {
    throw new NativeHostClientError("CONTEXT_FRAME", "sourceRef must contain ref and hash");
  }
  return { ref: record.ref, hash: record.hash };
};

const contextAuthority = (
  value: CommitContextVersionRequest["object"]["sourceAuthority"],
): { readonly kind: string; readonly ref: string } => {
  const record = value as Record<string, unknown>;
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    typeof record.kind !== "string" ||
    typeof record.ref !== "string"
  ) {
    throw new NativeHostClientError("CONTEXT_FRAME", "sourceAuthority must contain kind and ref");
  }
  return { kind: record.kind, ref: record.ref };
};

export function encodeContextVersionFrame(input: CommitContextVersionRequest): string {
  const source = contextSource(input.object.sourceRef);
  const authority = contextAuthority(input.object.sourceAuthority);
  return JSON.stringify({
    accessPolicyRevision: input.object.accessPolicyRevision,
    contentHash: input.object.contentHash,
    contextId: input.object.contextId,
    derivedFrom: input.object.derivedFrom.join(","),
    domainId: input.object.domainId,
    kind: input.object.kind,
    operation: "CommitContextVersion",
    operationId: input.operationId,
    readGrantRefs: input.access.readGrantRefs.join(","),
    scope: input.object.scope,
    sourceAuthorityKind: authority.kind,
    sourceAuthorityRef: authority.ref,
    sourceHash: source.hash,
    sourceRef: source.ref,
    supersedes: input.object.supersedes.join(","),
    version: input.object.version,
    visibility: input.access.visibility,
    ...(input.promotion === undefined
      ? {}
      : {
          provenanceRefs: input.promotion.provenanceRefs.join(","),
          sourceGrantRef: input.promotion.sourceGrantRef,
          sourceVersionRef: input.promotion.sourceVersionRef,
          targetGrantRef: input.promotion.targetGrantRef,
        }),
  });
}

export function decodeContextCommitReply(body: string): ContextCommitReceipt {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("CONTEXT_REPLY", "reply must be an object");
  }
  const record = value as Record<string, unknown>;
  if (
    (record.disposition !== "COMMITTED" && record.disposition !== "RECONCILED") ||
    typeof record.operationId !== "string" ||
    typeof record.contextId !== "string" ||
    typeof record.version !== "string" ||
    !Array.isArray(record.invalidatedVersionRefs) ||
    record.invalidatedVersionRefs.some((entry) => typeof entry !== "string") ||
    Reflect.ownKeys(record).some(
      (key) =>
        typeof key !== "string" ||
        !["disposition", "operationId", "contextId", "version", "invalidatedVersionRefs"].includes(
          key,
        ),
    )
  ) {
    throw new NativeHostClientError("CONTEXT_REPLY", "reply fields are invalid");
  }
  return Object.freeze({
    disposition: record.disposition,
    operationId: record.operationId,
    contextId: record.contextId,
    version: record.version,
    invalidatedVersionRefs: Object.freeze([...record.invalidatedVersionRefs]),
  });
}

const actionPayloadHex = (value: unknown): string =>
  Buffer.from(JSON.stringify(value), "utf8").toString("hex");

export function encodeActionReservationFrame(input: DurableActionReservation): string {
  const commitment = input.commitment;
  return JSON.stringify({
    actionKind: input.action.kind,
    authRevision: input.binding.authRevision,
    bindingId: input.binding.bindingId,
    childCeilingDigest: commitment.childCeilingDigest,
    childExecutionId: commitment.childExecutionId,
    childGeneration: commitment.childGeneration,
    childSessionId: commitment.childSessionId,
    executionId: input.binding.executionId,
    generation: input.binding.generation,
    lane: input.lane,
    instructionDigest: commitment.instructionDigest,
    materialSetDigest: commitment.materialSetDigest,
    operation: "ReserveAction",
    operationId: input.operationId,
    packageDigest: commitment.packageDigest,
    parentCeilingDigest: commitment.parentCeilingDigest,
    parentGrantRef: commitment.parentGrantRef,
    parentGrantRevision: commitment.parentGrantRevision,
    payloadHex: actionPayloadHex(input.action),
    profileId: input.binding.profileId,
    policyAction: commitment.policyAction,
    reservationId: `reservation-${input.operationId}`,
    runtimeInstanceId: input.binding.runtimeInstanceId,
    route: commitment.route,
    semanticDigest: input.semanticDigest,
    sessionId: input.binding.sessionId,
    sink: commitment.sink,
    sourceDomainId: commitment.sourceDomainId,
    sourceExecutionId: commitment.sourceExecutionId,
    sourceGeneration: commitment.sourceGeneration,
    sourcePrincipalId: commitment.sourcePrincipalId,
    sourceProjectId: commitment.sourceProjectId,
    sourceRole: commitment.sourceRole,
    sourceSessionId: commitment.sourceSessionId,
    targetDomainId: commitment.targetDomainId,
    targetPrincipalId: commitment.targetPrincipalId,
    targetProjectId: commitment.targetProjectId,
    targetRole: commitment.targetRole,
  });
}

export function encodeActionBeginFrame(input: DurableActionReservation): string {
  const frame = JSON.parse(encodeActionReservationFrame(input)) as Record<string, unknown>;
  frame.operation = "BeginActionCommitment";
  return JSON.stringify(frame);
}

const exactActionReplyKeys=(record:Record<string,unknown>,expected:ReadonlyArray<string>):boolean=>{
  const keys=Reflect.ownKeys(record);
  return keys.length===expected.length&&keys.every((key)=>typeof key==="string"&&expected.includes(key));
};

export function decodeActionReservationReply(body: string): ReserveActionResult {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("ACTION_REPLY", "reservation reply must be an object");
  }
  const record = value as Record<string, unknown>;
  if (typeof record.kind !== "string" || typeof record.operationId !== "string") {
    throw new NativeHostClientError("ACTION_REPLY", "reservation reply identity is invalid");
  }
  if (
    record.kind === "reserved" &&
    exactActionReplyKeys(record,["kind","operationId","semanticDigest","reservationId"]) &&
    typeof record.reservationId === "string" &&
    typeof record.semanticDigest === "string"
  ) {
    return Object.freeze({
      kind: "reserved",
      reservationId: record.reservationId,
      operationId: record.operationId,
      semanticDigest: record.semanticDigest,
    });
  }
  if (
    record.kind === "replay" &&
    exactActionReplyKeys(record,["kind","operationId","reservationId","semanticDigest","state"]) &&
    typeof record.reservationId === "string" &&
    typeof record.semanticDigest === "string" &&
    ["reserved", "dispatching", "legacy-unknown", "not-sent", "dispatched", "rejected", "outcome-unknown", "completed"].includes(
      String(record.state),
    )
  ) {
    return Object.freeze({
      kind: "replay",
      reservationId: record.reservationId,
      operationId: record.operationId,
      semanticDigest: record.semanticDigest,
      state: record.state as Extract<ReserveActionResult, { kind: "replay" }>["state"],
    });
  }
  if (record.kind === "conflict" && exactActionReplyKeys(record,["kind","operationId","existingSemanticDigest"]) && typeof record.existingSemanticDigest === "string") {
    return Object.freeze({
      kind: "conflict",
      operationId: record.operationId,
      existingSemanticDigest: record.existingSemanticDigest,
    });
  }
  throw new NativeHostClientError("ACTION_REPLY", "reservation reply fields are invalid");
}

export function decodeActionBeginReply(body: string): BeginActionResult {
  const value: unknown = JSON.parse(body);
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("ACTION_REPLY", "begin reply must be an object");
  }
  const record = value as Record<string, unknown>;
  if (typeof record.kind !== "string" || typeof record.operationId !== "string" || typeof record.reservationId !== "string") {
    throw new NativeHostClientError("ACTION_REPLY", "begin reply identity is invalid");
  }
  if (record.kind === "granted" && exactActionReplyKeys(record,["kind","operationId","reservationId","sendAuthority"]) && typeof record.sendAuthority === "string") {
    return Object.freeze({ kind: "granted", operationId: record.operationId, reservationId: record.reservationId, sendAuthority: record.sendAuthority });
  }
  if (record.kind === "replay" && exactActionReplyKeys(record,["kind","operationId","reservationId","state"]) && ["reserved", "dispatching", "legacy-unknown", "not-sent", "dispatched", "rejected", "outcome-unknown", "completed"].includes(String(record.state))) {
    return Object.freeze({ kind: "replay", operationId: record.operationId, reservationId: record.reservationId, state: record.state as Extract<BeginActionResult,{kind:"replay"}>["state"] });
  }
  if (record.kind === "conflict" && exactActionReplyKeys(record,["kind","operationId","reservationId"])) return Object.freeze({ kind: "conflict", operationId: record.operationId, reservationId: record.reservationId });
  if (record.kind === "unknown" && exactActionReplyKeys(record,["kind","operationId","reservationId"])) return Object.freeze({ kind: "unknown", operationId: record.operationId, reservationId: record.reservationId });
  throw new NativeHostClientError("ACTION_REPLY", "begin reply fields are invalid");
}

const exactDelegationRecord = (
  value: unknown,
  path: string,
  keys: ReadonlyArray<string>,
): Record<string, unknown> => {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new NativeHostClientError("DELEGATION_REPLY", `${path} must be an object`);
  }
  const record = value as Record<string, unknown>;
  const ownKeys = Reflect.ownKeys(record);
  if (
    ownKeys.length !== keys.length ||
    ownKeys.some((key) => typeof key !== "string" || !keys.includes(key)) ||
    keys.some((key) => !Object.hasOwn(record, key))
  ) {
    throw new NativeHostClientError("DELEGATION_REPLY", `${path} fields are invalid`);
  }
  return record;
};

const delegationWireString = (value: unknown, path: string): string => {
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    throw new NativeHostClientError("DELEGATION_REPLY", `${path} must be canonical text`);
  }
  return value;
};

const delegationWireDecimal = (value: unknown, path: string): string => {
  const text = delegationWireString(value, path);
  if (
    text.length > 20 ||
    !/^(?:0|[1-9][0-9]*)$/.test(text) ||
    BigInt(text) > 18_446_744_073_709_551_615n
  ) {
    throw new NativeHostClientError("DELEGATION_REPLY", `${path} must be a canonical u64 string`);
  }
  return text;
};

const delegationWireSafeDecimal = (value: unknown, path: string): string => {
  const text = delegationWireString(value, path);
  if (text.length > 16 || !/^(?:0|[1-9][0-9]*)$/.test(text) || BigInt(text) > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new NativeHostClientError("DELEGATION_REPLY", `${path} must be a safe decimal string`);
  }
  return text;
};

const delegationWireArray = (value: unknown, path: string): ReadonlyArray<string> => {
  if (!Array.isArray(value)) {
    throw new NativeHostClientError("DELEGATION_REPLY", `${path} must be an array`);
  }
  const entries = value.map((item, index) => delegationWireString(item, `${path}[${index}]`));
  if (new Set(entries).size !== entries.length) {
    throw new NativeHostClientError("DELEGATION_REPLY", `${path} contains duplicates`);
  }
  return Object.freeze(entries);
};

export function decodeCurrentDelegationGrantReply(
  body: string,
  expectedGrantRef: string,
): NativeDelegationGrantSnapshot {
  let parsed: unknown;
  try {
    parsed = parseStrictJsonBytes(new TextEncoder().encode(body));
  } catch {
    throw new NativeHostClientError("DELEGATION_REPLY", "reply must be strict JSON without duplicate members");
  }
  const raw = exactDelegationRecord(parsed, "grant", [
    "binding", "ceiling", "expiresAtEpochMs", "grantRef", "issuerId", "parentGrant",
    "policyRevision", "principal", "revision", "revocationHead", "seatId",
  ]);
  const grantRef = delegationWireString(raw.grantRef, "grant.grantRef");
  if (grantRef !== expectedGrantRef) {
    throw new NativeHostClientError("DELEGATION_REPLY", "grant identity mismatch");
  }
  const parent = raw.parentGrant === null
    ? null
    : (() => {
        const value = exactDelegationRecord(raw.parentGrant, "grant.parentGrant", ["grantRef", "revision"]);
        return Object.freeze({
          grantRef: delegationWireString(value.grantRef, "grant.parentGrant.grantRef"),
          revision: delegationWireDecimal(value.revision, "grant.parentGrant.revision"),
        });
      })();
  const principalRaw = exactDelegationRecord(raw.principal, "grant.principal", [
    "domainId", "principalId", "projectId", "role", "seatId",
  ]);
  const principal = Object.freeze({
    domainId: delegationWireString(principalRaw.domainId, "grant.principal.domainId"),
    principalId: delegationWireString(principalRaw.principalId, "grant.principal.principalId"),
    projectId: delegationWireString(principalRaw.projectId, "grant.principal.projectId"),
    role: delegationWireString(principalRaw.role, "grant.principal.role"),
    seatId: delegationWireString(principalRaw.seatId, "grant.principal.seatId"),
  });
  const bindingRaw = exactDelegationRecord(raw.binding, "grant.binding", [
    "executionId", "generation", "sessionId",
  ]);
  const binding = Object.freeze({
    executionId: delegationWireString(bindingRaw.executionId, "grant.binding.executionId"),
    generation: delegationWireDecimal(bindingRaw.generation, "grant.binding.generation"),
    sessionId: delegationWireString(bindingRaw.sessionId, "grant.binding.sessionId"),
  });
  const ceilingRaw = exactDelegationRecord(raw.ceiling, "grant.ceiling", [
    "allowedActions", "allowedContinuationResponses", "allowedMaterialClasses", "allowedSinks",
    "allowedTargetDomainIds", "allowedTargetPrincipalIds", "explicitPrivateMaterialIds",
    "maxMaterialBytes", "maxMaterialItems", "maxResponseBytes",
  ]);
  const ceiling = Object.freeze({
    allowedActions: delegationWireArray(ceilingRaw.allowedActions, "grant.ceiling.allowedActions"),
    allowedContinuationResponses: delegationWireArray(
      ceilingRaw.allowedContinuationResponses, "grant.ceiling.allowedContinuationResponses",
    ),
    allowedMaterialClasses: delegationWireArray(
      ceilingRaw.allowedMaterialClasses, "grant.ceiling.allowedMaterialClasses",
    ),
    allowedSinks: delegationWireArray(ceilingRaw.allowedSinks, "grant.ceiling.allowedSinks"),
    allowedTargetDomainIds: delegationWireArray(
      ceilingRaw.allowedTargetDomainIds, "grant.ceiling.allowedTargetDomainIds",
    ),
    allowedTargetPrincipalIds: delegationWireArray(
      ceilingRaw.allowedTargetPrincipalIds, "grant.ceiling.allowedTargetPrincipalIds",
    ),
    explicitPrivateMaterialIds: delegationWireArray(
      ceilingRaw.explicitPrivateMaterialIds, "grant.ceiling.explicitPrivateMaterialIds",
    ),
    maxMaterialBytes: delegationWireSafeDecimal(ceilingRaw.maxMaterialBytes, "grant.ceiling.maxMaterialBytes"),
    maxMaterialItems: delegationWireSafeDecimal(ceilingRaw.maxMaterialItems, "grant.ceiling.maxMaterialItems"),
    maxResponseBytes: delegationWireSafeDecimal(ceilingRaw.maxResponseBytes, "grant.ceiling.maxResponseBytes"),
  });
  const seatId = delegationWireString(raw.seatId, "grant.seatId");
  if (principal.seatId !== seatId) {
    throw new NativeHostClientError("DELEGATION_REPLY", "grant seat binding mismatch");
  }
  return Object.freeze({
    binding,
    ceiling,
    expiresAtEpochMs: delegationWireSafeDecimal(raw.expiresAtEpochMs, "grant.expiresAtEpochMs"),
    grantRef,
    issuerId: delegationWireString(raw.issuerId, "grant.issuerId"),
    parentGrant: parent,
    policyRevision: delegationWireDecimal(raw.policyRevision, "grant.policyRevision"),
    principal,
    revision: delegationWireDecimal(raw.revision, "grant.revision"),
    revocationHead: delegationWireDecimal(raw.revocationHead, "grant.revocationHead"),
    seatId,
  });
}

export const encodeCurrentDelegationGrantFrame = (grantRef: string): string => {
  if (typeof grantRef !== "string" || grantRef.length === 0 || grantRef !== grantRef.trim()) {
    throw new NativeHostClientError("DELEGATION_FRAME", "grantRef must be canonical text");
  }
  return JSON.stringify({ grantRef, operation: "ReadCurrentDelegationGrant" });
};

export function encodeActionOutcomeFrame(
  reservationId: string,
  operationId: string,
  semanticDigest: string,
  outcome: DurableDispatchOutcome,
): string {
  const detail =
    outcome.kind === "not-sent" || outcome.kind === "rejected"
      ? outcome.reason
      : outcome.kind === "outcome-unknown"
        ? `${outcome.reason}:${outcome.detail}`
        : "";
  return JSON.stringify({
    detail,
    operation: "RecordActionOutcome",
    operationId,
    receiptRef: outcome.kind === "dispatched" ? (outcome.receiptRef ?? "") : "",
    reservationId,
    semanticDigest,
    state: outcome.kind,
  });
}

/**
 * Node talks to the native product store through a current-user named pipe.
 * The host process chooses the database child path; this client never
 * passes a SQLite filename or inherits OS handles.
 */
export class NativeHostClient {
  readonly #child: NodeChildProcess.ChildProcess;
  readonly #pipe: number;

  private constructor(child: NodeChildProcess.ChildProcess, pipe: number) {
    this.#child = child;
    this.#pipe = pipe;
  }

  static async attach(input: {
    readonly root: string;
    readonly hostBinary: string;
  }): Promise<NativeHostClient> {
    const child = NodeChildProcess.spawn(input.hostBinary, ["--root", input.root], {
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    if (child.stdout === null) {
      child.kill();
      throw new NativeHostClientError("HOST_STDIO", "native-host stdout was not created");
    }
    const stdout = NodeReadline.createInterface({ input: child.stdout });
    let handshake: Awaited<ReturnType<typeof readStartupHandshake>>;
    try {
      handshake = await readStartupHandshake(stdout);
    } catch (error) {
      stdout.close();
      child.kill();
      throw error;
    }
    const { pipeLine, capabilityLine } = handshake;
    stdout.close();
    if (!capabilityLine.startsWith("CAPABILITY\t")) {
      child.kill();
      throw new NativeHostClientError(
        "HOST_CAPABILITY",
        "native-host did not provide a service capability",
      );
    }
    const capability = capabilityLine.slice(11).trim();
    if (!/^[0-9a-f]{64}$/.test(capability)) {
      child.kill();
      throw new NativeHostClientError(
        "HOST_CAPABILITY",
        "native-host service capability is not canonical",
      );
    }
    const pipePath = pipeLine.slice(5).trim();
    const pipe = NodeFS.openSync(pipePath, "r+");
    NodeFS.writeSync(pipe, Buffer.from([0x47]));
    const client = new NativeHostClient(child, pipe);
    try {
      const authenticated = client.request(
        JSON.stringify({ capability, operation: "AuthenticateService" }),
      );
      if (authenticated.body !== '{"authenticated":true}') {
        throw new NativeHostClientError(
          "HOST_CAPABILITY",
          "native-host did not confirm service authentication",
        );
      }
      return client;
    } catch (error) {
      NodeFS.closeSync(pipe);
      child.kill();
      throw error;
    }
  }

  async commitProject(input: {
    readonly commandId: string;
    readonly projectId: string;
    readonly title: string;
    readonly workspaceRoot: string;
    readonly occurredAt: string;
  }): Promise<NativeHostReply> {
    const frame = {
      commandId: input.commandId,
      commandType: "project.create",
      events: [
        {
          eventId: `${input.commandId}:event`,
          occurredAt: input.occurredAt,
          projectId: input.projectId,
          title: input.title,
          type: "project.created",
          workspaceRoot: input.workspaceRoot,
        },
      ],
      operation: "CommitOrchestration",
    };
    return this.request(JSON.stringify(frame));
  }

  async readSnapshot(limit = 10): Promise<NativeHostReply> {
    return this.request(JSON.stringify({ limit: String(limit), operation: "ReadSnapshot" }));
  }

  async getReceipt(commandId: string): Promise<NativeHostReply> {
    return this.request(JSON.stringify({ commandId, operation: "GetReceipt" }));
  }

  async readProductIdentity(): Promise<NativeProductIdentitySnapshot> {
    return decodeProductIdentity(
      this.request(JSON.stringify({ operation: "ReadProductIdentity" })).body,
    );
  }

  async publishDecisionSnapshot(input: NativeDecisionAuthoritySnapshot): Promise<void> {
    const reply = this.request(encodeDecisionSnapshotFrame(input));
    if (reply.body !== '{"published":true}') {
      throw new NativeHostClientError("DECISION_REPLY", "Decision snapshot was not confirmed");
    }
  }

  async commitDecision(input: NativeDecisionCommitRequest): Promise<NativeDecisionCommitReply> {
    return decodeDecisionCommitReply(this.request(encodeDecisionCommitFrame(input)).body);
  }

  async readDecisionReplay(
    domainId: string,
    operationId: string,
  ): Promise<NativeDecisionCommitReply> {
    const result = decodeDecisionCommitReply(
      this.request(encodeDecisionReplayFrame(domainId, operationId)).body,
    );
    if (result.kind !== "replayed") {
      throw new NativeHostClientError(
        "DECISION_REPLY",
        "durable Decision read did not return replay history",
      );
    }
    return result;
  }

  async publishContextAssemblySnapshot(input: NativeContextAssemblySnapshot): Promise<void> {
    const reply = this.request(encodeContextAssemblySnapshotFrame(input));
    if (reply.body !== '{"published":true}') {
      throw new NativeHostClientError("CONTEXT_REPLY", "assembly snapshot was not confirmed");
    }
  }

  async commitTaskContextRequirements(
    input: CommitTaskContextRequirements,
  ): Promise<TaskContextRequirementsReceipt> {
    return decodeTaskContextRequirementsReceipt(
      this.request(encodeTaskContextCommitFrame(input)).body,
      input,
    );
  }

  async readTaskContextRequirements(
    input: ReadTaskContextRequirements,
  ): Promise<TaskContextRequirements> {
    return decodeTaskContextRequirements(
      this.request(encodeTaskContextReadFrame(input)).body,
      input,
    );
  }

  async readCurrentDelegationGrant(grantRef: string): Promise<NativeDelegationGrantSnapshot> {
    return decodeCurrentDelegationGrantReply(
      this.request(encodeCurrentDelegationGrantFrame(grantRef)).body,
      grantRef,
    );
  }

  async readGranteeContextSet(
    requests: ReadonlyArray<NativeGranteeContextReadRequest>,
  ): Promise<NativeAuthorizedContextReadSet> {
    return decodeAuthorizedContextReadSet(
      this.request(encodeGranteeContextSetFrame(requests)).body,
    );
  }

  async readContextAssemblyBasis(
    input: NativeContextManifestReplayIdentity,
  ): Promise<NativeContextAssemblyBasis> {
    return decodeContextAssemblyBasis(
      this.request(encodeContextAssemblyReadFrame("ReadContextAssemblyBasis", input)).body,
      input.operationId,
    );
  }

  async listContextAssemblySources(
    input: NativeContextManifestReplayIdentity,
  ): Promise<ReadonlyArray<NativeContextAssemblySource>> {
    return decodeContextAssemblySources(
      this.request(encodeContextAssemblyReadFrame("ListContextAssemblySources", input)).body,
      input.operationId,
    );
  }

  async commitContextManifest(
    input: NativeContextManifestCommitRequest,
  ): Promise<NativeContextManifestReceipt> {
    return decodeContextManifestReceipt(this.request(encodeContextManifestCommitFrame(input)).body);
  }

  async readContextManifest(
    input: NativeContextManifestReplayIdentity,
  ): Promise<NativeContextManifestReceipt> {
    const receipt = decodeContextManifestReceipt(
      this.request(encodeContextManifestReplayFrame(input)).body,
    );
    if (receipt.disposition !== "REPLAYED") {
      throw new NativeHostClientError(
        "CONTEXT_REPLY",
        "Manifest replay did not return durable history",
      );
    }
    return receipt;
  }

  async appendExecutionRecipe(input: NativeExecutionRecipeAppendRequest): Promise<NativeExecutionRecipeReceipt> {
    return decodeExecutionRecipeReceipt(this.request(encodeExecutionRecipeFrame(input)).body);
  }
  async appendObjectiveOutcome(input: NativeObjectiveOutcomeRequest): Promise<NativeAuthorityRecordReceipt> { return decodeAuthorityReceipt(this.request(encodeObjectiveOutcomeFrame(input)).body,"AppendObjectiveOutcome"); }
  async readObjectiveOutcome(domainId:string, outcomeId:string, revision:string): Promise<NativeAuthorityObjectSnapshot> { return decodeAuthoritySnapshot(this.request(JSON.stringify({domainId,operation:"ReadObjectiveOutcome",outcomeId,revision})).body); }
  async appendEvaluation(input: NativeEvaluationRequest): Promise<NativeAuthorityRecordReceipt> { return decodeAuthorityReceipt(this.request(encodeEvaluationFrame(input)).body,"AppendEvaluation"); }
  async readEvaluation(domainId:string, evaluationId:string, revision:string): Promise<NativeAuthorityObjectSnapshot> { return decodeAuthoritySnapshot(this.request(JSON.stringify({domainId,evaluationId,operation:"ReadEvaluation",revision})).body); }
  async appendDreamRun(input: NativeDreamRunRequest): Promise<NativeAuthorityRecordReceipt> { return decodeAuthorityReceipt(this.request(encodeDreamRunFrame(input)).body,"AppendDreamRun"); }
  async readDreamRun(domainId:string, runId:string, revision:string): Promise<NativeAuthorityObjectSnapshot> { return decodeAuthoritySnapshot(this.request(JSON.stringify({domainId,objectId:runId,operation:"ReadDreamRun",revision})).body); }
  async appendDreamProposal(input: NativeDreamProposalRequest): Promise<NativeAuthorityRecordReceipt> { return decodeAuthorityReceipt(this.request(encodeDreamProposalFrame(input)).body,"AppendDreamProposal"); }
  async readDreamProposal(domainId:string, proposalId:string, revision:string): Promise<NativeAuthorityObjectSnapshot> { return decodeAuthoritySnapshot(this.request(JSON.stringify({domainId,objectId:proposalId,operation:"ReadDreamProposal",revision})).body); }
  async readSessionLineage(domainId:string, sessionId:string): Promise<NativeSessionLineageSnapshot> { return decodeSessionLineageSnapshot(this.request(JSON.stringify({domainId,operation:"ReadSessionLineage",sessionId})).body); }
  async readExposureReceipt(domainId:string, receiptId:string): Promise<NativeExposureReceiptSnapshot> { return decodeExposureReceiptSnapshot(this.request(JSON.stringify({domainId,operation:"ReadExposureReceipt",receiptId})).body); }

  async commitContextVersion(input: CommitContextVersionRequest): Promise<ContextCommitReceipt> {
    return decodeContextCommitReply(this.request(encodeContextVersionFrame(input)).body);
  }

  async reserve(input: DurableActionReservation): Promise<ReserveActionResult> {
    return decodeActionReservationReply(this.request(encodeActionReservationFrame(input)).body);
  }

  async begin(reservationId: string, input: DurableActionReservation): Promise<BeginActionResult> {
    if (reservationId !== `reservation-${input.operationId}`) {
      throw new NativeHostClientError("ACTION_REQUEST", "begin reservation identity mismatch");
    }
    return decodeActionBeginReply(this.request(encodeActionBeginFrame(input)).body);
  }

  async recordDispatchOutcome(
    reservationId: string,
    operationId: string,
    semanticDigest: string,
    outcome: DurableDispatchOutcome,
  ): Promise<void> {
    const reply = this.request(
      encodeActionOutcomeFrame(reservationId, operationId, semanticDigest, outcome),
    );
    if (reply.body !== '{"recorded":true}') {
      throw new NativeHostClientError("ACTION_REPLY", "outcome receipt was not confirmed");
    }
  }

  async sendRawForTest(frame: string): Promise<NativeHostReply> {
    return this.request(frame);
  }

  async close(): Promise<void> {
    try {
      if (this.#child.exitCode === null) {
        await this.request(JSON.stringify({ operation: "Shutdown" }));
      }
    } catch {
      this.#child.kill();
    }
    NodeFS.closeSync(this.#pipe);
    await new Promise<void>((resolve) => {
      if (this.#child.exitCode !== null) {
        resolve();
        return;
      }
      this.#child.once("exit", () => resolve());
      setTimeout(() => {
        this.#child.kill();
        resolve();
      }, 2000);
    });
  }

  private request(frame: string): NativeHostReply {
    const payload = Buffer.from(frame, "utf8");
    const header = Buffer.alloc(4);
    header.writeUInt32LE(payload.length, 0);
    NodeFS.writeSync(this.#pipe, header);
    NodeFS.writeSync(this.#pipe, payload);
    const lengthBuf = Buffer.alloc(4);
    readExact(this.#pipe, lengthBuf);
    const body = Buffer.alloc(lengthBuf.readUInt32LE(0));
    readExact(this.#pipe, body);
    const line = body.toString("utf8");
    const [status, replyBody, elapsed] = line.trim().split("\t");
    const elapsedMicros = Number((elapsed ?? "0us").replace("us", ""));
    if (status === "OK") {
      return { ok: true, body: replyBody ?? "", elapsedMicros };
    }
    throw new NativeHostClientError("HOST_OPERATION", line.trim());
  }
}

function readExact(fd: number, buffer: Buffer): void {
  let offset = 0;
  while (offset < buffer.length) {
    const n = NodeFS.readSync(fd, buffer, offset, buffer.length - offset, null);
    if (n === 0) {
      throw new NativeHostClientError("HOST_EOF", "native-host pipe closed");
    }
    offset += n;
  }
}

export function readStartupHandshake(stdout: NodeReadline.Interface): Promise<{
  readonly pipeLine: string;
  readonly capabilityLine: string;
}> {
  return new Promise((resolve, reject) => {
    let stage = 0;
    let pipeLine = "";
    const onLine = (line: string) => {
      if (stage === 0) {
        if (!line.startsWith("LOCKED")) {
          cleanup();
          reject(new NativeHostClientError("HOST_LOCK", line));
        } else {
          stage = 1;
        }
      } else if (stage === 1) {
        if (!line.startsWith("PIPE\t")) {
          cleanup();
          reject(new NativeHostClientError("HOST_PIPE", line));
        } else {
          pipeLine = line;
          stage = 2;
        }
      } else {
        cleanup();
        resolve({ pipeLine, capabilityLine: line });
      }
    };
    const onClose = () => {
      cleanup();
      reject(new NativeHostClientError("HOST_EOF", "native-host closed stdout before handshake"));
    };
    const cleanup = () => {
      stdout.off("line", onLine);
      stdout.off("close", onClose);
    };
    stdout.on("line", onLine);
    stdout.once("close", onClose);
  });
}
