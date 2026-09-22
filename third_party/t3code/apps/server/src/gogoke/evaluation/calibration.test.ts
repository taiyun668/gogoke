import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import {
  calibrationReport,
  type CalibrationKey,
  type CalibrationRequest,
  type OutcomeObservation,
} from "./calibration.ts";
import { EvaluationInputError } from "./boundary.ts";

const key = (): CalibrationKey => ({
  decisionFamily: "RESOURCE_SELECTION",
  modelVersion: "fixture-v1",
  questionVersion: "1",
  viewVersion: "1",
  criteriaVersion: "1",
  candidateGeneratorVersion: "1",
  candidateSetHash: "set-one",
  locale: "zh-CN",
  taskDomain: "fixture-task",
  runtimeEnvironment: "fixture-runtime",
});
const row = (id = "one", changes: Partial<OutcomeObservation> = {}): OutcomeObservation => ({
  member: {
    outcomeId: id,
    domainId: "domain-one",
    namespace: "SYNTHETIC",
    split: "CALIBRATION",
    projectId: `project-${id}`,
    timeGroup: `time-${id}`,
    sessionLineageId: `lineage-${id}`,
    nearDuplicateClusterId: `cluster-${id}`,
  },
  revision: "1",
  supersedesRevision: null,
  key: key(),
  probabilities: [
    { candidateId: "a", probability: 0.9 },
    { candidateId: "b", probability: 0.1 },
  ],
  nativeConfidence: 0.99,
  status: "OBSERVED",
  labelSource: "OBJECTIVE",
  observedCandidateId: "a",
  evidenceRefs: ["evidence-one"],
  contextCoverage: 0.8,
  candidateCoverage: 0.7,
  freshness: 0.6,
  scorerReliability: null,
  ood: "UNKNOWN",
  safetyPrivacy: "UNKNOWN",
  ...changes,
});
const request = (rows: readonly OutcomeObservation[]): CalibrationRequest => ({
  domainId: "domain-one",
  namespace: "SYNTHETIC",
  key: key(),
  binCount: 10,
  observations: rows,
});
const close = (actual: number | null, expected: number) => {
  Assert.notEqual(actual, null);
  Assert.ok(Math.abs(actual! - expected) < 1e-12);
};
const rejects = (fn: () => unknown) =>
  Assert.throws(
    fn,
    (e: unknown) => e instanceof EvaluationInputError && e.message === "EVALUATION_INPUT_REJECTED",
  );

describe("calibration reports preserve evidence dimensions and never confer authority", () => {
  it("EC01 no feedback is no metric rather than success or failure", () => {
    const report = calibrationReport(request([]));
    Assert.equal(report.outcomes, 0);
    Assert.equal(report.byLabelSource.OBJECTIVE.brier, null);
    Assert.equal(report.byLabelSource.OBJECTIVE.ece, null);
    Assert.equal(report.automaticUse, "NOT_AUTHORIZED");
  });
  it("EC02 binary probability loss and auxiliary calibration error use explicit definitions", () => {
    const report = calibrationReport(request([row()]));
    close(report.byLabelSource.OBJECTIVE.brier, 0.02);
    close(report.byLabelSource.OBJECTIVE.ece, 0.1);
    Assert.equal(
      report.byLabelSource.OBJECTIVE.brierDefinition,
      "MEAN_SUM_OVER_CLASSES_SQUARED_ERROR",
    );
    Assert.equal(report.byLabelSource.OBJECTIVE.bins[9]?.count, 1);
  });
  it("EC03 native confidence does not replace or rescale a measured probability loss", () => {
    const low = calibrationReport(request([row("one", { nativeConfidence: 0 })]));
    const high = calibrationReport(request([row("one", { nativeConfidence: 1 })]));
    Assert.equal(low.byLabelSource.OBJECTIVE.brier, high.byLabelSource.OBJECTIVE.brier);
    Assert.equal(low.byLabelSource.OBJECTIVE.ece, high.byLabelSource.OBJECTIVE.ece);
    Assert.equal(high.evidenceDimensions[0]?.nativeConfidence, 1);
    Assert.equal(high.universalScore, null);
  });
  it("EC04 multiclass loss keeps all class terms without multiplying question confidences", () => {
    const r = row("one", {
      probabilities: [
        { candidateId: "a", probability: 0.2 },
        { candidateId: "b", probability: 0.3 },
        { candidateId: "c", probability: 0.5 },
      ],
      observedCandidateId: "b",
    });
    const report = calibrationReport(request([r]));
    close(report.byLabelSource.OBJECTIVE.brier, 0.78);
    close(report.byLabelSource.OBJECTIVE.ece, 0.5);
  });
  it("EC05 objective semantic override and self-report feedback are never silently pooled", () => {
    const report = calibrationReport(
      request([
        row("objective"),
        row("semantic", { labelSource: "INDEPENDENT_SEMANTIC", observedCandidateId: "b" }),
        row("owner", { labelSource: "OWNER_OVERRIDE" }),
        row("self", { labelSource: "SELF_REPORT" }),
      ]),
    );
    Assert.equal(report.byLabelSource.OBJECTIVE.observations, 1);
    close(report.byLabelSource.OBJECTIVE.brier, 0.02);
    Assert.equal(report.byLabelSource.INDEPENDENT_SEMANTIC.observations, 1);
    close(report.byLabelSource.INDEPENDENT_SEMANTIC.brier, 1.62);
    Assert.equal(report.byLabelSource.OWNER_OVERRIDE.observations, 1);
    Assert.equal(report.counts.selfReportExcluded, 1);
  });
  it("EC06 pending censored and infrastructure outcomes do not enter a probability denominator", () => {
    const missing = (["PENDING", "CENSORED", "INFRA_FAILURE"] as const).map((status) =>
      row(status, { status, labelSource: null, observedCandidateId: null, evidenceRefs: [] }),
    );
    const report = calibrationReport(request([row(), ...missing]));
    Assert.equal(report.byLabelSource.OBJECTIVE.observations, 1);
    close(report.byLabelSource.OBJECTIVE.brier, 0.02);
    Assert.equal(report.counts.observed, 1);
    Assert.equal(report.outcomes, 4);
    Assert.equal(report.counts.pending, 1);
    Assert.equal(report.counts.censored, 1);
    Assert.equal(report.counts.infrastructure, 1);
  });
  it("EC07 late failure is a new revision and replaces the metric contribution without erasing history", () => {
    const first = row();
    const correction = row("one", {
      revision: "2",
      supersedesRevision: "1",
      observedCandidateId: "b",
      evidenceRefs: ["review-later"],
    });
    const report = calibrationReport(request([correction, first]));
    Assert.equal(report.historyRecords, 2);
    Assert.equal(report.outcomes, 1);
    Assert.equal(report.correctionRecords, 1);
    close(report.byLabelSource.OBJECTIVE.brier, 1.62);
    Assert.equal(first.observedCandidateId, "a");
    Assert.equal(report.evidenceDimensions[0]?.revision, "2");
  });
  it("EC08 gaps duplicate revisions and wrong predecessor pointers are rejected", () => {
    const first = row();
    for (const rows of [
      [first, first],
      [row("one", { revision: "2", supersedesRevision: "1" })],
      [first, row("one", { revision: "3", supersedesRevision: "1" })],
      [first, row("one", { revision: "2", supersedesRevision: null })],
    ]) {
      rejects(() => calibrationReport(request(rows)));
    }
  });
  it("EC09 label correction cannot rewrite the original prediction, key, or evaluation metadata", () => {
    const first = row();
    const memberFields = [
      "projectId",
      "timeGroup",
      "sessionLineageId",
      "nearDuplicateClusterId",
    ] as const;
    const changes: Partial<OutcomeObservation>[] = [
      { nativeConfidence: 0.2 },
      {
        probabilities: [
          { candidateId: "a", probability: 0.1 },
          { candidateId: "b", probability: 0.9 },
        ],
      },
      { key: { ...key(), questionVersion: "2" } },
      ...memberFields.map((field) => ({
        member: { ...first.member, [field]: `changed-${field}` },
      })),
      { contextCoverage: 0.2 },
      { candidateCoverage: 0.2 },
      { freshness: 0.2 },
      { scorerReliability: 0.2 },
      { ood: "IN_DISTRIBUTION" },
      { safetyPrivacy: "CLEAR" },
    ];
    for (const change of changes) {
      const next = row("one", { revision: "2", supersedesRevision: "1", ...change });
      rejects(() => calibrationReport(request([first, next])));
    }
  });
  it("EC10 cross-domain records fail without revealing their identities in errors", () => {
    const r = row();
    const foreign = { ...r, member: { ...r.member, domainId: "private-foreign-domain" } };
    rejects(() => calibrationReport(request([foreign])));
  });
  it("EC11 real and synthetic namespaces cannot share a report", () => {
    const r = row();
    rejects(() =>
      calibrationReport(request([{ ...r, member: { ...r.member, namespace: "REAL" } }])),
    );
  });
  it("EC12 sealed holdout and development rows cannot enter an optimizer-visible calibration report", () => {
    for (const split of ["SEALED_HOLDOUT", "DEVELOPMENT"] as const) {
      const r = row();
      rejects(() => calibrationReport(request([{ ...r, member: { ...r.member, split } }])));
    }
  });
  it("EC13 each model question view criteria candidate language task and runtime key axis is exact", () => {
    for (const field of Object.keys(key()) as Array<keyof CalibrationKey>) {
      const r = row("one", { key: { ...key(), [field]: "changed" } });
      rejects(() => calibrationReport(request([r])));
    }
  });
  it("EC14 malformed probability distributions are rejected rather than normalized silently", () => {
    const distributions = [
      [],
      [{ candidateId: "a", probability: 1 }],
      [
        { candidateId: "a", probability: 0.5 },
        { candidateId: "a", probability: 0.5 },
      ],
      [
        { candidateId: "a", probability: 0.8 },
        { candidateId: "b", probability: 0.1 },
      ],
      [
        { candidateId: "a", probability: NaN },
        { candidateId: "b", probability: 0.1 },
      ],
      [
        { candidateId: "a", probability: Infinity },
        { candidateId: "b", probability: 0.1 },
      ],
      [
        { candidateId: "a", probability: -0.1 },
        { candidateId: "b", probability: 1.1 },
      ],
    ];
    for (const probabilities of distributions)
      rejects(() => calibrationReport(request([row("one", { probabilities })])));
  });
  it("EC15 observed labels require a valid class source and nonempty evidence references", () => {
    for (const changes of [
      { observedCandidateId: "not-in-candidates" },
      { observedCandidateId: null },
      { labelSource: null },
      { evidenceRefs: [] },
      { evidenceRefs: ["same", "same"] },
    ]) {
      rejects(() => calibrationReport(request([row("one", changes)])));
    }
  });
  it("EC16 non-observed outcomes cannot carry a hidden success label", () => {
    for (const status of ["PENDING", "CENSORED", "INFRA_FAILURE"] as const) {
      const withHiddenSuccess = row("one", { status });
      rejects(() => calibrationReport(request([withHiddenSuccess])));
    }
  });
  it("EC17 confidence and evidence dimensions validate finite ranges without guessing defaults", () => {
    for (const field of [
      "nativeConfidence",
      "contextCoverage",
      "candidateCoverage",
      "freshness",
      "scorerReliability",
    ] as const) {
      for (const value of [-0.1, 1.1, NaN, Infinity])
        rejects(() => calibrationReport(request([row("one", { [field]: value })])));
    }
    const r = calibrationReport(
      request([row("one", { contextCoverage: null, candidateCoverage: null, freshness: null })]),
    );
    Assert.equal(r.evidenceDimensions[0]?.contextCoverage, null);
    Assert.equal(r.automaticUse, "NOT_AUTHORIZED");
  });
  it("EC18 active records and distributions are rejected without evaluating getters or proxy traps", () => {
    let reads = 0;
    const r = row();
    Object.defineProperty(r, "status", {
      enumerable: true,
      get() {
        reads++;
        return "OBSERVED";
      },
    });
    rejects(() => calibrationReport(request([r])));
    const proxy = new Proxy(row(), {
      get(target, key, receiver) {
        reads++;
        return Reflect.get(target, key, receiver);
      },
    });
    rejects(() => calibrationReport(request([proxy])));
    const r2 = row();
    Object.defineProperty(r2.probabilities, Symbol.iterator, {
      value: function* () {
        reads++;
      },
    });
    rejects(() => calibrationReport(request([r2])));
    Assert.equal(reads, 0);
  });
  it("EC19 many self-reports and high confidence cannot establish qualification or a correctness metric", () => {
    const report = calibrationReport(
      request(
        Array.from({ length: 200 }, (_, i) =>
          row(String(i), { labelSource: "SELF_REPORT", nativeConfidence: 1 }),
        ),
      ),
    );
    Assert.equal(report.counts.selfReportExcluded, 200);
    Assert.equal(report.byLabelSource.OBJECTIVE.observations, 0);
    Assert.equal(report.byLabelSource.INDEPENDENT_SEMANTIC.brier, null);
    Assert.equal(report.qualification, false);
    Assert.equal(report.automaticUse, "NOT_AUTHORIZED");
  });
  it("EC20 incident and OOD axes remain separate from numerical calibration quality", () => {
    const report = calibrationReport(
      request([row("one", { safetyPrivacy: "INCIDENT", ood: "OUT_OF_DISTRIBUTION" })]),
    );
    close(report.byLabelSource.OBJECTIVE.brier, 0.02);
    Assert.equal(report.counts.safetyPrivacyIncidents, 1);
    Assert.equal(report.counts.oodOrUnknown, 1);
    Assert.equal(report.universalScore, null);
    Assert.equal(report.automaticUse, "NOT_AUTHORIZED");
    Assert.equal(report.counterfactualOptimalityEstablished, false);
  });
  it("EC21 repeated project observations do not manufacture independent samples or intervals", () => {
    const records = Array.from({ length: 20 }, (_, i) => {
      const r = row(String(i));
      return { ...r, member: { ...r.member, projectId: "one-project" } };
    });
    const report = calibrationReport(request(records));
    Assert.equal(report.outcomes, 20);
    Assert.equal(report.knownConnectedGroups, 1);
    Assert.equal(report.independentSampleSize, null);
    Assert.equal(report.confidenceInterval, null);
  });
  it("EC22 bins are bounded and probability one stays in the final bin", () => {
    for (const binCount of [0, 101, 1.5, NaN])
      rejects(() => calibrationReport({ ...request([row()]), binCount }));
    const report = calibrationReport(
      request([
        row("one", {
          probabilities: [
            { candidateId: "a", probability: 1 },
            { candidateId: "b", probability: 0 },
          ],
        }),
      ]),
    );
    close(report.byLabelSource.OBJECTIVE.brier, 0);
    close(report.byLabelSource.OBJECTIVE.ece, 0);
    Assert.equal(report.byLabelSource.OBJECTIVE.bins[9]?.count, 1);
  });
  it("EC23 output is detached and frozen rather than a mutable authority snapshot", () => {
    const r = row();
    const report = calibrationReport(request([r]));
    (r.probabilities[0] as { probability: number }).probability = 0.1;
    Assert.equal(report.evidenceDimensions[0]?.probabilities[0]?.probability, 0.9);
    Assert.ok(Object.isFrozen(report));
    Assert.ok(Object.isFrozen(report.byLabelSource.OBJECTIVE.bins));
    Assert.ok(Object.isFrozen(report.evidenceDimensions[0]?.probabilities[0]));
  });
  it("EC24 revision syntax and candidate-set identity cannot drift inside a calibration key", () => {
    for (const revision of ["0", "01", "18446744073709551616"])
      rejects(() => calibrationReport(request([row("one", { revision })])));
    const second = row("two", {
      probabilities: [
        { candidateId: "a", probability: 0.9 },
        { candidateId: "c", probability: 0.1 },
      ],
    });
    rejects(() => calibrationReport(request([row(), second])));
  });
});
