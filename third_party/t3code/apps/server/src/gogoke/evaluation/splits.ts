import { array, choice, frame, record, reject, text } from "./boundary.ts";

export type DataNamespace = "SYNTHETIC" | "REAL";
export type DatasetSplit = "DEVELOPMENT" | "CALIBRATION" | "SEALED_HOLDOUT";
export interface DatasetMember {
  readonly outcomeId: string;
  readonly domainId: string;
  readonly namespace: DataNamespace;
  readonly split: DatasetSplit;
  readonly projectId: string;
  readonly timeGroup: string;
  readonly sessionLineageId: string;
  readonly nearDuplicateClusterId: string;
}
const FIELDS = Object.freeze(["outcomeId", "domainId", "namespace", "split", "projectId", "timeGroup", "sessionLineageId", "nearDuplicateClusterId"]);
export function memberSnapshot(value: unknown): DatasetMember {
  const raw = record(value, FIELDS);
  return Object.freeze({ outcomeId: text(raw.outcomeId), domainId: text(raw.domainId),
    namespace: choice(raw.namespace, ["SYNTHETIC", "REAL"]),
    split: choice(raw.split, ["DEVELOPMENT", "CALIBRATION", "SEALED_HOLDOUT"]),
    projectId: text(raw.projectId), timeGroup: text(raw.timeGroup),
    sessionLineageId: text(raw.sessionLineageId), nearDuplicateClusterId: text(raw.nearDuplicateClusterId) });
}

/** Pure validation of an ALREADY AUTHORIZED metadata index, not a dataset reader
 * or grant. No row content is loaded, holdout results exposed, or split assigned.
 * Sharing any declared project/time/lineage/duplicate group connects records;
 * transitive groups cannot straddle splits. Group count is not proven statistical
 * independence, and this function cannot authorize cross-domain aggregation.
 */
export function auditDatasetPartitions(value: unknown, domain: string, namespace: DataNamespace) {
  text(domain); choice(namespace, ["SYNTHETIC", "REAL"]);
  const members = array(value).map(memberSnapshot);
  const ids = new Set<string>();
  const parents = members.map((_, i) => i);
  const root = (n: number): number => {
    while (parents[n] !== n) { parents[n] = parents[parents[n]!]!; n = parents[n]!; }
    return n;
  };
  const groups = new Map<string, number>();
  for (let i = 0; i < members.length; i++) {
    const m = members[i]!;
    if (m.domainId !== domain || m.namespace !== namespace || ids.has(m.outcomeId)) return reject();
    ids.add(m.outcomeId);
    for (const field of ["projectId", "timeGroup", "sessionLineageId", "nearDuplicateClusterId"] as const) {
      const key = frame([field, m[field]]);
      const old = groups.get(key);
      if (old === undefined) groups.set(key, i);
      else parents[root(i)] = root(old);
    }
  }
  const splits = new Map<number, DatasetSplit>();
  for (let i = 0; i < members.length; i++) {
    const group = root(i), split = members[i]!.split, previous = splits.get(group);
    if (previous !== undefined && previous !== split) return reject();
    splits.set(group, split);
  }
  return Object.freeze({ domainId: domain, namespace, records: members.length,
    connectedGroups: splits.size, independenceEstablished: false as const,
    qualification: false as const });
}
