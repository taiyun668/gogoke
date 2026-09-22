import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import {
  createDreamCandidate,
  createDreamRun,
  dreamEligibility,
  transitionDreamCandidate,
} from "./dream.ts";

const hash = (c: string) => "sha256:" + c.repeat(64);
const key = {
  decisionFamily: "RESOURCE_SELECTION",
  modelVersion: "fake-v1",
  questionVersion: "1",
  viewVersion: "1",
  criteriaVersion: "1",
  candidateGeneratorVersion: "1",
  candidateSetHash: hash("c"),
  locale: "zh-CN",
  taskDomain: "fixture",
  runtimeEnvironment: "node22",
} as const;
const facts = (overrides: Record<string, unknown> = {}) => ({
  domainId: "domain-one",
  namespace: "REAL" as const,
  foregroundCritical: false,
  urgentResumePending: false,
  dangerousCustodyPending: false,
  budgetAuthorized: true,
  maintenanceWindowOpen: true,
  dataAuthorized: true,
  idleMillis: 120_000,
  minIdleMillis: 120_000,
  recentSamples: 10,
  minRecentSamples: 10,
  externalModelBudget: 0,
  maxSteps: 20,
  maxDurationMs: 300_000,
  preemptDeadlineMs: 2_000,
  budgetLease: { units: 1 },
  snapshotHash: hash("a"),
  datasetSplitHash: hash("b"),
  recipeRef: "dream-fixture",
  ...overrides,
});
const candidate = (namespace: "REAL" | "SYNTHETIC" = "REAL") =>
  createDreamCandidate({
    namespace,
    proposalId: "proposal-one",
    runId: "run-one",
    kind: "QUESTION_SET",
    basePolicyRevision: "1",
    beforeHash: hash("a"),
    afterHash: hash("b"),
    evaluationRefs: ["eval-one"],
    rollbackRef: "policy-one",
    calibrationKey: key,
    calibrationAutomaticUse: "NOT_AUTHORIZED",
    safetyPrivacyIncidents: 0,
    safetyPrivacyUnknown: 0,
    heldoutReceipt: null,
  });

describe("Dream candidate-only controls", () => {
  it("uses the existing scheduler maintenance kind with one-concurrency limits", () => {
    const e = dreamEligibility(facts());
    Assert.equal(e.kind, "READY");
    if (e.kind === "READY") {
      Assert.equal(e.task.kind, "maintenance");
      Assert.equal(e.task.concurrency, 1);
      Assert.equal(e.task.maxSteps, 20);
      Assert.equal(e.task.maxDurationMs, 300_000);
    }
  });
  it("foreground, custody, missing permission and insufficient idle/sample all wait", () => {
    for (const [key, value, reason] of [
      ["foregroundCritical", true, "FOREGROUND"],
      ["dangerousCustodyPending", true, "DANGEROUS_CUSTODY"],
      ["dataAuthorized", false, "DATA_DENIED"],
      ["idleMillis", 119_999, "NOT_IDLE"],
      ["recentSamples", 9, "INSUFFICIENT_SAMPLES"],
    ] as const) {
      const e = dreamEligibility(facts({ [key]: value }));
      Assert.deepEqual(e, { kind: "WAIT", reason });
    }
  });
  it("real namespace refuses nonzero external model budget in S1", () => {
    Assert.deepEqual(dreamEligibility(facts({ externalModelBudget: 1 })), {
      kind: "WAIT",
      reason: "REAL_EXTERNAL_BUDGET_DISABLED",
    });
  });
  it("creates an immutable run and declarative candidate without activation authority", () => {
    const e = dreamEligibility(facts());
    Assert.equal(e.kind, "READY");
    if (e.kind !== "READY") return;
    const run = createDreamRun({ runId: "run-one", task: e.task });
    const p = candidate();
    Assert.equal(run.runId, "run-one");
    Assert.equal(p.state, "DRAFT");
    Assert.equal(p.activationGrant, null);
    Assert.equal(Object.isFrozen(p), true);
    Assert.equal(Object.isFrozen(p.allowedChangeSet), true);
    Assert.deepEqual(p.allowedChangeSet, {
      kind: "QUESTION_SET",
      beforeHash: hash("a"),
      afterHash: hash("b"),
      testOnly: false,
    });
    // This proposal shape has no field for facts, permissions, or test-source edits.
  });
  it("binds transitions to process-local creator identity and the exact original change set", () => {
    const real = candidate();
    const transition = (proposal: typeof real, namespace: "REAL" | "SYNTHETIC" = "REAL") =>
      transitionDreamCandidate({
        namespace,
        proposal,
        next: "DEV_VALIDATED",
        evidenceRef: "dev",
      });

    const successor = transition(real);
    Assert.equal(Object.isFrozen(successor), true);
    Assert.equal(Object.isFrozen(successor.allowedChangeSet), true);
    Assert.equal(successor.allowedChangeSet, real.allowedChangeSet);
    Assert.deepEqual(successor.allowedChangeSet, {
      kind: "QUESTION_SET",
      beforeHash: hash("a"),
      afterHash: hash("b"),
      testOnly: false,
    });

    Assert.throws(() => transition({ ...real }));
    Assert.throws(() => transition(JSON.parse(JSON.stringify(real)) as typeof real));
    Assert.throws(() => transition(real, "SYNTHETIC"));

    const launderingCopy: typeof real = {
      ...real,
      allowedChangeSet: {
        ...(real.allowedChangeSet as Record<string, unknown>),
        testOnly: true,
      },
    };
    Assert.throws(() => transition(launderingCopy, "SYNTHETIC"));

    const extraFactCopy: typeof real = {
      ...real,
      allowedChangeSet: {
        ...(real.allowedChangeSet as Record<string, unknown>),
        sourceNamespace: "REAL",
      },
    };
    Assert.throws(() => transition(extraFactCopy));

    const synthetic = candidate("SYNTHETIC");
    Assert.throws(() => transition(synthetic, "REAL"));
  });
  it("real candidates stop before SHADOW while synthetic test-only candidates can demonstrate activation states", () => {
    let p = candidate();
    p = transitionDreamCandidate({
      namespace: "REAL",
      proposal: p,
      next: "DEV_VALIDATED",
      evidenceRef: "dev",
    });
    p = transitionDreamCandidate({
      namespace: "REAL",
      proposal: p,
      next: "CALIBRATED",
      evidenceRef: "cal",
    });
    p = transitionDreamCandidate({
      namespace: "REAL",
      proposal: p,
      next: "HOLDOUT_VALIDATED",
      evidenceRef: "hold",
      heldoutReceipt: "heldout-one",
    });
    const rejectRealActivation = (proposal: typeof p, next: "SHADOW" | "CANARY" | "ACTIVE") =>
      Assert.throws(() =>
        transitionDreamCandidate({
          namespace: "REAL",
          proposal,
          next,
          evidenceRef: `real-${next}`,
          activationGrant: { attempted: true },
        }),
      );
    rejectRealActivation(p, "SHADOW");

    let test = candidate("SYNTHETIC");
    const assertTestOnly = () =>
      Assert.equal((test.allowedChangeSet as Record<string, unknown>).testOnly, true);
    assertTestOnly();
    test = transitionDreamCandidate({
      namespace: "SYNTHETIC",
      proposal: test,
      next: "DEV_VALIDATED",
      evidenceRef: "d",
    });
    assertTestOnly();
    test = transitionDreamCandidate({
      namespace: "SYNTHETIC",
      proposal: test,
      next: "CALIBRATED",
      evidenceRef: "c",
    });
    assertTestOnly();
    // heldoutReceipt is opaque string data here; this does not establish issuance or authenticity.
    test = transitionDreamCandidate({
      namespace: "SYNTHETIC",
      proposal: test,
      next: "HOLDOUT_VALIDATED",
      evidenceRef: "h",
      heldoutReceipt: "heldout-fixture",
    });
    assertTestOnly();
    test = transitionDreamCandidate({
      namespace: "SYNTHETIC",
      proposal: test,
      next: "SHADOW",
      evidenceRef: "s",
      activationGrant: { fixture: true },
    });
    assertTestOnly();
    rejectRealActivation(test, "CANARY");
    const testOnlyRemoved: typeof test = {
      ...test,
      allowedChangeSet: {
        ...(test.allowedChangeSet as Record<string, unknown>),
        testOnly: false,
      },
    };
    Assert.throws(() =>
      transitionDreamCandidate({
        namespace: "SYNTHETIC",
        proposal: testOnlyRemoved,
        next: "CANARY",
        evidenceRef: "n",
        activationGrant: { fixture: true },
      }),
    );
    test = transitionDreamCandidate({
      namespace: "SYNTHETIC",
      proposal: test,
      next: "CANARY",
      evidenceRef: "n",
      activationGrant: { fixture: true },
    });
    assertTestOnly();
    rejectRealActivation(test, "ACTIVE");
    test = transitionDreamCandidate({
      namespace: "SYNTHETIC",
      proposal: test,
      next: "ACTIVE",
      evidenceRef: "a",
      activationGrant: { fixture: true },
    });
    Assert.equal(test.state, "ACTIVE");
    assertTestOnly();
  });
  it("safety/privacy unknown or incident cannot be averaged away into a proposal", () => {
    const input = {
      namespace: "REAL" as const,
      proposalId: "p",
      runId: "r",
      kind: "THRESHOLD" as const,
      basePolicyRevision: "1",
      beforeHash: hash("a"),
      afterHash: hash("b"),
      evaluationRefs: ["e"],
      rollbackRef: "r",
      calibrationKey: key,
      calibrationAutomaticUse: "NOT_AUTHORIZED" as const,
      safetyPrivacyIncidents: 0,
      safetyPrivacyUnknown: 0,
      heldoutReceipt: null,
    };
    Assert.throws(() => createDreamCandidate({ ...input, safetyPrivacyIncidents: 1 }));
    Assert.throws(() => createDreamCandidate({ ...input, safetyPrivacyUnknown: 1 }));
  });
});
