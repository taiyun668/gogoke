import { describe, expect, it } from "vitest";
import type { SeatsPage } from "@/features/seats/seatsPageModel";
import type { Design37SideChatPage } from "@/services/design37SideChats";
import { projectSideChats } from "./design37SideChatSource";

const claude = { id: "inst-c", name: "Claude Pro", vendor: "claude" as const, models: ["Opus 5.5"] };

function seats(): SeatsPage {
  return {
    running: 1,
    limit: 4,
    instances: [claude],
    efforts: ["高"],
    permissions: ["只读"],
    templates: [],
    seats: [
      { id: "audit", name: "审计", layer: "direct", term: "long", state: "IDLE", instance: claude,
        allowed: { tune: true, changeInstance: true, remove: true }, model: "Opus 5.5", effort: "高", permission: "只读" },
    ],
  };
}

function facts(overrides: Partial<Design37SideChatPage["chats"][number]> = {}): Design37SideChatPage {
  return {
    domainId: "p1",
    ledgerEpoch: "e1",
    ledgerCursor: "9",
    chats: [
      {
        id: "side-1", title: "实例卡片还能看到报错吗", state: "ACTIVE",
        seatId: "audit", seatIncarnation: "1", sourceSeatId: "lead", sourceSeatIncarnation: "1",
        sourceEpoch: "e1", sourceCursor: "3", syncedCursor: "3", revision: "r1",
        host: { sessionId: "s", generation: "g", expectedRevision: "r", instanceId: "inst-c", driverId: "claude",
          model: "Opus 5.5", answering: false, canAsk: true, questionUnresolved: false },
        messages: [
          { id: "m2", role: "assistant", text: "能。", occurredAt: "2000", sourceEpoch: "e1", sourceCursor: "2" },
          { id: "m1", role: "user", text: "还能看到吗", occurredAt: "1000", sourceEpoch: "e1", sourceCursor: "1" },
        ],
        transfers: [
          { id: "t1", direction: "SIDE_TO_LEAD", sourceSeatId: "audit", targetSeatId: "lead", body: "挪到右上角",
            createdAt: "3000", state: "unknown", reason: "回执未到", nativeReceiptId: "" },
        ],
        ...overrides,
      },
    ],
  };
}

describe("projectSideChats", () => {
  it("orders messages and transfers by host time and keeps an unconfirmed delivery as unknown", () => {
    const page = projectSideChats(facts(), seats());
    const items = page.chats[0].items;
    expect(items.map((item) => item.kind)).toEqual(["user", "answer", "sent-to-lead"]);
    expect(items[2]).toMatchObject({ result: "unknown", error: "回执未到" });
    expect(page.chats[0].model).toBe("Opus 5.5");
    expect(page.chats[0].effort).toBeUndefined();
  });

  it("never fills rounds or segment counts from ledger cursors", () => {
    const page = projectSideChats(facts(), seats());
    expect(page.leadRound).toBeUndefined();
    expect(page.chats[0].referenceRound).toBeUndefined();
    expect(page.chats[0].pendingLeadSegments).toBeUndefined();
  });

  it("marks a removed seat only from a read seats page, and an absent session as unavailable", () => {
    expect(projectSideChats(facts({ seatId: "gone" }), seats()).chats[0].problem).toEqual({ kind: "seat-removed" });
    expect(projectSideChats(facts({ seatId: "gone" }), null).chats[0].problem).toBeUndefined();
    expect(projectSideChats(facts({ host: undefined }), seats()).chats[0].problem).toEqual({ kind: "unavailable" });
  });
});
