import type { VendorId } from "@/features/instances/instancePageModel";

/**
 * Side chat (旁聊) in the right side panel.
 *
 * Mechanism follows the Codex desktop side chat: a side chat forks the lead's
 * context as reference only, and windows pass messages to each other in natural
 * language (the seat sends to the lead when the Owner asks; the lead can send
 * back). There are no relay buttons: each window shows one line recording the
 * send ("已发给主控" / "来自旁聊 · X") and whether it arrived.
 *
 * Differences from Codex (Owner 2026-09-26): side chats are kept by default
 * (archive, then delete), and the lead's later progress keeps syncing in as
 * reference, delivered with the Owner's next question unless the vendor can
 * append without starting a turn.
 */

export type SideSeat = {
  id: string;
  name: string;
  /** Instance the seat normally uses; preselected for a new side chat. */
  defaultInstanceId: string;
  permission: string;
};

export type SideInstance = { id: string; name: string; vendor: VendorId; models: string[] };

export type SideItem =
  | { kind: "user"; id: string; text: string }
  | {
      kind: "answer";
      id: string;
      text: string;
      /** When the answer started; the context it is based on. */
      basis: string;
      /** The lead moved on after this answer started. */
      leadUpdatedAfter?: boolean;
    }
  | {
      kind: "sent-to-lead";
      id: string;
      text: string;
      at: string;
      result: "steered" | "new-turn" | "failed";
      error?: string;
    }
  | { kind: "from-lead"; id: string; text: string; at: string };

export type SideChatProblem =
  | { kind: "answer-failed"; summary: string; raw?: string }
  | { kind: "instance-full"; instanceName: string; used: number; cap: number }
  | { kind: "seat-removed"; seatName: string };

export type SideChat = {
  id: string;
  title: string;
  seatId: string;
  instanceId: string;
  model: string;
  effort: string;
  /** Lead round the chat currently references. */
  referenceRound: number;
  /** Lead segments not yet delivered to this chat. */
  pendingLeadSegments: number;
  /** The vendor can append without starting a turn. */
  appendsImmediately: boolean;
  updatedAt: string;
  archived: boolean;
  answering: boolean;
  items: SideItem[];
  problem?: SideChatProblem;
};

export type SideChatPage = {
  /** Lead round a new side chat would reference. */
  leadRound: number;
  seats: SideSeat[];
  instances: SideInstance[];
  efforts: string[];
  chats: SideChat[];
};

export const seatById = (page: SideChatPage, id: string) => page.seats.find((seat) => seat.id === id);
export const instanceById = (page: SideChatPage, id: string) => page.instances.find((item) => item.id === id);

export function syncLine(chat: SideChat): string | null {
  if (chat.archived || chat.pendingLeadSegments === 0) return null;
  return chat.appendsImmediately
    ? `主控的 ${chat.pendingLeadSegments} 段新进展已同步过来`
    : `主控有 ${chat.pendingLeadSegments} 段新进展，下次提问时一起带上`;
}

export function sentLine(item: Extract<SideItem, { kind: "sent-to-lead" }>): string {
  switch (item.result) {
    case "steered":
      return "已发给主控 · 插进了主控正在跑的这一轮";
    case "new-turn":
      return "已发给主控 · 主控开始了新的一轮";
    case "failed":
      return "发给主控，没送到";
  }
}

/** Seat and instance are fixed once the first question is sent; model and effort stay adjustable. */
export const chatStarted = (chat: SideChat | null) => chat !== null && chat.items.length > 0;

export function canAsk(chat: SideChat): boolean {
  return !chat.archived && chat.problem?.kind !== "seat-removed" && chat.problem?.kind !== "instance-full";
}
