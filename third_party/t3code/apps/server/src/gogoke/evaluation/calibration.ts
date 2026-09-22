import { array, choice, frame, integer, optionalRatio, ratio, record, reject, revision, text } from "./boundary.ts";
import { auditDatasetPartitions, memberSnapshot, type DatasetMember, type DataNamespace } from "./splits.ts";

const KEY_FIELDS = Object.freeze(["decisionFamily", "modelVersion", "questionVersion", "viewVersion", "criteriaVersion",
  "candidateGeneratorVersion", "candidateSetHash", "locale", "taskDomain", "runtimeEnvironment"] as const);
export type CalibrationKey = Readonly<Record<typeof KEY_FIELDS[number], string>>;
export type LabelSource = "OBJECTIVE" | "INDEPENDENT_SEMANTIC" | "OWNER_OVERRIDE" | "SELF_REPORT";
export interface OutcomeObservation {
  readonly member: DatasetMember;
  readonly revision: string;
  readonly supersedesRevision: string | null;
  readonly key: CalibrationKey;
  readonly probabilities: readonly { readonly candidateId: string; readonly probability: number }[];
  readonly nativeConfidence: number | null;
  readonly status: "OBSERVED" | "PENDING" | "CENSORED" | "INFRA_FAILURE";
  readonly labelSource: LabelSource | null;
  readonly observedCandidateId: string | null;
  readonly evidenceRefs: readonly string[];
  readonly contextCoverage: number | null;
  readonly candidateCoverage: number | null;
  readonly freshness: number | null;
  readonly scorerReliability: number | null;
  readonly ood: "IN_DISTRIBUTION" | "OUT_OF_DISTRIBUTION" | "UNKNOWN";
  readonly safetyPrivacy: "CLEAR" | "INCIDENT" | "UNKNOWN";
}
export interface CalibrationRequest {
  readonly domainId: string;
  readonly namespace: DataNamespace;
  readonly key: CalibrationKey;
  readonly binCount: number;
  readonly observations: readonly OutcomeObservation[];
}
function keySnapshot(value: unknown): CalibrationKey {
  const raw = record(value, KEY_FIELDS);
  const out: Record<string, string> = Object.create(null);
  for (const key of KEY_FIELDS) out[key] = text(raw[key]);
  // Every key and value was validated above, not asserted from caller JSON.
  return Object.freeze(out) as CalibrationKey;
}
const keyId = (key: CalibrationKey) => frame(KEY_FIELDS.map(field => key[field]));
function observationSnapshot(value: unknown): OutcomeObservation {
  const raw = record(value, ["member", "revision", "supersedesRevision", "key", "probabilities", "nativeConfidence",
    "status", "labelSource", "observedCandidateId", "evidenceRefs", "contextCoverage", "candidateCoverage", "freshness",
    "scorerReliability", "ood", "safetyPrivacy"]);
  const probabilities = array(raw.probabilities).map(value => {
    const p = record(value, ["candidateId", "probability"]);
    return Object.freeze({ candidateId: text(p.candidateId), probability: ratio(p.probability) });
  });
  if (probabilities.length < 2 || new Set(probabilities.map(p => p.candidateId)).size !== probabilities.length ||
      Math.abs(probabilities.reduce((sum, p) => sum + p.probability, 0) - 1) > 1e-9) return reject();
  const evidenceRefs = array(raw.evidenceRefs).map(text);
  if (new Set(evidenceRefs).size !== evidenceRefs.length) return reject();
  const status = choice(raw.status, ["OBSERVED", "PENDING", "CENSORED", "INFRA_FAILURE"]);
  const labelSource = raw.labelSource === null ? null : choice(raw.labelSource, ["OBJECTIVE", "INDEPENDENT_SEMANTIC", "OWNER_OVERRIDE", "SELF_REPORT"]);
  const observedCandidateId = raw.observedCandidateId === null ? null : text(raw.observedCandidateId);
  if (status === "OBSERVED") {
    if (labelSource === null || observedCandidateId === null || evidenceRefs.length === 0 ||
        !probabilities.some(p => p.candidateId === observedCandidateId)) return reject();
  } else if (observedCandidateId !== null || labelSource !== null) return reject();
  return Object.freeze({ member: memberSnapshot(raw.member), revision: revision(raw.revision),
    supersedesRevision: raw.supersedesRevision === null ? null : revision(raw.supersedesRevision),
    key: keySnapshot(raw.key), probabilities: Object.freeze(probabilities), nativeConfidence: optionalRatio(raw.nativeConfidence),
    status, labelSource, observedCandidateId, evidenceRefs: Object.freeze(evidenceRefs),
    contextCoverage: optionalRatio(raw.contextCoverage), candidateCoverage: optionalRatio(raw.candidateCoverage),
    freshness: optionalRatio(raw.freshness), scorerReliability: optionalRatio(raw.scorerReliability),
    ood: choice(raw.ood, ["IN_DISTRIBUTION", "OUT_OF_DISTRIBUTION", "UNKNOWN"]),
    safetyPrivacy: choice(raw.safetyPrivacy, ["CLEAR", "INCIDENT", "UNKNOWN"]) });
}
function immutableObservationId(row: OutcomeObservation): string {
  const m = row.member;
  return frame([m.outcomeId, m.domainId, m.namespace, m.split, m.projectId, m.timeGroup,
    m.sessionLineageId, m.nearDuplicateClusterId, keyId(row.key), row.nativeConfidence,
    ...row.probabilities.flatMap(p => [p.candidateId, p.probability]),
    row.contextCoverage, row.candidateCoverage, row.freshness, row.scorerReliability,
    row.ood, row.safetyPrivacy]);
}
function latest(rows: readonly OutcomeObservation[]): readonly OutcomeObservation[] {
  const grouped = new Map<string, OutcomeObservation[]>();
  for (const row of rows) {
    const group = grouped.get(row.member.outcomeId) ?? [];
    group.push(row); grouped.set(row.member.outcomeId, group);
  }
  const current: OutcomeObservation[] = [];
  for (const group of grouped.values()) {
    group.sort((a, b) => BigInt(a.revision) < BigInt(b.revision) ? -1 : BigInt(a.revision) > BigInt(b.revision) ? 1 : 0);
    const original = group[0]!; let previous: string | null = null;
    for (let i = 0; i < group.length; i++) {
      const row = group[i]!;
      if (BigInt(row.revision) !== BigInt(i + 1) || row.supersedesRevision !== previous ||
          immutableObservationId(row) !== immutableObservationId(original)) return reject();
      previous = row.revision;
    }
    current.push(group[group.length - 1]!);
  }
  return Object.freeze(current);
}
function metrics(rows: readonly OutcomeObservation[], binCount: number) {
  let loss = 0;
  const bins = Array.from({ length: binCount }, () => ({ count: 0, confidenceSum: 0, correct: 0 }));
  for (const row of rows) {
    let predicted = row.probabilities[0]!;
    for (const p of row.probabilities) {
      const target = p.candidateId === row.observedCandidateId ? 1 : 0;
      loss += (p.probability - target) ** 2;
      if (p.probability > predicted.probability) predicted = p;
    }
    const bin = bins[Math.min(binCount - 1, Math.floor(predicted.probability * binCount))]!;
    bin.count++; bin.confidenceSum += predicted.probability;
    if (predicted.candidateId === row.observedCandidateId) bin.correct++;
  }
  const buckets = bins.map((b, index) => Object.freeze({ index, count: b.count,
    meanProbability: b.count === 0 ? null : b.confidenceSum / b.count,
    empiricalAccuracy: b.count === 0 ? null : b.correct / b.count }));
  const ece = rows.length === 0 ? null : bins.reduce((sum, b) =>
    sum + (b.count === 0 ? 0 : Math.abs(b.correct - b.confidenceSum) / rows.length), 0);
  return Object.freeze({ observations: rows.length, brier: rows.length === 0 ? null : loss / rows.length,
    brierDefinition: "MEAN_SUM_OVER_CLASSES_SQUARED_ERROR" as const,
    ece, eceRole: "AUXILIARY_TOP_LABEL_BINNING_ONLY" as const, bins: Object.freeze(buckets) });
}

/** Pure report from an already-authorized snapshot, NOT Outcome persistence,
 * current grant validation, label provenance verification or backend qualification.
 * Caller must resolve those facts using Product Authority before disclosure/use.
 * Only calibration rows are accepted: sealed holdout results cannot flow through
 * this optimizer-visible report. No files, transport, model, cache or writer.
 */
export function calibrationReport(value: CalibrationRequest) {
  const raw = record(value, ["domainId", "namespace", "key", "binCount", "observations"]);
  const domainId = text(raw.domainId), namespace = choice(raw.namespace, ["SYNTHETIC", "REAL"]);
  const key = keySnapshot(raw.key), bins = integer(raw.binCount, 1, 100);
  const rows = array(raw.observations).map(observationSnapshot);
  for (const row of rows) {
    if (row.member.domainId !== domainId || row.member.namespace !== namespace ||
        row.member.split !== "CALIBRATION" || keyId(row.key) !== keyId(key)) return reject();
  }
  const current = latest(rows);
  const partitionAudit = auditDatasetPartitions(current.map(r => r.member), domainId, namespace);
  const first = current[0];
  if (first && current.some(row => frame(row.probabilities.map(p => p.candidateId)) !==
      frame(first.probabilities.map(p => p.candidateId)))) return reject();
  const labeled = current.filter(r => r.status === "OBSERVED");
  const byLabelSource = Object.freeze({
    OBJECTIVE: metrics(labeled.filter(r => r.labelSource === "OBJECTIVE"), bins),
    INDEPENDENT_SEMANTIC: metrics(labeled.filter(r => r.labelSource === "INDEPENDENT_SEMANTIC"), bins),
    OWNER_OVERRIDE: metrics(labeled.filter(r => r.labelSource === "OWNER_OVERRIDE"), bins),
  });
  return Object.freeze({ domainId, namespace, key, historyRecords: rows.length, outcomes: current.length,
    correctionRecords: rows.length - current.length, knownConnectedGroups: partitionAudit.connectedGroups,
    independentSampleSize: null, confidenceInterval: null,
    counts: Object.freeze({ observed: labeled.length, pending: current.filter(r => r.status === "PENDING").length,
      censored: current.filter(r => r.status === "CENSORED").length, infrastructure: current.filter(r => r.status === "INFRA_FAILURE").length,
      selfReportExcluded: labeled.filter(r => r.labelSource === "SELF_REPORT").length,
      safetyPrivacyIncidents: current.filter(r => r.safetyPrivacy === "INCIDENT").length,
      safetyPrivacyUnknown: current.filter(r => r.safetyPrivacy === "UNKNOWN").length,
      oodOrUnknown: current.filter(r => r.ood !== "IN_DISTRIBUTION").length }),
    byLabelSource,
    evidenceDimensions: Object.freeze(current.map(r => Object.freeze({ outcomeId: r.member.outcomeId,
      revision: r.revision, nativeConfidence: r.nativeConfidence, probabilities: r.probabilities,
      contextCoverage: r.contextCoverage, candidateCoverage: r.candidateCoverage, freshness: r.freshness,
      scorerReliability: r.scorerReliability, ood: r.ood, safetyPrivacy: r.safetyPrivacy }))),
    universalScore: null, counterfactualOptimalityEstablished: false as const,
    automaticUse: "NOT_AUTHORIZED" as const, qualification: false as const,
    evidenceAuthority: "UPSTREAM_CURRENT_PRODUCT_AUTHORITY_REQUIRED" as const });
}
