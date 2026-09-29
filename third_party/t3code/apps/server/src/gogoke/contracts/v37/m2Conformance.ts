import * as assert from "node:assert/strict";
import { decodeV37Receipt, encodeV37Request, V37_SCHEMA, type V37Port, type V37Receipt, type V37Request } from "./protocol.ts";

export type V37M2Case = "side" | "side-delete-denied" | "gate-pass" | "gate-reject" |
  "trigger" | "worktree-no-grant" | "worktree-no-stop" | "worktree-active-reservation" |
  "worktree-ready" | "worktree-merge-unknown";
export interface V37M2Harness { readonly port: V37Port; reconstruct(): V37Port; }
export type V37M2HarnessFactory = (caseId: V37M2Case) => V37M2Harness;

const req = (family: V37Request["family"], operation: V37Request["operation"],
  requestId: string, targetId: string, expectedRevision: string,
  payload: V37Request["payload"] = {}): V37Request => ({
  schema: V37_SCHEMA, family, operation, requestId, targetId,
  domainId: "projectA", expectedRevision, payload,
});
const call = async (port: V37Port, request: V37Request): Promise<V37Receipt> => {
  const result = decodeV37Receipt(await port.execute(encodeV37Request(request)));
  assert.equal(result.requestId, request.requestId);
  assert.equal(result.targetId, request.targetId);
  return result;
};

/** Behavioral cases reusable by future native-backed D/E/F ports. */
export async function runV37M2ContractCases(factory: V37M2HarnessFactory): Promise<void> {
  {
    const h = factory("side");
    const create = req("K-SIDE", "create", "sideCreate", "sideA", "0", { sourceCursor: "12" });
    assert.equal((await call(h.port, create)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), create)).status, "REPLAYED");
    assert.equal((await call(h.port, req("K-SIDE", "archive", "sideArchive", "sideA", "1"))).status,
      "APPLIED");
    const archived = await call(h.reconstruct(), req("K-SIDE", "read-thread", "sideRead", "sideA", "2"));
    assert.equal(archived.result.state, "ARCHIVED");
    assert.equal(archived.result.sourceCursor, "12");
    assert.equal(archived.result.mainContextCopy, false);
    assert.equal(archived.revision, archived.previousRevision);
    const pending = await call(h.port, req("K-SIDE", "pending-delta", "sidePending", "sideA", "2"));
    assert.equal(pending.result.sourceCursor, "12");
    assert.equal(pending.revision, pending.previousRevision);
    assert.equal((await call(h.port, req("K-SIDE", "restore", "sideRestore", "sideA", "2"))).status,
      "APPLIED");
    assert.equal((await call(h.port, req("K-SIDE", "resume", "sideResume", "sideA", "3"))).status,
      "APPLIED");
    assert.equal((await call(h.port, req("K-SIDE", "delete", "sideDelete", "sideA", "4"))).status,
      "APPLIED");
    assert.equal((await call(h.reconstruct(), req("K-SIDE", "read-thread", "sideDeletedRead", "sideA", "5"))).status,
      "CONFLICT");
  }
  {
    const h = factory("side-delete-denied");
    assert.equal((await call(h.port, req("K-SIDE", "create", "sideNoDeleteCreate", "sideB", "0",
      { sourceCursor: "0" }))).status, "APPLIED");
    const denied = await call(h.port, req("K-SIDE", "delete", "sideNoDelete", "sideB", "1"));
    assert.equal(denied.status, "DENIED");
    assert.equal(denied.revision, "1");
  }
  {
    const h = factory("gate-pass");
    assert.equal((await call(h.port, req("K-POLICY", "call-permission-table", "unwiredPermissionRead", "gateA", "0"))).status,
      "UNSUPPORTED");
    const submit = req("K-POLICY", "gate-submit", "gateSubmit", "gateA", "0");
    assert.equal((await call(h.port, submit)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), submit)).status, "REPLAYED");
    assert.equal((await call(h.port, req("K-POLICY", "stage-transition", "earlyStage", "gateA", "1"))).status,
      "CONFLICT");
    assert.equal((await call(h.port, req("K-POLICY", "gate-decide", "gatePass", "gateA", "1",
      { decision: "PASS" }))).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), req("K-POLICY", "stage-transition", "stageA", "gateA", "2"))).result.state,
      "ADVANCED");
  }
  {
    const h = factory("gate-reject");
    assert.equal((await call(h.port, req("K-POLICY", "gate-submit", "rejectSubmit", "gateB", "0"))).status,
      "APPLIED");
    const rejected = await call(h.port, req("K-POLICY", "gate-decide", "rejectDecision", "gateB", "1",
      { decision: "REJECT", reason: "Evidence is incomplete" }));
    assert.equal(rejected.result.state, "REJECTED");
    assert.equal(rejected.result.reason, "Evidence is incomplete");
    assert.equal((await call(h.port, req("K-POLICY", "stage-transition", "rejectedStage", "gateB", "2"))).status,
      "CONFLICT");
  }
  {
    const h = factory("trigger");
    const register = req("K-POLICY", "trigger-register", "triggerRegister", "triggerA", "0",
      { eventRef: "sourceEventA" });
    assert.equal((await call(h.port, register)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), register)).status, "REPLAYED");
    assert.equal((await call(h.reconstruct(), req("K-POLICY", "trigger-register", "duplicateTrigger", "triggerA", "1",
      { eventRef: "sourceEventA" }))).status, "CONFLICT");
    assert.equal((await call(h.port, req("K-POLICY", "trigger-recover", "triggerRecover", "triggerA", "1"))).status,
      "APPLIED");
    assert.equal((await call(h.port, req("K-POLICY", "trigger-cancel", "triggerCancel", "triggerA", "2"))).status,
      "APPLIED");
    assert.equal((await call(h.port, req("K-POLICY", "trigger-recover", "recoverCancelled", "triggerA", "3"))).status,
      "CONFLICT");
  }
  for (const caseId of ["worktree-no-grant", "worktree-no-stop", "worktree-active-reservation", "worktree-ready"] as const) {
    const h = factory(caseId);
    assert.equal((await call(h.port, req("K-WORKTREE", "create", `${caseId}BadRepo`, "treeA", "0",
      { repositoryId: "unverifiedRepo", seatId: "seatA" }))).status, "DENIED");
    const create = req("K-WORKTREE", "create", `${caseId}Create`, "treeA", "0",
      { repositoryId: "verifiedRepo", seatId: "seatA" });
    assert.equal((await call(h.port, create)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), create)).status, "REPLAYED");
    const graph = await call(h.port, req("K-WORKTREE", "graph-query", `${caseId}Graph`, "treeA", "1"));
    assert.equal(graph.result.classification, "SINGLE");
    assert.equal(graph.revision, graph.previousRevision);
    assert.equal((await call(h.port, req("K-WORKTREE", "classify-single-or-mixed", `${caseId}Classify`, "treeA", "1"))).result.classification,
      "SINGLE");
    assert.equal((await call(h.port, req("K-WORKTREE", "register", `${caseId}Register`, "treeA", "1"))).status,
      "APPLIED");
    const merge = await call(h.port, req("K-WORKTREE", "merge", `${caseId}Merge`, "treeA", "2",
      { decision: "MERGE", reason: "Reviewed changes" }));
    assert.equal(merge.status, caseId === "worktree-no-grant" ? "DENIED" : "APPLIED");
    if (caseId === "worktree-no-grant") continue;
    const cleanup = await call(h.port, req("K-WORKTREE", "cleanup", `${caseId}Cleanup`, "treeA", "3"));
    assert.equal(cleanup.status, caseId === "worktree-ready" ? "APPLIED" : "DENIED");
    if (caseId === "worktree-ready") {
      assert.equal((await call(h.reconstruct(), req("K-WORKTREE", "graph-query", "cleanedGraph", "treeA", "4"))).status,
        "CONFLICT");
    }
  }
  {
    const h = factory("worktree-merge-unknown");
    assert.equal((await call(h.port, req("K-WORKTREE", "create", "unknownTreeCreate", "treeU", "0",
      { repositoryId: "verifiedRepo", seatId: "seatA" }))).status, "APPLIED");
    assert.equal((await call(h.port, req("K-WORKTREE", "register", "unknownTreeRegister", "treeU", "1"))).status,
      "APPLIED");
    const merge = req("K-WORKTREE", "merge", "unknownMerge", "treeU", "2",
      { decision: "MERGE", reason: "Reviewed changes" });
    assert.equal((await call(h.port, merge)).status, "UNKNOWN");
    assert.equal((await call(h.reconstruct(), merge)).status, "UNKNOWN");
    const graph = await call(h.port, req("K-WORKTREE", "graph-query", "unknownGraph", "treeU", "3"));
    assert.equal(graph.result.state, "MERGE_UNKNOWN");
    assert.equal((await call(h.port, req("K-WORKTREE", "merge", "blindRetry", "treeU", "3",
      { decision: "MERGE", reason: "Do it again" }))).status, "CONFLICT");
  }
}
