import * as assert from "node:assert/strict";
import { it } from "vite-plus/test";
import { InboxDeliveryCoordinator, type HInboxDelivery, type InboxDeliveryReceipt,
  type InboxDeliveryRequest, type NativeInboxDeliveryStore } from "./delivery.ts";

const target = { domainId: "projectA", messageId: "messageA", senderSeatId: "senderA", seatId: "seatA",
  turnId: "turnA", generation: "1", body: "hello" };
const request: InboxDeliveryRequest = { requestId: "deliverA", rawRequest: new TextEncoder().encode("exact wire"),
  target, expectedRevision: "1", kind: "steer" };
const receipt = (status: InboxDeliveryReceipt["status"], state: InboxDeliveryReceipt["state"]):
  InboxDeliveryReceipt => ({ status, state, previousRevision: "1", revision: status === "UNKNOWN" ? "2" : "1" });

function harness(options: { revoke?: boolean; endTurn?: boolean; abortConfirmed?: boolean;
  completion?: "completed" | "unknown"; markDenied?: boolean; } = {}) {
  const calls: string[] = [];
  let turnReads = 0;
  const store: NativeInboxDeliveryStore = {
    reserve: async () => { calls.push("reserve"); return { state: "RESERVED", target }; },
    currentGrant: async () => { calls.push("grant"); return !options.revoke; },
    currentTurn: async () => { calls.push("turn"); turnReads += 1;
      return options.endTurn && turnReads > 1 ? null : "turnA"; },
    abort: async (_request, confirmed) => { calls.push(`store-abort:${confirmed}`);
      return confirmed ? receipt("CONFLICT", "PENDING") : receipt("UNKNOWN", "UNKNOWN"); },
    markCommitUnknown: async () => { calls.push("mark-unknown");
      return options.markDenied ? receipt("DENIED", "PENDING") : receipt("UNKNOWN", "UNKNOWN"); },
    settle: async (_request, result) => { calls.push("settle");
      return result.kind === "completed" ? { ...receipt("APPLIED", "DELIVERED"),
        nativeReceiptId: result.nativeReceiptId,
        ...(result.mode ? { mode: result.mode } : {}) } : receipt("FAILED", "FAILED"); },
  };
  const host: HInboxDelivery = {
    prepare: async (_request, formattedBody) => { calls.push("prepare");
      assert.equal(formattedBody.includes('from-seat="senderA"'), true); return "preparedA"; },
    abortPrepared: async () => { calls.push("host-abort"); return options.abortConfirmed ?? true; },
    beginCommitted: async () => { calls.push("beginCommitted"); },
    complete: async () => { calls.push("completion");
      return options.completion === "unknown" ? { kind: "unknown" } as const :
        { kind: "completed", nativeReceiptId: "nativeReceiptA" } as const; },
    steerMode: async () => { calls.push("mode"); return "INTERRUPT_RESUME"; },
  };
  return { calls, coordinator: new InboxDeliveryCoordinator(store, host) };
}

it("persists UNKNOWN before H begin and marks delivered only with H completion", async () => {
  const h = harness();
  const result = await h.coordinator.deliver(request);
  assert.equal(result.status, "APPLIED");
  assert.equal(result.nativeReceiptId, "nativeReceiptA");
  assert.equal(result.mode, "INTERRUPT_RESUME");
  assert.deepEqual(h.calls, ["reserve", "turn", "mode", "prepare", "grant", "turn",
    "mark-unknown", "beginCommitted", "completion", "settle"]);
});

it("keeps an ended turn queued only after H confirms abort", async () => {
  const h = harness({ endTurn: true });
  assert.equal((await h.coordinator.deliver(request)).state, "PENDING");
  assert.deepEqual(h.calls, ["reserve", "turn", "mode", "prepare", "grant", "turn",
    "host-abort", "store-abort:true"]);
});

it("holds uncertain abort and uncertain completion for reconciliation without a second send", async () => {
  const aborted = harness({ revoke: true, abortConfirmed: false });
  assert.equal((await aborted.coordinator.deliver(request)).status, "UNKNOWN");
  assert.deepEqual(aborted.calls, ["reserve", "turn", "mode", "prepare", "grant", "turn",
    "host-abort", "store-abort:false"]);
  const unknown = harness({ completion: "unknown" });
  assert.equal((await unknown.coordinator.deliver(request)).status, "UNKNOWN");
  assert.equal(unknown.calls.includes("settle"), false);
});

it("aborts a prepared H delivery when native admission rejects the final commit boundary", async () => {
  const h = harness({ markDenied: true });
  assert.equal((await h.coordinator.deliver(request)).state, "PENDING");
  assert.deepEqual(h.calls, ["reserve", "turn", "mode", "prepare", "grant", "turn",
    "mark-unknown", "host-abort", "store-abort:true"]);
});
