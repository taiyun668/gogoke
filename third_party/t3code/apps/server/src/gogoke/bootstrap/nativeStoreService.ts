import * as NodePath from "node:path";
import * as NodeUtilTypes from "node:util/types";

import type {
  NativeAuthorizedContextReadSet,
  NativeContextAssemblyBasis,
  NativeContextAssemblySource,
  NativeContextAssemblySnapshot,
  NativeDecisionAuthoritySnapshot,
  NativeDecisionCommitReply,
  NativeDecisionCommitRequest,
  NativeDelegationGrantSnapshot,
  NativeContextManifestCommitRequest,
  NativeContextManifestReceipt,
  NativeContextManifestReplayIdentity,
  NativeGranteeContextReadRequest,
  NativeHostReply,
  NativeProductIdentitySnapshot,
  NativeControllerCallerContext,
  NativeControllerAdmissionReceipt,
  NativeExecutionRecipeAppendRequest,
  NativeExecutionRecipeReceipt,
  NativeObjectiveOutcomeRequest, NativeEvaluationRequest, NativeAuthorityRecordReceipt,
  NativeAuthorityObjectSnapshot, NativeSessionLineageSnapshot, NativeExposureReceiptSnapshot,
  NativeDreamRunRequest, NativeDreamProposalRequest,
  CommitTaskContextRequirements,
  ReadTaskContextRequirements,
  TaskContextRequirements,
  TaskContextRequirementsReceipt,
} from "../persistence/base/nativeHostClient.ts";
import type {
  CommitContextVersionRequest,
  ContextCommitReceipt,
} from "../context/repository/repository.ts";
import type {
  BeginActionResult,
  DurableActionReservation,
  DurableDispatchOutcome,
  ReserveActionResult,
} from "../actions/typedAction.ts";
import { validateMinimalReleaseRequest } from "../releasePolicy.ts";
import {
  constructMinimalService,
  type MinimalServiceConstructionRequest,
} from "./minimalService.ts";
import {
  RootProfileOwnership,
  type RootProfileIdentity,
  type ServiceAuthority,
} from "./rootProfileOwnership.ts";

export interface NativeStoreSession extends NativeDelegationGrantReader {
  commitProject(input: {
    readonly commandId: string;
    readonly projectId: string;
    readonly title: string;
    readonly workspaceRoot: string;
    readonly occurredAt: string;
  }): Promise<NativeHostReply>;
  readSnapshot(limit?: number): Promise<NativeHostReply>;
  getReceipt(commandId: string): Promise<NativeHostReply>;
  /** Native Product Authority identity; the service capability itself is not a grant. */
  readonly readProductIdentity?: () => Promise<NativeProductIdentitySnapshot>;
  readonly admitControllerCaller?: (
    input: NativeControllerCallerContext,
  ) => Promise<NativeControllerAdmissionReceipt>;
  commitContextVersion(input: CommitContextVersionRequest): Promise<ContextCommitReceipt>;
  reserve(input: DurableActionReservation): Promise<ReserveActionResult>;
  begin(reservationId: string, input: DurableActionReservation): Promise<BeginActionResult>;
  recordDispatchOutcome(
    reservationId: string,
    operationId: string,
    semanticDigest: string,
    outcome: DurableDispatchOutcome,
  ): Promise<void>;
  publishDecisionSnapshot(input: NativeDecisionAuthoritySnapshot): Promise<void>;
  commitDecision(input: NativeDecisionCommitRequest): Promise<NativeDecisionCommitReply>;
  readDecisionReplay(domainId: string, operationId: string): Promise<NativeDecisionCommitReply>;
  publishContextAssemblySnapshot(input: NativeContextAssemblySnapshot): Promise<void>;
  commitTaskContextRequirements(
    input: CommitTaskContextRequirements,
  ): Promise<TaskContextRequirementsReceipt>;
  readTaskContextRequirements(input: ReadTaskContextRequirements): Promise<TaskContextRequirements>;
  readContextAssemblyBasis(
    input: NativeContextManifestReplayIdentity,
  ): Promise<NativeContextAssemblyBasis>;
  listContextAssemblySources(
    input: NativeContextManifestReplayIdentity,
  ): Promise<ReadonlyArray<NativeContextAssemblySource>>;
  readGranteeContextSet(
    requests: ReadonlyArray<NativeGranteeContextReadRequest>,
  ): Promise<NativeAuthorizedContextReadSet>;
  commitContextManifest(
    input: NativeContextManifestCommitRequest,
  ): Promise<NativeContextManifestReceipt>;
  readContextManifest(
    input: NativeContextManifestReplayIdentity,
  ): Promise<NativeContextManifestReceipt>;
  readonly appendExecutionRecipe?: (input: NativeExecutionRecipeAppendRequest) => Promise<NativeExecutionRecipeReceipt>;
  readonly appendObjectiveOutcome?: (input: NativeObjectiveOutcomeRequest) => Promise<NativeAuthorityRecordReceipt>;
  readonly readObjectiveOutcome?: (domainId:string, outcomeId:string, revision:string) => Promise<NativeAuthorityObjectSnapshot>;
  readonly appendEvaluation?: (input: NativeEvaluationRequest) => Promise<NativeAuthorityRecordReceipt>;
  readonly readEvaluation?: (domainId:string, evaluationId:string, revision:string) => Promise<NativeAuthorityObjectSnapshot>;
  readonly appendDreamRun?: (input: NativeDreamRunRequest) => Promise<NativeAuthorityRecordReceipt>;
  readonly readDreamRun?: (domainId:string, runId:string, revision:string) => Promise<NativeAuthorityObjectSnapshot>;
  readonly appendDreamProposal?: (input: NativeDreamProposalRequest) => Promise<NativeAuthorityRecordReceipt>;
  readonly readDreamProposal?: (domainId:string, proposalId:string, revision:string) => Promise<NativeAuthorityObjectSnapshot>;
  readonly readSessionLineage?: (domainId:string, sessionId:string) => Promise<NativeSessionLineageSnapshot>;
  readonly readExposureReceipt?: (domainId:string, receiptId:string) => Promise<NativeExposureReceiptSnapshot>;
  close(): Promise<void>;
}

export interface NativeDelegationGrantReader {
  /** Implemented by the native Product Authority client; absent adapters fail closed. */
  readonly readCurrentDelegationGrant?: (grantRef: string) => Promise<NativeDelegationGrantSnapshot>;
}

export interface NativeStoreConnector {
  attach(input: {
    readonly root: string;
    readonly hostBinary: string;
  }): Promise<NativeStoreSession>;
}

export interface GogokeNativeStoreService {
  readonly store: NativeStoreSession;
  /** Actual Product Authority identity bound to this native-host instance. */
  readonly identity: NativeProductIdentitySnapshot;
  /** Releases the process-local owner only after native store shutdown is confirmed. */
  close(): Promise<void>;
}

export interface NativeStoreAdmissionRequest {
  readonly authority: ServiceAuthority;
  /** Optional expectations only; Product Authority supplies the authoritative identity. */
  readonly rootIdentity?: string;
  readonly profileId?: string;
  readonly requestedCapabilities?: readonly string[];
  readonly enabledRuntimeDriverIds?: readonly string[];
}

export interface GogokeNativeStoreConstructionRequest {
  readonly request: NativeStoreAdmissionRequest;
  readonly root: string;
  readonly hostBinary: string;
}

type InternalConstructionRequest = GogokeNativeStoreConstructionRequest & {
  readonly ownership: RootProfileOwnership;
  readonly connector: NativeStoreConnector;
};

const invalid = (path: string, detail: string): never => {
  throw new Error(`INVALID_NATIVE_STORE_CONSTRUCTION: ${path} ${detail}`);
};

function passiveRecord(
  value: unknown,
  path: string,
  required: ReadonlyArray<string>,
  optional: ReadonlyArray<string> = [],
): Readonly<Record<string, unknown>> {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    NodeUtilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) {
    return invalid(path, "must be a non-Proxy plain object");
  }
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key === "symbol")) return invalid(path, "has symbol keys");
  const names = keys as ReadonlyArray<string>;
  const allowed = new Set([...required, ...optional]);
  const extra = names.find((name) => !allowed.has(name));
  if (extra !== undefined) return invalid(`${path}.${extra}`, "is not allowed");
  const missing = required.find((name) => !names.includes(name));
  if (missing !== undefined) return invalid(`${path}.${missing}`, "is required");
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const snapshot: Record<string, unknown> = {};
  for (const name of names) {
    const descriptor = descriptors[name];
    if (descriptor === undefined || !("value" in descriptor) || !descriptor.enumerable) {
      return invalid(`${path}.${name}`, "must be an enumerable data property");
    }
    snapshot[name] = descriptor.value;
  }
  return Object.freeze(snapshot);
}

function canonicalString(value: unknown, path: string): string {
  if (typeof value !== "string" || value.length === 0 || value !== value.trim()) {
    return invalid(path, "must be canonical text");
  }
  return value;
}

function canonicalStringArray(value: unknown, path: string): readonly string[] {
  if (
    !Array.isArray(value) ||
    NodeUtilTypes.isProxy(value) ||
    Object.getPrototypeOf(value) !== Array.prototype
  ) {
    return invalid(path, "must be a non-Proxy plain array");
  }
  const descriptors=Object.getOwnPropertyDescriptors(value);
  const result:string[]=[];
  for(let index=0;index<value.length;index+=1){
    const descriptor=descriptors[String(index)];
    if(descriptor===undefined||!("value" in descriptor)||descriptor.enumerable!==true){
      return invalid(`${path}[${index}]`,"must be an enumerable data property");
    }
    result.push(canonicalString(descriptor.value,`${path}[${index}]`));
  }
  const allowed=new Set(["length",...result.map((_value,index)=>String(index))]);
  if(Reflect.ownKeys(descriptors).some(key=>typeof key!=="string"||!allowed.has(key))){
    return invalid(path,"has extra properties");
  }
  if (new Set(result).size !== result.length) return invalid(path, "must not contain duplicates");
  return Object.freeze(result);
}

function absoluteWindowsPath(value: unknown, path: string): string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value !== value.trim() ||
    value.includes("\0") ||
    !NodePath.win32.isAbsolute(value)
  ) {
    return invalid(path, "must be a canonical absolute Windows path");
  }
  return value;
}

function snapshotAdmissionRequest(value: unknown): NativeStoreAdmissionRequest {
  const record = passiveRecord(value, "input.request", ["authority"], [
    "rootIdentity",
    "profileId",
    "requestedCapabilities",
    "enabledRuntimeDriverIds",
  ]);
  const authority = record.authority;
  if (authority !== "legacy" && authority !== "public") {
    return invalid("input.request.authority", "must be legacy or public");
  }
  const rootIdentity = record.rootIdentity === undefined
    ? undefined
    : canonicalString(record.rootIdentity, "input.request.rootIdentity");
  const profileId = record.profileId === undefined
    ? undefined
    : canonicalString(record.profileId, "input.request.profileId");
  const requestedCapabilities = record.requestedCapabilities === undefined
    ? undefined
    : canonicalStringArray(record.requestedCapabilities, "input.request.requestedCapabilities");
  const enabledRuntimeDriverIds = record.enabledRuntimeDriverIds === undefined
    ? undefined
    : canonicalStringArray(record.enabledRuntimeDriverIds, "input.request.enabledRuntimeDriverIds");
  validateMinimalReleaseRequest({ requestedCapabilities, enabledRuntimeDriverIds });
  return Object.freeze({
    authority,
    ...(rootIdentity === undefined ? {} : { rootIdentity }),
    ...(profileId === undefined ? {} : { profileId }),
    ...(requestedCapabilities === undefined ? {} : { requestedCapabilities }),
    ...(enabledRuntimeDriverIds === undefined ? {} : { enabledRuntimeDriverIds }),
  });
}

function snapshotConstruction(input: InternalConstructionRequest): InternalConstructionRequest {
  const record = passiveRecord(input, "input", [
    "request",
    "root",
    "hostBinary",
    "ownership",
    "connector",
  ]);
  const connector = passiveRecord(record.connector, "input.connector", ["attach"]);
  if (typeof connector.attach !== "function") {
    return invalid("input.connector.attach", "must be a data-property function");
  }
  if (!(record.ownership instanceof RootProfileOwnership)) {
    return invalid("input.ownership", "must be a RootProfileOwnership");
  }
  return Object.freeze({
    request: snapshotAdmissionRequest(record.request),
    root: absoluteWindowsPath(record.root, "input.root"),
    hostBinary: absoluteWindowsPath(record.hostBinary, "input.hostBinary"),
    ownership: record.ownership,
    connector: Object.freeze({
      attach: connector.attach.bind(record.connector) as NativeStoreConnector["attach"],
    }),
  });
}

function assertExpectedIdentity(
  request: NativeStoreAdmissionRequest,
  identity: NativeProductIdentitySnapshot,
): void {
  const expectations: ReadonlyArray<readonly [keyof RootProfileIdentity, string | undefined, string]> = [
    ["rootIdentity", request.rootIdentity, identity.rootIdentity],
    ["profileId", request.profileId, identity.profileId],
  ];
  for (const [field, expected, actual] of expectations) {
    if (expected !== undefined && expected !== actual) {
      invalid(`input.request.${field}`, "does not match native Product Authority");
    }
  }
}

/**
 * Narrow construction seam used by the canonical wrapper and focused tests.
 * Release policy and caller shape are validated before native process creation.
 * The authoritative root/profile/Seat identity is then read from the authenticated
 * Product Authority channel before process-local ownership is acquired.
 */
export async function constructNativeStoreServiceForAdapter(
  raw: InternalConstructionRequest,
): Promise<GogokeNativeStoreService> {
  const input = snapshotConstruction(raw);
  const session = await input.connector.attach({ root: input.root, hostBinary: input.hostBinary });
  let identity: NativeProductIdentitySnapshot;
  try {
    if (typeof session.readProductIdentity !== "function") {
      return invalid("native.readProductIdentity", "is required for public construction");
    }
    identity = await session.readProductIdentity();
    assertExpectedIdentity(input.request, identity);
  } catch (error) {
    try {
      await session.close();
    } catch (closeError) {
      throw new AggregateError(
        [error, closeError],
        "native identity admission failed and shutdown is unknown",
      );
    }
    throw error;
  }

  let constructed;
  try {
    const request: MinimalServiceConstructionRequest = Object.freeze({
      authority: input.request.authority,
      rootIdentity: identity.rootIdentity,
      profileId: identity.profileId,
      ...(input.request.requestedCapabilities === undefined
        ? {}
        : { requestedCapabilities: input.request.requestedCapabilities }),
      ...(input.request.enabledRuntimeDriverIds === undefined
        ? {}
        : { enabledRuntimeDriverIds: input.request.enabledRuntimeDriverIds }),
    });
    constructed = constructMinimalService({
      request,
      ownership: input.ownership,
      constructLocalNonModelService: () => session,
    });
  } catch (error) {
    try {
      await session.close();
    } catch (closeError) {
      throw new AggregateError(
        [error, closeError],
        "native ownership admission failed and shutdown is unknown",
      );
    }
    throw error;
  }

  let closed = false;
  let closing: Promise<void> | undefined;
  return Object.freeze({
    store: constructed.service,
    identity,
    close: () => {
      if (closed) return Promise.resolve();
      if (closing !== undefined) return closing;
      closing = Promise.resolve()
        .then(() => constructed.service.close())
        .then(() => {
          closed = true;
          constructed.ownership.release();
        })
        .finally(() => {
          if (!closed) closing = undefined;
        });
      return closing;
    },
  });
}
