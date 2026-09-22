import { ContextRepository } from "../context/repository/repository.ts";
import { ContextManifestAssembler } from "../context/assembly/assembler.ts";
import {
  createNativeContextAssemblyAuthorityPort,
  type NativeBoundedContextSource,
  type NativeContextAssemblyPlan,
} from "../context/assembly/nativeAuthorityPort.ts";
import type { AssemblyRequest } from "../context/assembly/model.ts";
import { hashData, ownRecord } from "../context/assembly/passive.ts";
import {
  DecisionEngine,
  type DecisionBackend,
  type DecisionEligibilityPort,
  type DecisionRequest,
  type DecisionResult,
} from "../decision/engine/engine.ts";
import {
  createNativeDecisionCommitPort,
  type NativeDecisionCommitBasis,
} from "../decision/nativeAuthority.ts";
import type { ContextManifest } from "../contracts/model.ts";
import { calibrationReport } from "../evaluation/calibration.ts";
import {
  createDreamCandidate,
  createDreamRun,
  dreamEligibility,
  transitionDreamCandidate,
} from "../dream/dream.ts";
import {
  semanticDigestForAction,
  type DispatchActionResult,
  type TypedActionDispatcher,
  type TypedActionIntent,
} from "../actions/typedAction.ts";
import type { NativeStoreSession } from "./nativeStoreService.ts";

export interface CognitionVerticalConstruction {
  readonly assemblyPlan: NativeContextAssemblyPlan;
  readonly contentSource: NativeBoundedContextSource;
  readonly eligibility: DecisionEligibilityPort;
  readonly backend: DecisionBackend;
  readonly nativeBasis: NativeDecisionCommitBasis;
}

export interface CognitionActionLineage {
  readonly manifest: Readonly<
    Pick<
      ManifestLineage,
      | "manifestId"
      | "manifestHash"
      | "taskId"
      | "seatId"
      | "domainId"
      | "sessionId"
      | "bindingId"
      | "bindingGeneration"
    >
  >;
  readonly decision: Readonly<{
    operationId: string;
    receiptId: string;
    candidateId: string;
  }>;
  readonly action: Readonly<{
    operationId: string;
    semanticDigest: string;
  }>;
  readonly result: DispatchActionResult;
}

export interface CognitionVertical {
  /** Only manifests returned by this vertical's authoritative assembly can be used below. */
  assembleContext(input: AssemblyRequest): Promise<ContextManifest>;
  decideForContext(input: {
    readonly manifest: ContextManifest;
    readonly request: DecisionRequest;
  }): Promise<DecisionResult>;
  dispatchForDecision(input: {
    readonly manifest: ContextManifest;
    readonly decision: DecisionResult;
    readonly intent: TypedActionIntent;
  }): Promise<CognitionActionLineage>;
}

export interface GogokeCognitionService {
  readonly contextRepository: ContextRepository;
  readonly evaluation: Readonly<{
    calibrationReport: typeof calibrationReport;
  }>;
  readonly dream: Readonly<{
    eligibility: typeof dreamEligibility;
    createRun: typeof createDreamRun;
    createCandidate: typeof createDreamCandidate;
    transitionCandidate: typeof transitionDreamCandidate;
  }>;
  createVertical(input: CognitionVerticalConstruction): CognitionVertical;
}

interface ManifestLineage {
  readonly manifestId: string;
  readonly manifestHash: string;
  readonly taskId: string;
  readonly seatId: string;
  readonly domainId: string;
  readonly taskRevision: string;
  readonly policyRevision: string;
  readonly sessionId: string;
  readonly principalId: string;
  readonly bindingId: string;
  readonly bindingGeneration: string;
  readonly sourceEpoch: string;
  readonly authRevision: string;
  readonly runtimeInstanceId: string;
}

const MANIFEST_KEYS = Object.freeze([
  "manifestId",
  "taskId",
  "seatId",
  "bindingGeneration",
  "domainId",
  "policyRevision",
  "sourceSnapshot",
  "requiredConstraints",
  "includedVersions",
  "redactions",
  "selectionDecisionId",
  "manifestHash",
]);
const SOURCE_KEYS = Object.freeze([
  "assemblySchema",
  "operationId",
  "requestDigest",
  "principalId",
  "sessionId",
  "bindingId",
  "sourceEpoch",
  "runtimeInstanceId",
  "taskRevision",
  "authRevision",
  "revocationHead",
  "partitions",
  "mode",
  "excluded",
]);
const SHA256 = /^sha256:[0-9a-f]{64}$/u;

export class CognitionLineageError extends Error {
  override readonly name = "CognitionLineageError";
  readonly code:
    | "INVALID_MANIFEST_LINEAGE"
    | "INVALID_DECISION_LINEAGE"
    | "INVALID_ACTION_LINEAGE"
    | "VERTICAL_ALREADY_CREATED";
  constructor(code: CognitionLineageError["code"]) {
    super(code);
    this.code = code;
  }
}

function manifestLineage(value: unknown): ManifestLineage {
  const manifest = ownRecord(value, MANIFEST_KEYS);
  const source = ownRecord(manifest.sourceSnapshot, SOURCE_KEYS);
  const strings = [
    manifest.manifestId,
    manifest.taskId,
    manifest.seatId,
    manifest.bindingGeneration,
    manifest.domainId,
    manifest.policyRevision,
    source.sessionId,
    source.principalId,
    source.bindingId,
    source.sourceEpoch,
    source.taskRevision,
    source.authRevision,
    source.runtimeInstanceId,
  ];
  if (
    strings.some((item) => typeof item !== "string" || item.length === 0) ||
    source.assemblySchema !== "gogoke.context-assembly.v1" ||
    source.mode !== "FIXED_SOURCE_RULES" ||
    typeof manifest.manifestHash !== "string" ||
    !SHA256.test(manifest.manifestHash)
  ) {
    throw new CognitionLineageError("INVALID_MANIFEST_LINEAGE");
  }
  const body = Object.freeze({
    manifestId: manifest.manifestId,
    taskId: manifest.taskId,
    seatId: manifest.seatId,
    bindingGeneration: manifest.bindingGeneration,
    domainId: manifest.domainId,
    policyRevision: manifest.policyRevision,
    sourceSnapshot: manifest.sourceSnapshot,
    requiredConstraints: manifest.requiredConstraints,
    includedVersions: manifest.includedVersions,
    redactions: manifest.redactions,
    selectionDecisionId: manifest.selectionDecisionId,
  });
  if (hashData(body) !== manifest.manifestHash)
    throw new CognitionLineageError("INVALID_MANIFEST_LINEAGE");
  return Object.freeze({
    manifestId: manifest.manifestId as string,
    manifestHash: manifest.manifestHash,
    taskId: manifest.taskId as string,
    seatId: manifest.seatId as string,
    domainId: manifest.domainId as string,
    taskRevision: source.taskRevision as string,
    policyRevision: manifest.policyRevision as string,
    sessionId: source.sessionId as string,
    principalId: source.principalId as string,
    bindingId: source.bindingId as string,
    bindingGeneration: manifest.bindingGeneration as string,
    sourceEpoch: source.sourceEpoch as string,
    authRevision: source.authRevision as string,
    runtimeInstanceId: source.runtimeInstanceId as string,
  });
}

function boundDecisionHashes(lineage: ManifestLineage, request: DecisionRequest) {
  const scope = Object.freeze({
    schema: "gogoke.manifest-bound-decision.v1",
    manifestId: lineage.manifestId,
    manifestHash: lineage.manifestHash,
    taskId: lineage.taskId,
    seatId: lineage.seatId,
    domainId: lineage.domainId,
    sessionId: lineage.sessionId,
    bindingId: lineage.bindingId,
    bindingGeneration: lineage.bindingGeneration,
    authRevision: lineage.authRevision,
    runtimeInstanceId: lineage.runtimeInstanceId,
    taskRevision: lineage.taskRevision,
    policyRevision: lineage.policyRevision,
  });
  return Object.freeze({
    stateViewHash: hashData(
      Object.freeze({ ...scope, sourceStateViewHash: request.stateViewHash }),
    ),
    candidateHash: hashData(
      Object.freeze({ ...scope, sourceCandidateHash: request.candidateHash }),
    ),
  });
}

/**
 * Composition over the SAME native store and the caller's one authoritative
 * Action dispatcher. This layer has no Outcome writer/reader; it cannot turn an
 * Action receipt into a downstream Outcome or claim Outcome/Evaluation durability.
 */
export function constructCognitionService(
  store: NativeStoreSession,
  actionDispatcher: TypedActionDispatcher,
): GogokeCognitionService {
  const contextRepository = new ContextRepository({
    commitContextVersion: (request) => store.commitContextVersion(request),
  });
  const evaluation = Object.freeze({ calibrationReport });
  const dream = Object.freeze({
    eligibility: dreamEligibility,
    createRun: createDreamRun,
    createCandidate: createDreamCandidate,
    transitionCandidate: transitionDreamCandidate,
  });
  let verticalCreated = false;
  return Object.freeze({
    contextRepository,
    evaluation,
    dream,
    createVertical: (input: CognitionVerticalConstruction) => {
      if (verticalCreated) throw new CognitionLineageError("VERTICAL_ALREADY_CREATED");
      const nativeAuthority = createNativeContextAssemblyAuthorityPort({
        store,
        plan: input.assemblyPlan,
        content: input.contentSource,
      });
      // ContextManifestAssembler deliberately accepts only own data-property
      // methods. Capture the class adapter once without changing its authority.
      const assemblyAuthority = Object.freeze({
        openAssembly: nativeAuthority.openAssembly.bind(nativeAuthority),
        searchVisibleContext: nativeAuthority.searchVisibleContext.bind(nativeAuthority),
        loadVisibleVersions: nativeAuthority.loadVisibleVersions.bind(nativeAuthority),
        commitManifest: nativeAuthority.commitManifest.bind(nativeAuthority),
        readCurrentManifest: nativeAuthority.readCurrentManifest.bind(nativeAuthority),
      });
      const assembler = new ContextManifestAssembler(assemblyAuthority);
      verticalCreated = true;
      const requireCurrentManifest = async (lineage: ManifestLineage): Promise<void> => {
        const receipt = await store.readContextManifest({
          operationId: input.assemblyPlan.snapshot.operationId,
          principalId: lineage.principalId,
          seatId: lineage.seatId,
          taskId: lineage.taskId,
          sessionId: lineage.sessionId,
          domainId: lineage.domainId,
          bindingId: lineage.bindingId,
          bindingGeneration: lineage.bindingGeneration,
          sourceEpoch: lineage.sourceEpoch,
          runtimeInstanceId: lineage.runtimeInstanceId,
        });
        if (
          receipt.operationId !== input.assemblyPlan.snapshot.operationId ||
          receipt.manifestId !== lineage.manifestId ||
          receipt.manifestHash !== lineage.manifestHash
        ) {
          throw new CognitionLineageError("INVALID_MANIFEST_LINEAGE");
        }
      };
      const assembleContext = async (request: AssemblyRequest): Promise<ContextManifest> => {
        return assembler.assemble(request);
      };
      return Object.freeze({
        assembleContext,
        async decideForContext({
          manifest,
          request,
        }: {
          readonly manifest: ContextManifest;
          readonly request: DecisionRequest;
        }): Promise<DecisionResult> {
          const lineage = manifestLineage(manifest);
          await requireCurrentManifest(lineage);
          if (
            request.taskRevision !== lineage.taskRevision ||
            request.policyRevision !== lineage.policyRevision ||
            request.bindingGeneration !== lineage.bindingGeneration ||
            request.operationId !== input.nativeBasis.operationId ||
            input.nativeBasis.domainId !== lineage.domainId
          ) {
            throw new CognitionLineageError("INVALID_MANIFEST_LINEAGE");
          }
          const hashes = boundDecisionHashes(lineage, request);
          const candidates = new Map<string, NativeDecisionCommitBasis["candidates"][number]>();
          const boundCandidates = input.nativeBasis.candidates.map((candidate) => {
            const snapshot = candidate.snapshot;
            if (
              snapshot.taskRevision !== lineage.taskRevision ||
              snapshot.policyRevision !== lineage.policyRevision ||
              snapshot.bindingId !== lineage.bindingId ||
              snapshot.bindingGeneration !== lineage.bindingGeneration ||
              snapshot.authRevision !== lineage.authRevision ||
              snapshot.operationId !== input.nativeBasis.operationId ||
              snapshot.stateViewHash !== request.stateViewHash ||
              snapshot.candidateHash !== request.candidateHash ||
              candidates.has(candidate.candidateId)
            ) {
              throw new CognitionLineageError("INVALID_MANIFEST_LINEAGE");
            }
            const bound = Object.freeze({
              ...candidate,
              snapshot: Object.freeze({ ...snapshot, ...hashes }),
            });
            candidates.set(candidate.candidateId, bound);
            return bound;
          });
          const basis: NativeDecisionCommitBasis = Object.freeze({
            ...input.nativeBasis,
            candidates: Object.freeze(boundCandidates),
          });
          const engine = new DecisionEngine(
            input.eligibility,
            input.backend,
            createNativeDecisionCommitPort(store, basis),
          );
          const result = await engine.decide(Object.freeze({ ...request, ...hashes }));
          if (result.kind === "COMMITTED" || result.kind === "REPLAYED") {
            if (
              result.record.operationId !== input.nativeBasis.operationId ||
              result.record.stateViewHash !== hashes.stateViewHash ||
              result.record.candidateHash !== hashes.candidateHash ||
              result.record.taskRevision !== lineage.taskRevision ||
              result.record.policyRevision !== lineage.policyRevision ||
              result.record.bindingGeneration !== lineage.bindingGeneration
            ) {
              throw new CognitionLineageError("INVALID_DECISION_LINEAGE");
            }
          }
          return result;
        },
        async dispatchForDecision({
          manifest,
          decision,
          intent,
        }: {
          readonly manifest: ContextManifest;
          readonly decision: DecisionResult;
          readonly intent: TypedActionIntent;
        }): Promise<CognitionActionLineage> {
          const lineage = manifestLineage(manifest);
          await requireCurrentManifest(lineage);
          const durableDecision = await store.readDecisionReplay(
            lineage.domainId,
            decision.record.operationId,
          );
          if (
            (decision.kind !== "COMMITTED" && decision.kind !== "REPLAYED") ||
            durableDecision.kind !== "replayed" ||
            durableDecision.operationId !== decision.record.operationId ||
            durableDecision.decisionReceiptId !== decision.decisionReceiptId ||
            durableDecision.record.operationId !== decision.record.operationId ||
            durableDecision.record.choice !== decision.record.choice ||
            durableDecision.record.taskRevision !== lineage.taskRevision ||
            durableDecision.record.policyRevision !== lineage.policyRevision ||
            durableDecision.record.bindingGeneration !== lineage.bindingGeneration
          ) {
            throw new CognitionLineageError("INVALID_DECISION_LINEAGE");
          }
          const choice = decision.record.choice;
          const candidate = choice === null
            ? undefined
            : input.nativeBasis.candidates.find((item) => item.candidateId === choice);
          if (
            !candidate ||
            intent.binding.sessionId !== lineage.sessionId ||
            intent.binding.bindingId !== lineage.bindingId ||
            intent.binding.authRevision !== lineage.authRevision ||
            intent.binding.runtimeInstanceId !== lineage.runtimeInstanceId ||
            intent.binding.generation !== lineage.bindingGeneration ||
            candidate.snapshot.taskRevision !== lineage.taskRevision ||
            candidate.snapshot.policyRevision !== lineage.policyRevision ||
            candidate.snapshot.bindingId !== lineage.bindingId ||
            candidate.snapshot.bindingGeneration !== lineage.bindingGeneration ||
            candidate.snapshot.authRevision !== lineage.authRevision ||
            candidate.snapshot.operationId !== decision.record.operationId ||
            candidate.snapshot.actionOperationId !== intent.operationId ||
            candidate.snapshot.actionDigest !==
              semanticDigestForAction({
                schema: intent.schema,
                operationId: intent.operationId,
                binding: intent.binding,
                taskPackage: intent.taskPackage,
                action: intent.action,
              }) ||
            candidate.snapshot.actionDigest !== intent.semanticDigest
          ) {
            throw new CognitionLineageError("INVALID_ACTION_LINEAGE");
          }
          const result = await actionDispatcher.dispatch(intent);
          return Object.freeze({
            manifest: Object.freeze({
              manifestId: lineage.manifestId,
              manifestHash: lineage.manifestHash,
              taskId: lineage.taskId,
              seatId: lineage.seatId,
              domainId: lineage.domainId,
              sessionId: lineage.sessionId,
              bindingId: lineage.bindingId,
              bindingGeneration: lineage.bindingGeneration,
            }),
            decision: Object.freeze({
              operationId: decision.record.operationId,
              receiptId: decision.decisionReceiptId,
              candidateId: candidate.candidateId,
            }),
            action: Object.freeze({
              operationId: intent.operationId,
              semanticDigest: intent.semanticDigest,
            }),
            result,
          });
        },
      });
    },
  });
}
