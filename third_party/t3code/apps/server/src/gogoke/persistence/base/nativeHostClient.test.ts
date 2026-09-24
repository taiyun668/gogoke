import * as NodeAssert from "node:assert/strict";
import * as NodeFS from "node:fs";
import * as NodeOS from "node:os";
import * as NodePath from "node:path";
import * as NodeReadline from "node:readline";
import * as NodeStream from "node:stream";
import * as NodeTest from "node:test";

import {
  decodeNativeControlledFixtureAction,
  decodeR2ObjectiveFactRefs,
  decodeR2TestRollbackPlan,
  decodeR2TestFixtureDriverRegistration,
  decodeR2TestFixtureActionBinding,
  decodeCurrentDelegationGrantReply,
  encodeCurrentDelegationGrantFrame,
  decodeExecutionRecipeReceipt,
  encodeExecutionRecipeFrame,
  encodeObjectiveOutcomeFrame,
  encodeEvaluationFrame,
  encodeDreamRunFrame,
  encodeDreamProposalFrame,
  NativeHostClient,
  NativeHostClientError,
  readStartupHandshake,
  type NativeDelegationGrantSnapshot,
} from "./nativeHostClient.ts";

const assert: typeof NodeAssert = NodeAssert;
const test: typeof NodeTest.test = NodeTest.test;

test("novel fixture registration and Action binding keep native identity exact", () => {
  const driverId = "mock_novel_0123456789abcdef";
  const runtimeInstanceId = "runtime-r2-03-0123456789abcdef-fedcba9876543210";
  const registration = { state: "TEST_ONLY_FIXTURE_DRIVER_REGISTERED", driverId,
    adapterVersion: "1.0.0", runtimeInstanceId, contentHash: `sha256:${"a".repeat(64)}` };
  const binding = { state: "TEST_ONLY_ACTION_BINDING", driverId,
    adapterVersion: "1.0.0", runtimeInstanceId, launchDigestSha256: `sha256:${"b".repeat(64)}` };
  assert.deepEqual(decodeR2TestFixtureDriverRegistration(JSON.stringify(registration)), registration);
  assert.deepEqual(decodeR2TestFixtureActionBinding(JSON.stringify(binding)), binding);
  assert.throws(() => decodeR2TestFixtureActionBinding(JSON.stringify({
    ...binding, runtimeInstanceId: "runtime-r2-02-fixture",
  })), /native Action binding mismatch/);
});

test("R2 Objective refs require exact native hashes", () => {
  const valid = {
    state: "TEST_ONLY_NATIVE_OBJECTIVE_REFS",
    manifestHash: `sha256:${"a".repeat(64)}`,
    manifestContentHash: `sha256:${"b".repeat(64)}`,
    decisionContentHash: `sha256:${"c".repeat(64)}`,
    actionCompletionHash: `sha256:${"d".repeat(64)}`,
    actionCompletedAt: "2026-09-23T00:00:01.000Z",
  };
  assert.deepEqual(decodeR2ObjectiveFactRefs(JSON.stringify(valid)), valid);
  assert.throws(() => decodeR2ObjectiveFactRefs(JSON.stringify({
    ...valid, actionCompletionHash: "sha256:caller-assertion",
  })), /native refs identity mismatch/);
});

test("R2 rollback plan reply remains test-only and unactivated", () => {
  const plan = {
    state: "TEST_ONLY_ROLLBACK_PLAN_NOT_ACTIVATED",
    disposition: "COMMITTED",
    domainId: "domain-r2-02-test",
    planId: "rollback-r2-02-test",
    revision: "1",
    contentHash: `sha256:${"a".repeat(64)}`,
    beforeHash: `sha256:${"b".repeat(64)}`,
    afterHash: `sha256:${"c".repeat(64)}`,
  };
  assert.deepEqual(decodeR2TestRollbackPlan(JSON.stringify(plan)), plan);
  assert.throws(() => decodeR2TestRollbackPlan(JSON.stringify({
    ...plan, state: "ACTIVATED",
  })), /native rollback identity mismatch/);
});

test("native Action transport codec keeps completion distinct from Result and replay", () => {
  const actionCompletionRef = "receipt-action-one";
  const stopProofHash = `sha256:${"a".repeat(64)}`;
  const frames = ["ack", "start", "message", "end", "settled"];
  const completed = decodeNativeControlledFixtureAction(JSON.stringify({
    state: "ACTION_TRANSPORT_COMPLETED_NOT_RESULT", frames, actionCompletionRef, stopProofHash,
  }));
  assert.equal(completed.state, "ACTION_TRANSPORT_COMPLETED_NOT_RESULT");
  const reconciled = decodeNativeControlledFixtureAction(JSON.stringify({
    state: "ACTION_COMPLETION_RECONCILED_NOT_RESULT", actionCompletionRef,
  }));
  assert.deepEqual(reconciled, { state: "ACTION_COMPLETION_RECONCILED_NOT_RESULT", actionCompletionRef });
  assert.throws(() => decodeNativeControlledFixtureAction(JSON.stringify({
    state: "ACTION_TRANSPORT_COMPLETED_NOT_RESULT", frames: [], actionCompletionRef, stopProofHash,
  })), NativeHostClientError);
  assert.throws(() => decodeNativeControlledFixtureAction(JSON.stringify({
    state: "ACTION_COMPLETION_RECONCILED_NOT_RESULT", actionCompletionRef, frames,
  })), NativeHostClientError);
});

test("startup handshake retains all three lines emitted in one pipe chunk", async () => {
  const pipe = new NodeStream.PassThrough();
  const lines = NodeReadline.createInterface({ input: pipe });
  try {
    const startup = readStartupHandshake(lines);
    pipe.write("LOCKED\nPIPE\tfixture\nCAPABILITY\t" + "a".repeat(64) + "\n");
    assert.deepEqual(await startup, {
      pipeLine: "PIPE\tfixture",
      capabilityLine: "CAPABILITY\t" + "a".repeat(64),
    });
  } finally {
    lines.close();
    pipe.destroy();
  }
});

test("startup handshake rejects an invalid second line without waiting for a third", async () => {
  const pipe = new NodeStream.PassThrough();
  const lines = NodeReadline.createInterface({ input: pipe });
  try {
    const startup = readStartupHandshake(lines);
    pipe.write("LOCKED\nNOT_A_PIPE\n");
    await assert.rejects(
      startup,
      (error: unknown) => error instanceof NativeHostClientError && error.code === "HOST_PIPE",
    );
  } finally {
    lines.close();
    pipe.destroy();
  }
});

const delegationGrant: NativeDelegationGrantSnapshot = {
  binding: { executionId: "execution-one", generation: "18446744073709551615", sessionId: "session-one" },
  ceiling: {
    allowedActions: ["delegate"],
    allowedContinuationResponses: ["continue"],
    allowedMaterialClasses: ["task-context"],
    allowedSinks: ["task-package"],
    allowedTargetDomainIds: ["domain-one"],
    allowedTargetPrincipalIds: ["worker-one"],
    explicitPrivateMaterialIds: ["private-one"],
    maxMaterialBytes: "4096",
    maxMaterialItems: "8",
    maxResponseBytes: "2048",
  },
  expiresAtEpochMs: "4102444800000",
  grantRef: "grant-one",
  issuerId: "owner-issuer",
  parentGrant: { grantRef: "grant-parent", revision: "2" },
  policyRevision: "3",
  principal: {
    domainId: "domain-one",
    principalId: "controller-one",
    projectId: "project-one",
    role: "controller",
    seatId: "seat-one",
  },
  revision: "4",
  revocationHead: "5",
  seatId: "seat-one",
};

test("current delegation grant frame carries only canonical grantRef", () => {
  assert.deepEqual(JSON.parse(encodeCurrentDelegationGrantFrame("grant-one")), {
    grantRef: "grant-one",
    operation: "ReadCurrentDelegationGrant",
  });
  assert.throws(
    () => encodeCurrentDelegationGrantFrame(" grant-one "),
    (error: unknown) => error instanceof NativeHostClientError,
  );
});

test("Objective and Evaluation frames use independent flat-string record codecs", () => {
  const ref = { objectType: "Result", objectId: "result-one", revision: "1", contentHash: "sha256:" + "a".repeat(64) };
  const objective = JSON.parse(encodeObjectiveOutcomeFrame({ domainId: "domain-one", outcomeId: "outcome-one", revision: "1", expectedPreviousRevision: null, expectedPreviousContentHash: null, operationId: "outcome-op", eventId: "outcome-event", receiptId: "outcome-receipt", recordedAt: "2026-09-22T00:00:00Z", manifestId: "manifest-one", manifestVersion: "1", manifestHash: "sha256:" + "b".repeat(64), decisionId: "decision-one", decisionVersion: "1", decisionHash: "sha256:" + "c".repeat(64), actionOperationId: "action-one", actionCompletionRef: "completion-one", resultRefs: [ref], evidenceRefs: [ref], observationStartsAt: "a", observationEndsAt: "b", observationStatus: "CLOSED" }));
  const evaluation = JSON.parse(encodeEvaluationFrame({ domainId: "domain-one", evaluationId: "evaluation-one", revision: "1", expectedPreviousRevision: null, expectedPreviousContentHash: null, operationId: "evaluation-op", eventId: "evaluation-event", receiptId: "evaluation-receipt", recordedAt: "2026-09-22T00:00:00Z", sourceIdentity: "fixture", outcomeRefs: [ref], decisionFamily: "TEST", scorerVersion: "1", rubricVersion: "1", calibrationKey: "key", calibrationVersion: "1", datasetNamespace: "test", datasetSplit: "split", evidenceRefs: [ref], metricsHash: "sha256:" + "d".repeat(64), safetyStatus: "SAFE", privacyStatus: "SAFE" }));
  assert.equal(objective.operation, "AppendObjectiveOutcome");
  assert.equal(evaluation.operation, "AppendEvaluation");
  assert.match(objective.resultRefs, /^gogoke\.objective-evidence\.v1\|1\|/);
  assert.match(evaluation.outcomeRefs, /^gogoke\.evaluation-outcomes\.v1\|1\|/);
});

test("Dream frames use typed object, evaluation, and change records", () => {
  const ref = { objectType: "ContextManifest", objectId: "manifest-one", revision: "1", contentHash: "sha256:" + "a".repeat(64) };
  const run = JSON.parse(encodeDreamRunFrame({ domainId:"domain-one", runId:"run-one", revision:"1", expectedPreviousRevision:null, expectedPreviousContentHash:null, operationId:"run-op", eventId:"run-event", receiptId:"run-receipt", recordedAt:"2026-09-22T00:00:00Z", sourceIdentity:"fixture", inputSnapshot:ref, datasetNamespace:"test", datasetSplit:"split", datasetSplitHash:"sha256:"+"b".repeat(64), recipeRef:{...ref,objectType:"ExecutionRecipe"}, budgetLease:{leaseRef:"lease",operationId:"decision",resourceRef:"pool",resourceRevision:"1",units:"1"}, evaluationRefs:[{objectType:"EvaluationRecord",objectId:"eval",revision:"1",contentHash:"sha256:"+"c".repeat(64)}] }));
  const proposal = JSON.parse(encodeDreamProposalFrame({ domainId:"domain-one", proposalId:"proposal-one", revision:"1", expectedPreviousRevision:null, expectedPreviousContentHash:null, operationId:"proposal-op", eventId:"proposal-event", receiptId:"proposal-receipt", recordedAt:"2026-09-22T00:00:00Z", sourceIdentity:"fixture", runRef:{...ref,objectType:"DreamRun"}, candidateKind:"PARAMETER_TUNING", beforeHash:"sha256:"+"d".repeat(64), afterHash:"sha256:"+"e".repeat(64), allowedChangeSet:[{key:"candidate.parameter.x",beforeHash:"sha256:"+"d".repeat(64),afterHash:"sha256:"+"e".repeat(64)}], heldoutReceipt:null, rollbackRef:{...ref,objectType:"Rollback"}, basePolicyRevision:"1", namespace:"test/fixture", testOnly:true }));
  assert.match(run.inputSnapshot, /^gogoke\.dream-object\.v1\|1\|/);
  assert.match(run.evaluationRefs, /^gogoke\.dream-evaluations\.v1\|1\|/);
  assert.equal(run.budgetLeaseRef, "lease");
  assert.equal(run.budgetOperationId, "decision");
  assert.match(proposal.allowedChangeSet, /^gogoke\.dream-changes\.v1\|1\|/);
  assert.equal(proposal.testOnly, "true");
});

test("ExecutionRecipe uses flat string request fields and exact typed reply", () => {
  const frame = JSON.parse(encodeExecutionRecipeFrame({
    operationId: "recipe-op", domainId: "domain-one", expectedPreviousRevision: null,
    recipeId: "recipe-one", seatId: "seat-one", runtimeInstanceId: "runtime-one",
    modelRef: { model: "fixture" }, toolProfile: null, isolationProfile: { mode: "private" },
    contextManifestId: "manifest-one", budgetPolicy: { units: 1 }, admissionRef: "grant-one",
    eventId: "recipe-event", receiptId: "recipe-receipt", recordedAt: "2026-09-22T00:00:00Z",
  }));
  assert.equal(frame.operation, "AppendExecutionRecipe");
  assert.equal(typeof frame.modelRef, "string");
  assert.equal(typeof frame.toolProfile, "string");
  const receipt = decodeExecutionRecipeReceipt(JSON.stringify({
    disposition: "COMMITTED", operationId: "recipe-op", currentnessStatus: "PREPARATORY_CURRENTNESS_REQUIRED",
    recipe: { contentHash: "sha256:" + "a".repeat(64), domainId: "domain-one", recipeId: "recipe-one", revision: "1", seatId: "seat-one", runtimeInstanceId: "runtime-one", contextManifestId: "manifest-one", admissionRef: "grant-one" },
  }));
  assert.equal(receipt.recipe.recipeId, "recipe-one");
  assert.equal(Object.isFrozen(receipt.recipe), true);
});

test("current delegation reply codec returns an exact immutable typed snapshot", () => {
  const decoded = decodeCurrentDelegationGrantReply(JSON.stringify(delegationGrant), "grant-one");
  assert.deepEqual(decoded, delegationGrant);
  assert.equal(Object.isFrozen(decoded), true);
  assert.equal(Object.isFrozen(decoded.principal), true);
  assert.equal(Object.isFrozen(decoded.binding), true);
  assert.equal(Object.isFrozen(decoded.ceiling), true);
  assert.equal(Object.isFrozen(decoded.ceiling.allowedActions), true);
  assert.throws(
    () => decodeCurrentDelegationGrantReply(JSON.stringify({ ...delegationGrant, extra: "x" }), "grant-one"),
    (error: unknown) => error instanceof NativeHostClientError && error.code === "DELEGATION_REPLY",
  );
  assert.throws(
    () => decodeCurrentDelegationGrantReply(JSON.stringify(delegationGrant), "grant-other"),
    (error: unknown) => error instanceof NativeHostClientError,
  );
  const canonical = JSON.stringify(delegationGrant);
  assert.throws(
    () => decodeCurrentDelegationGrantReply(
      canonical.replace('"grantRef":"grant-one"', '"grantRef":"grant-one","grantRef":"grant-one"'),
      "grant-one",
    ),
    (error: unknown) => error instanceof NativeHostClientError && error.code === "DELEGATION_REPLY",
  );
  assert.throws(
    () => decodeCurrentDelegationGrantReply(
      canonical.replace('"seatId":"seat-one"', '"seatId":"seat-one","seatId":"seat-one"'),
      "grant-one",
    ),
    (error: unknown) => error instanceof NativeHostClientError && error.code === "DELEGATION_REPLY",
  );
  assert.throws(
    () => decodeCurrentDelegationGrantReply(
      JSON.stringify({ ...delegationGrant, revision: 4 }),
      "grant-one",
    ),
    (error: unknown) => error instanceof NativeHostClientError,
  );
  assert.throws(
    () => decodeCurrentDelegationGrantReply(
      JSON.stringify({ ...delegationGrant, principal: { ...delegationGrant.principal, seatId: "other-seat" } }),
      "grant-one",
    ),
    (error: unknown) => error instanceof NativeHostClientError,
  );
});

const requiredHostBinary = (): string => {
  const value = process.env.GOGOKE_NATIVE_HOST;
  if (typeof value !== "string") {
    assert.fail("GOGOKE_NATIVE_HOST must bind the exact native-host under test");
  }
  if (value.length === 0) {
    assert.fail("GOGOKE_NATIVE_HOST must not be empty");
  }
  return value;
};

test("Node typed client commits through native-host without opening SQLite", async () => {
  const hostBinary = requiredHostBinary();
  assert.equal(NodeFS.existsSync(hostBinary), true, `native-host missing: ${hostBinary}`);
  const root = NodeFS.mkdtempSync(NodePath.join(NodeOS.tmpdir(), "gogoke-node-host-"));
  const client = await NativeHostClient.attach({ root, hostBinary });
  try {
    const committed = await client.commitProject({
      commandId: "cmd-project",
      projectId: "proj-1",
      title: "one",
      workspaceRoot: "C:/tmp/one",
      occurredAt: "2026-09-20T00:00:00Z",
    });
    assert.equal(committed.ok, true);
    assert.match(committed.body, /COMMITTED/);
    assert.equal(committed.elapsedMicros > 0, true);
    const snapshot = await client.readSnapshot(10);
    assert.match(snapshot.body, /"count":1/);
    const receipt = await client.getReceipt("cmd-project");
    assert.match(receipt.body, /"found":true/);
  } finally {
    await client.close();
    NodeFS.rmSync(root, { recursive: true, force: true });
  }
});

test("SQL frames are refused before a product database path is ever named", async () => {
  const hostBinary = requiredHostBinary();
  assert.equal(NodeFS.existsSync(hostBinary), true, `native-host missing: ${hostBinary}`);
  const root = NodeFS.mkdtempSync(NodePath.join(NodeOS.tmpdir(), "gogoke-node-sql-"));
  const client = await NativeHostClient.attach({ root, hostBinary });
  try {
    await assert.rejects(
      client.sendRawForTest(JSON.stringify({ operation: "execute", sql: "SELECT 1" })),
      (error: unknown) => error instanceof NativeHostClientError,
    );
  } finally {
    await client.close();
    NodeFS.rmSync(root, { recursive: true, force: true });
  }
});
