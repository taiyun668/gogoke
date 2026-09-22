import * as Assert from "node:assert/strict";
import { describe, it } from "vite-plus/test";
import { auditDatasetPartitions, type DatasetMember } from "./splits.ts";
import { EvaluationInputError } from "./boundary.ts";

const member = (id: string, changes: Partial<DatasetMember> = {}): DatasetMember => ({
  outcomeId: id,
  domainId: "domain-one",
  namespace: "SYNTHETIC",
  split: "DEVELOPMENT",
  projectId: `project-${id}`,
  timeGroup: `time-${id}`,
  sessionLineageId: `lineage-${id}`,
  nearDuplicateClusterId: `cluster-${id}`,
  ...changes,
});
const audit = (members: readonly DatasetMember[]) =>
  auditDatasetPartitions(members, "domain-one", "SYNTHETIC");
const rejects = (fn: () => unknown) => Assert.throws(fn, EvaluationInputError);

describe("authorized dataset metadata split audit", () => {
  it("ES01 disjoint declared groups may occupy different splits without granting access", () => {
    const result = audit([
      member("dev"),
      member("cal", { split: "CALIBRATION" }),
      member("hold", { split: "SEALED_HOLDOUT" }),
    ]);
    Assert.equal(result.records, 3);
    Assert.equal(result.connectedGroups, 3);
    Assert.equal(result.independenceEstablished, false);
    Assert.equal(result.qualification, false);
  });
  it("ES02 common project forbids cross-split reuse", () => {
    rejects(() =>
      audit([member("dev"), member("hold", { projectId: "project-dev", split: "SEALED_HOLDOUT" })]),
    );
  });
  it("ES03 common time group forbids cross-split reuse", () => {
    rejects(() =>
      audit([member("dev"), member("cal", { timeGroup: "time-dev", split: "CALIBRATION" })]),
    );
  });
  it("ES04 common session lineage forbids cross-split reuse", () => {
    rejects(() =>
      audit([
        member("dev"),
        member("hold", { sessionLineageId: "lineage-dev", split: "SEALED_HOLDOUT" }),
      ]),
    );
  });
  it("ES05 near-duplicate cluster forbids cross-split reuse", () => {
    rejects(() =>
      audit([
        member("dev"),
        member("hold", { nearDuplicateClusterId: "cluster-dev", split: "SEALED_HOLDOUT" }),
      ]),
    );
  });
  it("ES06 transitive mixed-axis groups cannot carry holdout feedback into development", () => {
    rejects(() =>
      audit([
        member("a"),
        member("b", { projectId: "project-a", sessionLineageId: "shared" }),
        member("c", { sessionLineageId: "shared", split: "SEALED_HOLDOUT" }),
      ]),
    );
  });
  it("ES07 cross-domain and real/synthetic mixtures fail before any split summary", () => {
    rejects(() => audit([member("a"), member("b", { domainId: "foreign" })]));
    rejects(() => audit([member("a"), member("b", { namespace: "REAL" })]));
  });
  it("ES08 duplicate outcome identities cannot inflate a dataset", () => {
    rejects(() => audit([member("same"), member("same")]));
  });
  it("ES09 passive metadata rejects getters proxies and extra holdout row contents", () => {
    let calls = 0;
    const value = member("one");
    Object.defineProperty(value, "split", {
      enumerable: true,
      get() {
        calls++;
        return "DEVELOPMENT";
      },
    });
    rejects(() => audit([value]));
    rejects(() =>
      audit([
        new Proxy(member("two"), {
          get(target, key, receiver) {
            calls++;
            return Reflect.get(target, key, receiver);
          },
        }),
      ]),
    );
    rejects(() => audit([{ ...member("three"), contents: "private-row-result" } as DatasetMember]));
    Assert.equal(calls, 0);
  });
  it("ES10 correlated rows report one declared group, not fictitious independent evidence", () => {
    const result = audit(
      Array.from({ length: 100 }, (_, i) => member(String(i), { projectId: "one-project" })),
    );
    Assert.equal(result.records, 100);
    Assert.equal(result.connectedGroups, 1);
    Assert.equal(result.independenceEstablished, false);
  });
});
