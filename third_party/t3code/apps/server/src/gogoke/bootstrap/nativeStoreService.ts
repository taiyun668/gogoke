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
import {
  constructMinimalService,
  type MinimalServiceConstructionRequest,
} from "./minimalService.ts";
import { RootProfileOwnership } from "./rootProfileOwnership.ts";

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
  /** Releases the process-local owner only after native store shutdown is confirmed. */
  close(): Promise<void>;
}

export interface GogokeNativeStoreConstructionRequest {
  readonly request: MinimalServiceConstructionRequest;
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
  expected: ReadonlyArray<string>,
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
  const extra = names.find((name) => !expected.includes(name));
  if (extra !== undefined) return invalid(`${path}.${extra}`, "is not allowed");
  const missing = expected.find((name) => !names.includes(name));
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
    request: record.request as MinimalServiceConstructionRequest,
    root: absoluteWindowsPath(record.root, "input.root"),
    hostBinary: absoluteWindowsPath(record.hostBinary, "input.hostBinary"),
    ownership: record.ownership,
    connector: Object.freeze({
      attach: connector.attach.bind(record.connector) as NativeStoreConnector["attach"],
    }),
  });
}

/**
 * Narrow construction seam used by the canonical wrapper and focused tests.
 * It is intentionally not re-exported by bootstrap/index.ts.
 */
export async function constructNativeStoreServiceForAdapter(
  raw: InternalConstructionRequest,
): Promise<GogokeNativeStoreService> {
  const input = snapshotConstruction(raw);
  const constructed = await constructMinimalService({
    request: input.request,
    ownership: input.ownership,
    constructLocalNonModelService: () =>
      input.connector.attach({ root: input.root, hostBinary: input.hostBinary }),
  });
  let closed = false;
  let closing: Promise<void> | undefined;
  return Object.freeze({
    store: constructed.service,
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
