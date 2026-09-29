import * as assert from "node:assert/strict";
import { canonicalJson } from "../strictJson.ts";
import { decodeV37Receipt, encodeV37Request, V37_SCHEMA, type V37Port, type V37Receipt, type V37Request } from "./protocol.ts";

export type V37CoreCase = "session" | "session-more" | "session-release" |
  "session-unsupported" | "session-unknown" | "session-resume" | "session-resume-unsupported" |
  "session-resume-custody-unknown" | "session-resume-binding-mismatch" |
  "session-resume-vendor-unknown" | "ledger" | "ledger-subscription" |
  "ledger-subscription-revoked" | "inbox-race" | "inbox-revoke" | "inbox-unknown" | "inbox-steer" |
  "inbox-steer-fallback" |
  "inbox-steer-ended" | "inbox-steer-race" | "inbox-steer-abort-unknown" | "inbox-failed";
export interface V37CoreHarness {
  readonly port: V37Port;
  /** Reopen the implementation against the same durable store. */
  reconstruct(): V37Port;
  readonly deliveryCalls?: readonly string[];
  readonly resumeCalls?: readonly string[];
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
    assert.equal((await call(h.port, request("K-SESSION", "admission-release", "releaseA", "sessionA", "5",
      { generation: "1" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "resume", "resumeReleased", "sessionA", "6",
      { generation: "1" }))).status, "CONFLICT");
  }
  {
    const h = factory("session-more");
    for (const [operation, revision, payload] of [
      ["admission-reserve", "0", { generation: "1" }],
      ["admission-commit", "1", { generation: "1" }],
      ["open", "2", { generation: "1", pinnedBinaryDigest: "verifiedDigest" }],
    ] as const) {
      assert.equal((await call(h.port, request("K-SESSION", operation, `more${operation}`, "sessionB", revision, payload))).status,
        "APPLIED");
    }
    const capability = await call(h.port, request("K-SESSION", "capability-probe", "probeB", "sessionB", "3",
      { generation: "1" }));
    assert.equal((capability.result.capabilities as { compact: boolean }).compact, true);
    assert.equal(capability.revision, capability.previousRevision);
    const output = await call(h.port, request("K-SESSION", "output-stream", "outputB", "sessionB", "3",
      { generation: "1", afterCursor: "0" }));
    assert.equal(output.result.cursor, "1");
    assert.equal(output.revision, output.previousRevision);
    const sent = await call(h.port, request("K-SESSION", "send", "sendB", "sessionB", "3",
      { generation: "1", body: "start" }));
    assert.equal(sent.result.createdTurn, true);
    const appended = await call(h.port, request("K-SESSION", "append-without-turn", "appendB", "sessionB", "4",
      { generation: "1", body: "context" }));
    assert.equal(appended.result.createdTurn, false);
    const compact = await call(h.port, request("K-SESSION", "compact", "compactB", "sessionB", "5",
      { generation: "1" }));
    assert.equal(compact.result.oldGeneration, "1");
    assert.equal(compact.result.newGeneration, "2");
    assert.equal((await call(h.port, request("K-SESSION", "send", "oldGenSend", "sessionB", "6",
      { generation: "1", body: "wrong generation" }))).status, "CONFLICT");
    const renew = await call(h.reconstruct(), request("K-SESSION", "renew-session", "renewB", "sessionB", "6",
      { generation: "2" }));
    assert.equal(renew.result.oldGeneration, "2");
    assert.equal(renew.result.newGeneration, "3");
    assert.equal((await call(h.port, request("K-SESSION", "reconnect", "reconnectB", "sessionB", "7",
      { generation: "3" }))).result.newGeneration, "3");
    assert.equal((await call(h.port, request("K-SESSION", "stop", "stopB", "sessionB", "8",
      { generation: "3" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "exit-and-stop-receipt", "stopFactB", "sessionB", "9",
      { generation: "3", nativeStopProofId: "verifiedProof" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "admission-release", "releaseB", "sessionB", "10",
      { generation: "3" }))).result.state, "RELEASED");
  }
  {
    const h = factory("session-release");
    assert.equal((await call(h.port, request("K-SESSION", "admission-reserve", "reserveRelease", "sessionR", "0",
      { generation: "1" }))).status, "APPLIED");
    assert.equal((await call(h.reconstruct(), request("K-SESSION", "admission-release", "releaseReserved", "sessionR", "1",
      { generation: "1" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "admission-commit", "commitReleased", "sessionR", "2",
      { generation: "1" }))).status, "CONFLICT");
  }
  {
    const h = factory("session-unsupported");
    assert.equal((await call(h.port, request("K-SESSION", "admission-reserve", "unsupportedReserve", "sessionU", "0",
      { generation: "1" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "admission-commit", "unsupportedCommit", "sessionU", "1",
      { generation: "1" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "open", "unsupportedOpen", "sessionU", "2",
      { generation: "1", pinnedBinaryDigest: "verifiedDigest" }))).status, "APPLIED");
    const unsupported = await call(h.port, request("K-SESSION", "compact", "unsupportedCompact", "sessionU", "3",
      { generation: "1" }));
    assert.equal(unsupported.status, "UNSUPPORTED");
    assert.equal(unsupported.revision, "3");
    assert.equal((await call(h.port, request("K-SESSION", "resume", "unsupportedResume", "sessionU", "3",
      { generation: "1" }))).status, "CONFLICT");
  }
  for (const caseId of ["session-resume", "session-resume-unsupported",
    "session-resume-custody-unknown", "session-resume-binding-mismatch",
    "session-resume-vendor-unknown"] as const) {
    const h = factory(caseId);
    const sessionId = `session${caseId}`;
    for (const [operation, revision, payload] of [
      ["admission-reserve", "0", { generation: "1" }],
      ["admission-commit", "1", { generation: "1" }],
      ["open", "2", { generation: "1", pinnedBinaryDigest: "verifiedDigest" }],
    ] as const) {
      assert.equal((await call(h.port, request("K-SESSION", operation,
        `${caseId}${operation}`, sessionId, revision, payload))).status, "APPLIED");
    }
    assert.equal((await call(h.port, request("K-SESSION", "resume", `${caseId}Running`, sessionId, "3",
      { generation: "1" }))).status, "CONFLICT");
    assert.equal((await call(h.port, request("K-SESSION", "stop", `${caseId}Stop`, sessionId, "3",
      { generation: "1" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "resume", `${caseId}Stopping`, sessionId, "4",
      { generation: "1" }))).status, "CONFLICT");
    assert.equal((await call(h.reconstruct(), request("K-SESSION", "exit-and-stop-receipt",
      `${caseId}StopFact`, sessionId, "4",
      { generation: "1", nativeStopProofId: "verifiedProof" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-SESSION", "resume", `${caseId}WrongGeneration`, sessionId, "5",
      { generation: "2" }))).status, "CONFLICT");
    assert.equal((await call(h.port, request("K-SESSION", "reconnect", `${caseId}Reconnect`, sessionId, "5",
      { generation: "1" }))).status, "CONFLICT");
    await assert.rejects(() => call(h.port, request("K-SESSION", "resume", `${caseId}WireRef`, sessionId, "5",
      { generation: "1", continuationRef: "untrusted" })), /resume payload/);
    const resume = request("K-SESSION", "resume", `${caseId}Resume`, sessionId, "5", { generation: "1" });
    const first = await call(h.port, resume);
    if (caseId === "session-resume-unsupported") {
      assert.equal(first.status, "UNSUPPORTED");
      assert.equal(first.revision, "5");
      assert.deepEqual(h.resumeCalls, []);
    } else if (caseId === "session-resume-binding-mismatch") {
      assert.equal(first.status, "CONFLICT");
      assert.equal(first.revision, "5");
      assert.deepEqual(h.resumeCalls, []);
    } else if (caseId === "session-resume-custody-unknown") {
      assert.equal(first.status, "UNKNOWN");
      assert.equal(first.result.state, "RESUME_UNKNOWN");
      assert.equal((await call(h.port, request("K-SESSION", "resume", `${caseId}Second`, sessionId, "6",
        { generation: "1" }))).status, "CONFLICT");
      assert.equal((await call(h.port, { ...resume, expectedRevision: "6" })).status, "CONFLICT");
      assert.equal((await call(h.reconstruct(), resume)).status, "UNKNOWN");
      assert.deepEqual(h.resumeCalls, []);
    } else if (caseId === "session-resume-vendor-unknown") {
      assert.equal(first.status, "UNKNOWN");
      assert.equal(first.result.state, "RESUME_UNKNOWN");
      assert.equal((await call(h.port, request("K-SESSION", "resume", `${caseId}Second`, sessionId, "6",
        { generation: "1" }))).status, "CONFLICT");
      assert.equal((await call(h.port, { ...resume, expectedRevision: "6" })).status, "CONFLICT");
      assert.equal((await call(h.reconstruct(), resume)).status, "UNKNOWN");
      const resolved = await call(h.reconstruct(), resume);
      assert.equal(resolved.status, "REPLAYED");
      assert.equal(resolved.previousRevision, "6");
      assert.equal(resolved.revision, "7");
      assert.equal(canonicalJson(resolved.result), canonicalJson({ state: "RUNNING", oldGeneration: "1",
        newGeneration: "2", receiptId: "reconciledReceipt" }));
      assert.deepEqual(h.resumeCalls, [`${sessionId}:1`]);
    } else {
      assert.equal(first.status, "APPLIED");
      assert.equal(canonicalJson(first.result), canonicalJson({ state: "RUNNING", oldGeneration: "1",
        newGeneration: "2", receiptId: "resumeReceipt" }));
      assert.equal((await call(h.reconstruct(), resume)).status, "REPLAYED");
      const reordered = new TextEncoder().encode(JSON.stringify(resume));
      assert.equal(decodeV37Receipt(await h.port.execute(reordered)).status, "CONFLICT");
      assert.equal((await call(h.port, request("K-SESSION", "send", `${caseId}OldGeneration`, sessionId, "6",
        { generation: "1", body: "stale" }))).status, "CONFLICT");
      assert.equal((await call(h.port, request("K-SESSION", "send", `${caseId}NewGeneration`, sessionId, "6",
        { generation: "2", body: "continued" }))).status, "APPLIED");
      assert.deepEqual(h.resumeCalls, [`${sessionId}:1`]);
    }
  }
  {
    const h = factory("session-unknown");
    for (const [operation, revision, payload] of [
      ["admission-reserve", "0", { generation: "1" }],
      ["admission-commit", "1", { generation: "1" }],
      ["open", "2", { generation: "1", pinnedBinaryDigest: "verifiedDigest" }],
    ] as const) {
      assert.equal((await call(h.port, request("K-SESSION", operation, `unknown${operation}`, "sessionX", revision, payload))).status,
        "APPLIED");
    }
    const change = request("K-SESSION", "compact", "unknownCompact", "sessionX", "3", { generation: "1" });
    assert.equal((await call(h.port, change)).status, "UNKNOWN");
    assert.equal((await call(h.reconstruct(), change)).status, "UNKNOWN");
    assert.equal((await call(h.port, request("K-SESSION", "send", "blockedInput", "sessionX", "4",
      { generation: "1", body: "must wait" }))).status, "CONFLICT");
    assert.equal((await call(h.port, request("K-SESSION", "reconnect", "resolveGeneration", "sessionX", "4",
      { generation: "1" }))).result.newGeneration, "2");
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
    const h = factory("ledger-subscription");
    assert.equal((await call(h.port, request("K-LEDGER", "record", "subEventA", "ledger", "0",
      { sourceEventId: "eventA", sourceCursor: "1", scope: "PROJECT" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-LEDGER", "record", "subGlobal", "ledger", "1",
      { sourceEventId: "eventGlobal", sourceCursor: "2", scope: "GLOBAL" }))).status, "APPLIED");
    const subscribe = request("K-LEDGER", "subscribe", "subscribeA", "subA", "0",
      { scope: "PROJECT", epoch: "1", afterCursor: "0" });
    const first = await call(h.port, subscribe);
    assert.equal(canonicalJson(first.result.events!), canonicalJson([{ sourceEventId: "eventA", cursor: "1" }]));
    assert.equal(first.result.cursor, "2");
    assert.equal((await call(h.reconstruct(), subscribe)).status, "REPLAYED");
    assert.equal((await call(h.port, request("K-LEDGER", "subscribe", "globalSubscribe", "subGlobal", "0",
      { scope: "GLOBAL", epoch: "1", afterCursor: "0" }))).status, "DENIED");
    assert.equal((await call(h.port, request("K-LEDGER", "record", "subEventB", "ledger", "2",
      { sourceEventId: "eventB", sourceCursor: "3", scope: "PROJECT" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-LEDGER", "resume-subscription", "rewindSub", "subA", "1",
      { epoch: "1", afterCursor: "1" }))).result.reason, "CURSOR_REWIND");
    assert.equal((await call(h.port, request("K-LEDGER", "resume-subscription", "gapSub", "subA", "1",
      { epoch: "1", afterCursor: "3" }))).result.reason, "CURSOR_GAP");
    const resumed = await call(h.reconstruct(), request("K-LEDGER", "resume-subscription", "resumeSub", "subA", "1",
      { epoch: "1", afterCursor: "2" }));
    assert.equal(canonicalJson(resumed.result.events!), canonicalJson([{ sourceEventId: "eventB", cursor: "3" }]));
    assert.equal(resumed.result.cursor, "3");
    assert.equal((await call(h.port, request("K-LEDGER", "end-subscription", "endSub", "subA", "2", {}))).status,
      "APPLIED");
    assert.equal((await call(h.port, request("K-LEDGER", "resume-subscription", "resumeEnded", "subA", "3",
      { epoch: "1", afterCursor: "3" }))).status, "CONFLICT");
    assert.equal((await call(h.port, request("K-LEDGER", "scoped-query", "aheadQuery", "ledger", "3",
      { epoch: "1", afterCursor: "4", scope: "PROJECT" }))).result.reason, "CURSOR_AHEAD");
  }
  {
    const h = factory("ledger-subscription-revoked");
    assert.equal((await call(h.port, request("K-LEDGER", "record", "revokedEvent", "ledger", "0",
      { sourceEventId: "firstEvent", sourceCursor: "1", scope: "PROJECT" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-LEDGER", "subscribe", "revokedSubscribe", "subR", "0",
      { scope: "PROJECT", epoch: "1", afterCursor: "0" }))).status, "APPLIED");
    assert.equal((await call(h.port, request("K-LEDGER", "record", "revokedSecond", "ledger", "1",
      { sourceEventId: "secondEvent", sourceCursor: "2", scope: "PROJECT" }))).status, "APPLIED");
    const denied = await call(h.reconstruct(), request("K-LEDGER", "resume-subscription", "revokedResume", "subR", "1",
      { epoch: "1", afterCursor: "1" }));
    assert.equal(denied.status, "DENIED");
    assert.equal(denied.revision, "1");
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
    assert.deepEqual(h.deliveryCalls, ["prepare", "abort"]);
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
  for (const caseId of ["inbox-steer", "inbox-steer-fallback"] as const) {
    const h = factory(caseId);
    assert.equal((await call(h.port, request("K-INBOX", "enqueue", "steerEnqueue", "messageS", "0",
      { seatId: "seatA", turnId: "turnA", generation: "1", body: "interrupt" }))).status, "APPLIED");
    const steered = await call(h.port, request("K-INBOX", "steer", "steerMessage", "messageS", "1",
      { turnId: "turnA", generation: "1" }));
    assert.equal(steered.status, "APPLIED");
    assert.equal(steered.result.state, "DELIVERED");
    assert.equal(steered.result.mode, caseId === "inbox-steer-fallback" ? "INTERRUPT_RESUME" : "NATIVE");
    assert.deepEqual(h.deliveryCalls, ["prepare", "beginCommitted", "completion"]);
    assert.equal((await call(h.port, request("K-INBOX", "edit", "editSteered", "messageS", "2",
      { body: "too late" }))).status, "CONFLICT");
  }
  for (const caseId of ["inbox-steer-ended", "inbox-steer-race"] as const) {
    const h = factory(caseId);
    assert.equal((await call(h.port, request("K-INBOX", "enqueue", `${caseId}Enqueue`, "messageE", "0",
      { seatId: "seatA", turnId: "turnA", generation: "1", body: "wait" }))).status, "APPLIED");
    const ended = await call(h.port, request("K-INBOX", "steer", `${caseId}Steer`, "messageE", "1",
      { turnId: "turnA", generation: "1" }));
    assert.equal(ended.status, "CONFLICT");
    assert.equal(ended.result.reason, "TURN_ENDED");
    assert.equal(ended.revision, "1");
    const queued = await call(h.reconstruct(), request("K-INBOX", "check-unknown", `${caseId}Check`, "messageE", "1", {}));
    assert.equal(queued.result.state, "PENDING");
    assert.deepEqual(h.deliveryCalls, caseId === "inbox-steer-race" ? ["prepare", "abort"] : []);
  }
  {
    const h = factory("inbox-steer-abort-unknown");
    assert.equal((await call(h.port, request("K-INBOX", "enqueue", "abortUnknownEnqueue", "messageAbort", "0",
      { seatId: "seatA", turnId: "turnA", generation: "1", body: "do not redirect" }))).status,
      "APPLIED");
    const uncertain = await call(h.port, request("K-INBOX", "steer", "abortUnknownSteer", "messageAbort", "1",
      { turnId: "turnA", generation: "1" }));
    assert.equal(uncertain.status, "UNKNOWN");
    assert.equal(uncertain.result.state, "UNKNOWN");
    assert.deepEqual(h.deliveryCalls, ["prepare", "abort"]);
    assert.equal((await call(h.reconstruct(), request("K-INBOX", "check-unknown", "abortUnknownCheck", "messageAbort", "2",
      {}))).result.state, "UNKNOWN");
  }
  {
    const h = factory("inbox-failed");
    assert.equal((await call(h.port, request("K-INBOX", "enqueue", "failedEnqueue", "messageF", "0",
      { seatId: "seatA", turnId: "turnA", generation: "1", body: "retry later" }))).status, "APPLIED");
    const failed = await call(h.port, request("K-INBOX", "deliver", "failedDeliver", "messageF", "1",
      { generation: "1" }));
    assert.equal(failed.status, "FAILED");
    assert.equal(failed.result.error, "native delivery rejected");
    await assert.rejects(() => call(h.port, request("K-INBOX", "requeue", "missingTarget", "messageF", "2",
      { newMessageId: "messageMissing", seatId: "seatA", generation: "1" })), /payload.turnId/);
    const staleTarget = await call(h.port, request("K-INBOX", "requeue", "staleTarget", "messageF", "2",
      { newMessageId: "messageStale", seatId: "seatA", turnId: "endedTurn", generation: "1" }));
    assert.equal(staleTarget.status, "DENIED");
    assert.equal(staleTarget.revision, "2");
    const requeued = await call(h.reconstruct(), request("K-INBOX", "requeue", "requeueFailed", "messageF", "2",
      { newMessageId: "messageNew", seatId: "seatA", turnId: "turnA", generation: "1" }));
    assert.equal(requeued.status, "APPLIED");
    assert.equal(requeued.result.newMessageId, "messageNew");
    assert.equal(requeued.revision, "3");
    const old = await call(h.port, request("K-INBOX", "check-unknown", "oldCheck", "messageF", "3", {}));
    assert.equal(old.result.state, "FAILED");
    assert.equal(old.result.requeuedAs, "messageNew");
    assert.equal((await call(h.reconstruct(), request("K-INBOX", "deliver", "failedDeliver", "messageF", "1",
      { generation: "1" }))).result.error, "native delivery rejected");
    assert.equal((await call(h.port, request("K-INBOX", "check-unknown", "newCheck", "messageNew", "1", {}))).result.state,
      "PENDING");
    assert.equal((await call(h.port, request("K-INBOX", "requeue", "requeueAgain", "messageF", "3",
      { newMessageId: "messageAgain", seatId: "seatA", turnId: "turnA", generation: "1" }))).status,
      "CONFLICT");
  }
}
