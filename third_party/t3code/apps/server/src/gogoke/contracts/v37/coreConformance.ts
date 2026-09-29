import * as assert from "node:assert/strict";
import { canonicalJson } from "../strictJson.ts";
import { decodeV37Receipt, encodeV37Request, V37_SCHEMA, type V37Port, type V37Receipt, type V37Request } from "./protocol.ts";

export type V37CoreCase = "session" | "ledger" | "inbox-race" | "inbox-revoke" | "inbox-unknown";
export interface V37CoreHarness {
  readonly port: V37Port;
  /** Reopen the implementation against the same durable store. */
  reconstruct(): V37Port;
  readonly deliveryCalls?: readonly string[];
}
export type V37CoreHarnessFactory = (caseId: V37CoreCase) => V37CoreHarness;

function request(family: V37Request["family"], operation: V37Request["operation"],
  requestId: string, targetId: string, expectedRevision: string,
  payload: V37Request["payload"]): V37Request {
  return { schema: V37_SCHEMA, family, operation, requestId, targetId,
    domainId: "projectA", expectedRevision, payload };
}

async function call(port: V37Port, r: V37Request): Promise<V37Receipt> {
  const result = decodeV37Receipt(await port.execute(encodeV37Request(r)));
  assert.equal(result.requestId, r.requestId);
  assert.equal(result.targetId, r.targetId);
  return result;
}

/** Runs unchanged against the fake and each real H/A/C port at their milestones. */
export async function runV37CoreContractCases(factory: V37CoreHarnessFactory): Promise<void> {
  {
    const h = factory("session");
    const reserve = request("K-SESSION", "admission-reserve", "reserveA", "sessionA", "0", { generation: "1" });
    assert.equal((await call(h.port, reserve)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), reserve)).status, "REPLAYED");
    assert.equal((await call(h.port, request("K-SESSION", "open", "openEarly", "sessionA", "1",
      { generation: "1", pinnedBinaryDigest: "verifiedDigest" }))).status, "CONFLICT");
    assert.equal((await call(h.port, request("K-SESSION", "admission-commit", "commitA", "sessionA", "1",
      { generation: "1" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "open", "openBadPin", "sessionA", "2",
      { generation: "1", pinnedBinaryDigest: "wrongDigest" }))).status, "DENIED");
    assert.equal((await call(h.port, request("K-SESSION", "open", "openA", "sessionA", "2",
      { generation: "1", pinnedBinaryDigest: "verifiedDigest" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "stop", "stopA", "sessionA", "3",
      { generation: "1" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "exit-and-stop-receipt", "fakeStop", "sessionA", "4",
      { generation: "1", nativeStopProofId: "unverified" }))).status, "DENIED");
    const stopped = await call(h.reconstruct(), request("K-SESSION", "exit-and-stop-receipt", "stopFact", "sessionA", "4",
      { generation: "1", nativeStopProofId: "verifiedProof" }));
    assert.equal(stopped.status, "APPLIED");
    assert.equal(stopped.result.state, "STOPPED");
  }
  {
    const h = factory("ledger");
    const first = request("K-LEDGER", "record", "ledgerA", "ledger", "0",
      { sourceEventId: "oldMessageA", sourceCursor: "1", scope: "PROJECT" });
    assert.equal((await call(h.port, first)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), first)).status, "REPLAYED");
    assert.equal((await call(h.port, request("K-LEDGER", "record", "duplicateSource", "ledger", "1",
      { sourceEventId: "oldMessageA", sourceCursor: "2", scope: "PROJECT" }))).status, "CONFLICT");
    assert.equal((await call(h.port, request("K-LEDGER", "record", "gap", "ledger", "1",
      { sourceEventId: "oldMessageB", sourceCursor: "3", scope: "PROJECT" }))).status, "CONFLICT");
    assert.equal((await call(h.port, request("K-LEDGER", "record", "globalA", "ledger", "1",
      { sourceEventId: "globalEvent", sourceCursor: "2", scope: "GLOBAL" }))).status, "APPLIED");
    const project = await call(h.reconstruct(), request("K-LEDGER", "scoped-query", "queryProject", "ledger", "2",
      { epoch: "1", afterCursor: "0", scope: "PROJECT" }));
    assert.equal(project.status, "APPLIED");
    assert.equal(canonicalJson(project.result.events!),
      canonicalJson([{ sourceEventId: "oldMessageA", cursor: "1" }]));
    assert.equal(project.revision, project.previousRevision);
    assert.equal((await call(h.port, request("K-LEDGER", "scoped-query", "globalDenied", "ledger", "2",
      { epoch: "1", afterCursor: "0", scope: "GLOBAL" }))).status, "DENIED");
    assert.equal((await call(h.port, request("K-LEDGER", "scoped-query", "staleEpoch", "ledger", "2",
      { epoch: "2", afterCursor: "0", scope: "PROJECT" }))).status, "STALE");
  }
  {
    const h = factory("inbox-race");
    const enqueue = request("K-INBOX", "enqueue", "enqueueA", "messageA", "0",
      { seatId: "seatA", turnId: "turnA", generation: "1", body: "first" });
    assert.equal((await call(h.port, enqueue)).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), enqueue)).status, "REPLAYED");
    const competing = await Promise.all([
      call(h.port, request("K-INBOX", "edit", "editA", "messageA", "1", { body: "second" })),
      call(h.port, request("K-INBOX", "cancel", "cancelA", "messageA", "1", {})),
    ]);
    assert.deepEqual(competing.map((r) => r.status).sort(), ["APPLIED", "STALE"]);
    assert.equal((await call(h.port, request("K-INBOX", "deliver", "wrongGeneration", "messageA", "2",
      { generation: "2" }))).status, "CONFLICT");
  }
  {
    const h = factory("inbox-revoke");
    assert.equal((await call(h.port, request("K-INBOX", "enqueue", "enqueueRevoke", "messageR", "0",
      { seatId: "seatA", turnId: "turnA", generation: "1", body: "private" }))).status, "APPLIED");
    const denied = await call(h.port, request("K-INBOX", "deliver", "deliverRevoked", "messageR", "1",
      { generation: "1" }));
    assert.equal(denied.status, "DENIED");
    assert.equal(denied.revision, "1");
    assert.deepEqual(h.deliveryCalls, ["prepare"]);
  }
  {
    const h = factory("inbox-unknown");
    assert.equal((await call(h.port, request("K-INBOX", "enqueue", "enqueueUnknown", "messageU", "0",
      { seatId: "seatA", turnId: "turnA", generation: "1", body: "uncertain" }))).status, "APPLIED");
    const unknown = await call(h.port, request("K-INBOX", "deliver", "deliverUnknown", "messageU", "1",
      { generation: "1" }));
    assert.equal(unknown.status, "UNKNOWN");
    assert.equal((await call(h.reconstruct(), request("K-INBOX", "deliver", "deliverUnknown", "messageU", "1",
      { generation: "1" }))).status, "UNKNOWN");
    const check = await call(h.reconstruct(), request("K-INBOX", "check-unknown", "checkUnknown", "messageU", "2", {}));
    assert.equal(check.result.state, "UNKNOWN");
    assert.equal(check.revision, check.previousRevision);
    assert.equal((await call(h.port, request("K-INBOX", "requeue", "requeueUnknown", "messageU", "2", {}))).status, "CONFLICT");
  }
}
