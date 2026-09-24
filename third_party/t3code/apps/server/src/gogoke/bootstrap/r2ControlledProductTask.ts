import { createHash } from "node:crypto";

import { PiManagedSession } from "../adapters/pi/session.ts";
import {
  validateControlledFixtureResult,
  type ValidatedControlledFixtureResult,
} from "../actions/controlledFixtureResult.ts";
import {
  prepareR2ControlledManifest,
  prepareR2PublicContext,
} from "../context/assembly/r2ControlledManifest.ts";
import type { GitFactReadback } from "../context/repository/gitFact.ts";
import { commitR2ControlledDecision } from "../decision/r2ControlledDecision.ts";
import type { NativeProductIdentitySnapshot } from "../persistence/base/nativeHostClient.ts";
import type { NativeStoreSession } from "./nativeStoreService.ts";

export type R2ControlledProductTaskResult =
  | {
      readonly state: "VALIDATED_TEST_RESULT_NOT_ADOPTED";
      readonly result: ValidatedControlledFixtureResult;
      readonly actionCompletionRef: string;
      readonly manifestHash: string;
      readonly decisionReceiptId: string;
      readonly objectiveOutcomeContentHash: string;
      readonly objectiveOutcomeReceiptId: string;
      readonly evaluationContentHash: string;
      readonly evaluationReceiptId: string;
      readonly metricsHash: string;
      readonly dreamRunContentHash: string;
      readonly dreamProposalContentHash: string;
      readonly dreamProposalState: "DRAFT_TEST_ONLY_NOT_ACTIVATED";
      readonly fixtureDriverBinding?: {
        readonly driverId: string;
        readonly adapterVersion: "1.0.0";
        readonly runtimeInstanceId: string;
        readonly launchDigestSha256: string;
      };
    }
  | {
      readonly state: "ACTION_REPLAY_NO_NEW_RESULT";
      readonly reservationState: "dispatching" | "not-sent" | "dispatched" |
        "rejected" | "outcome-unknown" | "completed";
      readonly actionCompletionRef?: string;
      readonly decisionReceiptId: string;
    };

/** One fixed public R2-02 task, using native custody for its only Action. */
export async function runR2ControlledProductTask(input: {
  readonly store: NativeStoreSession;
  readonly identity: NativeProductIdentitySnapshot;
  readonly source: GitFactReadback;
  readonly fixtureDriverId?: string;
}): Promise<R2ControlledProductTaskResult> {
  const { store, identity, source, fixtureDriverId } = input;
  const sourceHash = createHash("sha256").update(source.bytes).digest("hex");
  const sourceBlob = createHash("sha1")
    .update(`blob ${source.bytes.length}\0`).update(source.bytes).digest("hex");
  if (source.state !== "COMMITTED_BYTES_VERIFIED_NOT_ADOPTED" ||
      source.coordinate.contentHash !== `sha256:${sourceHash}` ||
      source.gitBlob !== sourceBlob) throw new Error("R2_PUBLIC_SOURCE_BYTES_MISMATCH");
  const required = [
    "admitControllerCaller", "prepareR2TestDelegation", "prepareR2TestContextGrant",
    "prepareR2TestTask", "prepareR2TestPackage", "prepareR2TestLineage",
    "prepareR2TestRecipe", "readR2TestActionDecisionBasis", "prepareR2TestAction",
    "runControlledFixtureAction", "readR2ObjectiveFactRefs",
    "appendObjectiveOutcome", "readObjectiveOutcome",
    "appendEvaluation", "readEvaluation",
    "prepareR2TestRollbackPlan", "appendDreamRun", "readDreamRun",
    "appendDreamProposal", "readDreamProposal",
  ] as const;
  for (const name of required) {
    if (typeof store[name] !== "function") throw new Error(`R2_CONTROLLED_TASK_UNAVAILABLE: ${name}`);
  }
  const caller = {
    policyRevision: identity.policyRevision,
    principalId: identity.principalId,
    profileId: identity.profileId,
    revocationHead: identity.revocationHead,
    role: "controller" as const,
    seatId: identity.seatId,
  };
  const admission = await store.admitControllerCaller!(caller);
  if (admission.admitted !== true || admission.principalId !== caller.principalId ||
      admission.seatId !== caller.seatId || admission.profileId !== caller.profileId ||
      admission.policyRevision !== caller.policyRevision ||
      admission.revocationHead !== caller.revocationHead) {
    throw new Error("R2_CONTROLLER_ADMISSION_MISMATCH");
  }

  const grant = await store.prepareR2TestDelegation!(caller);
  let fixtureDriverRuntimeId: string | undefined;
  if (fixtureDriverId !== undefined) {
    if (!/^mock_novel_[0-9a-f]{16}$/u.test(fixtureDriverId) ||
        typeof store.registerR2TestFixtureDriver !== "function" ||
        typeof store.readR2TestFixtureActionBinding !== "function") {
      throw new Error("R2_NOVEL_FIXTURE_DRIVER_UNAVAILABLE");
    }
    const registration = await store.registerR2TestFixtureDriver(caller, fixtureDriverId);
    if (registration.state !== "TEST_ONLY_FIXTURE_DRIVER_REGISTERED" ||
        registration.driverId !== fixtureDriverId ||
        registration.adapterVersion !== "1.0.0" ||
        !/^runtime-r2-03-[0-9a-f]{16}-[0-9a-f]{16}$/u.test(registration.runtimeInstanceId) ||
        !registration.runtimeInstanceId.startsWith(`runtime-r2-03-${fixtureDriverId.slice(11)}-`)) {
      throw new Error("R2_NOVEL_FIXTURE_REGISTRATION_MISMATCH");
    }
    fixtureDriverRuntimeId = registration.runtimeInstanceId;
  }
  const contextGrant = await store.prepareR2TestContextGrant!(caller);
  await prepareR2PublicContext({
    store, source, grant: contextGrant, policyRevision: identity.policyRevision,
  });
  const task = await store.prepareR2TestTask!(caller);
  if (task.state !== "TEST_ONLY_TASK_PREPARED_NOT_ACTION" ||
      task.taskId !== "task-r2-02-test") throw new Error("R2_TEST_TASK_MISMATCH");
  const message = JSON.stringify({
    schema: "gogoke.s1-r4.r2-02.fixture-task.v1", testOnly: true,
    source: {
      repository: source.coordinate.repository,
      commit: source.coordinate.commit,
      path: source.coordinate.path,
      sha256: sourceHash,
      content: Buffer.from(source.bytes).toString("utf8"),
    },
  });
  const promptJson = JSON.stringify({ type: "prompt", message, id: "gogoke-pi-1" });
  const packageReceipt = await store.prepareR2TestPackage!(caller, promptJson);
  const lineage = await store.prepareR2TestLineage!(caller);
  const recipe = await store.prepareR2TestRecipe!(caller, fixtureDriverRuntimeId);
  if (packageReceipt.state !== "TEST_ONLY_PACKAGE_PREPARED_NOT_ACTION" ||
      lineage.state !== "TEST_ONLY_LINEAGE_PREPARED_NOT_ACTION" ||
      recipe.state !== "TEST_ONLY_RECIPE_PREPARED_NOT_ACTION") {
    throw new Error("R2_TEST_PREPARATION_MISMATCH");
  }
  const basis = await store.readR2TestActionDecisionBasis!(caller, promptJson);
  if (basis.state !== "TEST_ONLY_DECISION_BASIS_NOT_ACTION" ||
      basis.taskRevision !== task.taskRevision ||
      basis.bindingId !== lineage.bindingId ||
      basis.bindingGeneration !== lineage.generation ||
      basis.policyRevision !== identity.policyRevision) {
    throw new Error("R2_TEST_DECISION_BASIS_MISMATCH");
  }
  // This one test-only operation uses a fixed record identity across process
  // restarts, including a lost reply after the manifest was committed.
  const recordedAt = "2026-09-23T00:00:00.000Z";
  const decision = await commitR2ControlledDecision({ store, basis, grant, recordedAt });
  if (decision.kind !== "committed" && decision.kind !== "replayed") {
    throw new Error("R2_TEST_DECISION_NOT_COMMITTED");
  }
  const action = await store.prepareR2TestAction!({
    grantRef: grant.grantRef,
    promptJson,
    expectedActionDigest: basis.actionDigest,
    expectedPackageDigest: packageReceipt.packageDigest,
  });
  if (action.packageDigest !== packageReceipt.packageDigest ||
      action.semanticDigest !== basis.actionDigest) {
    throw new Error("R2_TEST_ACTION_RESERVATION_MISMATCH");
  }
  if (action.reservationState === "completed") {
    // Native reads its trusted completion receipt before any process or protocol write.
    const completion = await store.runControlledFixtureAction!({
      caller,
      domainId: "domain-r2-02-test",
      operationId: action.operationId,
      reservationId: action.reservationId,
      promptJson,
    });
    if (completion.state !== "ACTION_COMPLETION_RECONCILED_NOT_RESULT") {
      throw new Error("R2_TEST_COMPLETION_RECONCILIATION_MISMATCH");
    }
    return Object.freeze({
      state: "ACTION_REPLAY_NO_NEW_RESULT" as const,
      reservationState: "completed" as const,
      actionCompletionRef: completion.actionCompletionRef,
      decisionReceiptId: decision.decisionReceiptId,
    });
  }
  if (action.reservationState !== "reserved") {
    return Object.freeze({
      state: "ACTION_REPLAY_NO_NEW_RESULT" as const,
      reservationState: action.reservationState,
      decisionReceiptId: decision.decisionReceiptId,
    });
  }
  const manifest = await prepareR2ControlledManifest({
    store, basis, grant, contextGrant, source, recordedAt,
    ...(fixtureDriverRuntimeId === undefined ? {} : { runtimeInstanceId: fixtureDriverRuntimeId }),
  });
  if (manifest.manifestId !== "manifest-r2-02-test" || manifest.includedVersions.length !== 1) {
    throw new Error("R2_TEST_MANIFEST_MISMATCH");
  }

  let session!: PiManagedSession;
  let actionCompletionRef: string | undefined;
  session = new PiManagedSession({
    admission: { mode: "ordinary", protocolQualified: true,
      protectedDomainQualified: false, contextExposure: "UNKNOWN" },
    sink: { async write(chunk) {
      if (Buffer.from(chunk).toString("utf8") !== `${promptJson}\n`) {
        throw new Error("R2_TEST_PROMPT_BYTES_MISMATCH");
      }
      const evidence = await store.runControlledFixtureAction!({
        caller,
        domainId: "domain-r2-02-test",
        operationId: action.operationId,
        reservationId: action.reservationId,
        promptJson,
      });
      if (evidence.state !== "ACTION_TRANSPORT_COMPLETED_NOT_RESULT") {
        throw new Error("R2_TEST_ACTION_NO_FRESH_FRAMES");
      }
      actionCompletionRef = evidence.actionCompletionRef;
      for (const frame of evidence.frames) session.acceptStdout(Buffer.from(frame));
    } },
  });
  const observation = await session.promptAndObserveSettlement(message, 30_000);
  if (actionCompletionRef === undefined) throw new Error("R2_TEST_ACTION_COMPLETION_MISSING");
  const result = validateControlledFixtureResult(source, observation);
  const fixtureDriverBinding = fixtureDriverId === undefined ? undefined :
    await store.readR2TestFixtureActionBinding!(caller, actionCompletionRef);
  if (fixtureDriverBinding !== undefined &&
      (fixtureDriverBinding.state !== "TEST_ONLY_ACTION_BINDING" ||
       fixtureDriverBinding.driverId !== fixtureDriverId ||
       fixtureDriverBinding.adapterVersion !== "1.0.0" ||
       fixtureDriverBinding.runtimeInstanceId !== fixtureDriverRuntimeId)) {
    throw new Error("R2_NOVEL_FIXTURE_ACTION_BINDING_MISMATCH");
  }
  const refs = await store.readR2ObjectiveFactRefs!(caller, actionCompletionRef);
  if (refs.manifestHash !== manifest.manifestHash) {
    throw new Error("R2_TEST_OBJECTIVE_MANIFEST_MISMATCH");
  }
  const completedMs = Date.parse(refs.actionCompletedAt);
  if (!Number.isFinite(completedMs)) throw new Error("R2_TEST_COMPLETION_TIME_INVALID");
  const observedAt = new Date(Math.max(Date.now(), completedMs + 1)).toISOString();
  const outcome = await store.appendObjectiveOutcome!({
    domainId: "domain-r2-02-test",
    outcomeId: "outcome-r2-02-test",
    revision: "1",
    expectedPreviousRevision: null,
    expectedPreviousContentHash: null,
    operationId: "outcome-op-r2-02-test",
    eventId: "outcome-event-r2-02-test",
    receiptId: "outcome-receipt-r2-02-test",
    recordedAt: observedAt,
    manifestId: "manifest-r2-02-test",
    manifestVersion: "1",
    manifestHash: refs.manifestHash,
    decisionId: "decision-r2-02-test",
    decisionVersion: "1",
    decisionHash: refs.decisionContentHash,
    actionOperationId: action.operationId,
    actionCompletionRef,
    resultRefs: [{ objectType: "ActionCompletion", objectId: action.operationId,
      revision: "1", contentHash: refs.actionCompletionHash }],
    evidenceRefs: [{ objectType: "ContextManifest", objectId: manifest.manifestId,
      revision: "1", contentHash: refs.manifestContentHash }],
    observationStartsAt: refs.actionCompletedAt,
    observationEndsAt: observedAt,
    observationStatus: "OBSERVED",
  });
  const outcomeReadback = await store.readObjectiveOutcome!(
    "domain-r2-02-test", "outcome-r2-02-test", "1",
  );
  if (outcome.disposition !== "COMMITTED" ||
      outcomeReadback.contentHash !== outcome.contentHash) {
    throw new Error("R2_TEST_OBJECTIVE_OUTCOME_READBACK_MISMATCH");
  }
  const metrics = JSON.stringify({
    schema: "gogoke.s1-r4.r2-02.fixture-metrics.v1",
    testOnly: true,
    exactSourceMatch: true,
    reportSha256: result.reportSha256,
    sourceBlob: result.sourceBlob,
  });
  const metricsHash = `sha256:${createHash("sha256").update(metrics).digest("hex")}`;
  const evaluation = await store.appendEvaluation!({
    domainId: "domain-r2-02-test",
    evaluationId: "evaluation-r2-02-test",
    revision: "1",
    expectedPreviousRevision: null,
    expectedPreviousContentHash: null,
    operationId: "evaluation-op-r2-02-test",
    eventId: "evaluation-event-r2-02-test",
    receiptId: "evaluation-receipt-r2-02-test",
    recordedAt: observedAt,
    sourceIdentity: "deterministic-public-fixture",
    outcomeRefs: [{ objectType: "OutcomeRecord", objectId: outcome.objectId,
      revision: outcome.revision, contentHash: outcome.contentHash }],
    decisionFamily: "CONTEXT_SELECTION",
    scorerVersion: "1",
    rubricVersion: "1",
    calibrationKey: "r2-02-fixture",
    calibrationVersion: "1",
    datasetNamespace: "test/s1-r4/r2-02",
    datasetSplit: "controlled-public-fixture",
    evidenceRefs: [{ objectType: "ActionCompletion", objectId: action.operationId,
      revision: "1", contentHash: refs.actionCompletionHash }],
    metricsHash,
    safetyStatus: "REVIEW_REQUIRED",
    privacyStatus: "CLEAR",
  });
  const evaluationReadback = await store.readEvaluation!(
    "domain-r2-02-test", "evaluation-r2-02-test", "1",
  );
  if (evaluation.disposition !== "COMMITTED" ||
      evaluationReadback.contentHash !== evaluation.contentHash) {
    throw new Error("R2_TEST_EVALUATION_READBACK_MISMATCH");
  }
  const rollback = await store.prepareR2TestRollbackPlan!(caller);
  if (rollback.state !== "TEST_ONLY_ROLLBACK_PLAN_NOT_ACTIVATED" ||
      rollback.disposition !== "COMMITTED") {
    throw new Error("R2_TEST_ROLLBACK_PLAN_NOT_PREPARED");
  }
  const dreamRun = await store.appendDreamRun!({
    domainId: "domain-r2-02-test",
    runId: "dream-run-r2-02-test",
    revision: "1",
    expectedPreviousRevision: null,
    expectedPreviousContentHash: null,
    operationId: "dream-run-op-r2-02-test",
    eventId: "dream-run-event-r2-02-test",
    receiptId: "dream-run-receipt-r2-02-test",
    recordedAt: observedAt,
    sourceIdentity: "deterministic-public-fixture",
    inputSnapshot: { objectType: "ContextManifest", objectId: manifest.manifestId,
      revision: "1", contentHash: refs.manifestContentHash },
    datasetNamespace: "test/s1-r4/r2-02",
    datasetSplit: "controlled-public-fixture",
    datasetSplitHash: source.coordinate.contentHash,
    recipeRef: { objectType: "ExecutionRecipe", objectId: recipe.recipeId,
      revision: recipe.revision, contentHash: recipe.contentHash },
    budgetLease: { leaseRef: "capacity-lease-r2-02", operationId: "decision-r2-02-test",
      resourceRef: "capacity-r2-02-fixture", resourceRevision: "1", units: "1" },
    evaluationRefs: [{ objectType: "EvaluationRecord", objectId: evaluation.objectId,
      revision: evaluation.revision, contentHash: evaluation.contentHash }],
  });
  const dreamRunReadback = await store.readDreamRun!(
    "domain-r2-02-test", "dream-run-r2-02-test", "1",
  );
  if (dreamRun.disposition !== "COMMITTED" ||
      dreamRunReadback.contentHash !== dreamRun.contentHash) {
    throw new Error("R2_TEST_DREAM_RUN_READBACK_MISMATCH");
  }
  const proposal = await store.appendDreamProposal!({
    domainId: "domain-r2-02-test",
    proposalId: "dream-proposal-r2-02-test",
    revision: "1",
    expectedPreviousRevision: null,
    expectedPreviousContentHash: null,
    operationId: "dream-proposal-op-r2-02-test",
    eventId: "dream-proposal-event-r2-02-test",
    receiptId: "dream-proposal-receipt-r2-02-test",
    recordedAt: observedAt,
    sourceIdentity: "deterministic-public-fixture",
    runRef: { objectType: "DreamRun", objectId: dreamRun.objectId,
      revision: dreamRun.revision, contentHash: dreamRun.contentHash },
    candidateKind: "PARAMETER_TUNING",
    beforeHash: rollback.beforeHash,
    afterHash: rollback.afterHash,
    allowedChangeSet: [{ key: "candidate.parameter.temperature",
      beforeHash: rollback.beforeHash, afterHash: rollback.afterHash }],
    heldoutReceipt: null,
    rollbackRef: { objectType: "RollbackPlan", objectId: rollback.planId,
      revision: rollback.revision, contentHash: rollback.contentHash },
    basePolicyRevision: identity.policyRevision,
    namespace: "test/s1-r4/r2-02",
    testOnly: true,
  });
  const proposalReadback = await store.readDreamProposal!(
    "domain-r2-02-test", "dream-proposal-r2-02-test", "1",
  );
  if (proposal.disposition !== "COMMITTED" ||
      proposalReadback.contentHash !== proposal.contentHash) {
    throw new Error("R2_TEST_DREAM_PROPOSAL_READBACK_MISMATCH");
  }
  return Object.freeze({
    state: "VALIDATED_TEST_RESULT_NOT_ADOPTED" as const,
    result,
    actionCompletionRef,
    manifestHash: manifest.manifestHash,
    decisionReceiptId: decision.decisionReceiptId,
    objectiveOutcomeContentHash: outcome.contentHash,
    objectiveOutcomeReceiptId: outcome.receiptId,
    evaluationContentHash: evaluation.contentHash,
    evaluationReceiptId: evaluation.receiptId,
    metricsHash,
    dreamRunContentHash: dreamRun.contentHash,
    dreamProposalContentHash: proposal.contentHash,
    dreamProposalState: "DRAFT_TEST_ONLY_NOT_ACTIVATED" as const,
    ...(fixtureDriverBinding === undefined ? {} : { fixtureDriverBinding: Object.freeze({
      driverId: fixtureDriverBinding.driverId,
      adapterVersion: fixtureDriverBinding.adapterVersion,
      runtimeInstanceId: fixtureDriverBinding.runtimeInstanceId,
      launchDigestSha256: fixtureDriverBinding.launchDigestSha256,
    }) }),
  });
}
