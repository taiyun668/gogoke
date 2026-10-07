import { test } from "node:test";
import { strict as assert } from "node:assert";
import { SideChatDelivery, type SideDeliveryIntent, type SideDeliveryRecord } from "./delivery.ts";

const intent: SideDeliveryIntent = {
  domainId: "project", requestId: "request", sideId: "side", direction: "SIDE_TO_LEAD",
  sourceSeatId: "side-seat", sourceSeatIncarnation: "1", sourceSessionId: "side-session",
  targetSeatId: "lead-seat", targetSeatIncarnation: "1", targetSessionId: "lead-session",
  targetGeneration: "2", body: "Tell the lead", messageId: "sidemsg-x",
  enqueueRequestId: "sideenqueue-x", deliveryRequestId: "sidedeliver-x", mayDispatch: true,
  createdAt: "2026-10-06T00:00:00.000Z", dispatchError: "",
};

test("uncertain C submit remains observable and never grants a replay send", async () => {
  let prepares = 0;
  let submits = 0;
  const native = {
    prepare: async () => ({ ...intent, mayDispatch: prepares++ === 0 }),
    observe: async (): Promise<SideDeliveryRecord> => ({ intent, state: "UNKNOWN",
      nativeReceiptId: "", reason: "H response timed out" }),
    recordError: async (_domain: string, _request: string, error: string) => {
      assert.equal(error, "H response timed out");
    },
    lines: async () => [],
  };
  const delivery = new SideChatDelivery(native, { submitOnce: async () => {
    submits++;
    throw new Error("H response timed out");
  } });
  const first = await delivery.send("project", "side", "request", "SIDE_TO_LEAD", "Tell the lead");
  const replay = await delivery.send("project", "side", "request", "SIDE_TO_LEAD", "Tell the lead");
  assert.equal(submits, 1);
  assert.equal(first.state, "UNKNOWN");
  assert.equal(replay.reason, "H response timed out");
});

test("a success line requires the original native receipt", async () => {
  const delivery = new SideChatDelivery({
    prepare: async () => ({ ...intent, mayDispatch: false }),
    observe: async () => ({ intent, state: "STEERED" as const, nativeReceiptId: "", reason: "" }),
    recordError: async () => assert.fail("no submit error"),
    lines: async () => [],
  }, { submitOnce: async () => assert.fail("replayed intent dispatched") });
  await assert.rejects(delivery.send("project", "side", "request", "SIDE_TO_LEAD", "Tell the lead"),
    /V37_SIDE_DELIVERY_RECEIPT_CORRELATION/u);
});
