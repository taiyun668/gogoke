import type { VendorId } from "@/features/instances/instancePageModel";

/**
 * Side chat (旁聊) in the right side panel.
 *
 * Mechanism follows the Codex desktop side chat: a side chat forks the lead's
 * context as reference only, and windows pass messages to each other in natural
 * language (the seat sends to the lead when the Owner asks; the lead can send
 * back). There are no relay buttons: each window shows one line recording the
 * send ("已发给主控" / "来自旁聊 · X") and what the host confirmed about it.
 *
 * Differences from Codex (Owner 2026-09-26): side chats are kept by default
 * (archive, then delete), and the lead's later progress keeps syncing in as
 * reference.
 *
 * Every count, round, model and time is the host's; absent facts are hidden,
 * never filled from ledger cursors or differences.
 */

export type SideSeat = {
  id: string;
  name: string;
  /** Instance the seat normally uses; preselected for a new side chat. */
  defaultInstanceId: string;
  permission: string;
};

/** `models` are the host's verified models for this instance; empty when there are none. */
export type SideInstance = { id: string; name: string; vendor: VendorId; models: string[] };

/** What the host confirmed about one message between windows. */
export type Delivery = "steered" | "new-turn" | "failed" | "unknown";

export type SideItem =
  | { kind: "user"; id: string; text: string }
  | {
      kind: "answer";
      id: string;
      text: string;
      /** When the answer started; the context it is based on. */
      basis?: string;
      /** The lead moved on after this answer started. */
      leadUpdatedAfter?: boolean;
    }
  | {
      kind: "sent-to-lead";
      id: string;
      text: string;
      at: string;
      result: Delivery;
      error?: string;
    }
  | { kind: "from-lead"; id: string; text: string; at: string; delivery: Delivery; error?: string };

export type SideChatProblem =
  | { kind: "answer-failed"; summary: string; raw?: string }
  | { kind: "instance-full"; instanceName: string; used: number; cap: number }
  | { kind: "seat-removed"; seatName?: string }
  /** The chat's session is not reachable now; it can be read but not asked. */
  | { kind: "unavailable" };

export type SideChat = {
  id: string;
  title: string;
  seatId: string;
  instanceId: string;
  model?: string;
  effort?: string;
  /** Lead round the chat currently references, when the host reports one. */
  referenceRound?: number;
  /** Lead segments not yet delivered to this chat, when the host reports them. */
  pendingLeadSegments?: number;
  /** The vendor can append without starting a turn. */
  appendsImmediately?: boolean;
  updatedAt?: string;
  archived: boolean;
  /** From the host's actual turn state. */
  answering: boolean;
  /** The host can take a question now; false hides nothing but disables asking. */
  askable?: boolean;
  /** The previous question's delivery is not confirmed yet. */
  questionUnconfirmed?: boolean;
  items: SideItem[];
  problem?: SideChatProblem;
};

export type SideChatPage = {
  /** Lead round a new side chat would reference, when the host reports one. */
  leadRound?: number;
  /** False when the seats page could not be read: a missing seat is then unknown, not removed. */
  seatsKnown?: boolean;
  seats: SideSeat[];
  /** Instances a new side chat may use. */
  instances: SideInstance[];
  /** Further instances known by name, for showing existing chats only; never offered for a new chat. */
  knownInstances?: SideInstance[];
  efforts: string[];
  chats: SideChat[];
};

export const seatById = (page: SideChatPage, id: string) => page.seats.find((seat) => seat.id === id);
export const instanceById = (page: SideChatPage, id: string) =>
  page.instances.find((item) => item.id === id) ?? page.knownInstances?.find((item) => item.id === id);

export function syncLine(chat: SideChat): string | null {
  if (chat.archived || !chat.pendingLeadSegments) return null;
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
    case "unknown":
      return "发给主控，未能确认送达";
  }
}

export function fromLeadLabel(item: Extract<SideItem, { kind: "from-lead" }>): string {
  switch (item.delivery) {
    case "failed":
      return `主控发来的，没送到这里 · ${item.at}`;
    case "unknown":
      return `主控发来的，未能确认送达 · ${item.at}`;
    default:
      return `来自主控 · ${item.at}`;
  }
}

/** Seat and instance are fixed once the first question is sent; model and effort stay adjustable. */
export const chatStarted = (chat: SideChat | null) => chat !== null && chat.items.length > 0;

export function canAsk(chat: SideChat): boolean {
  return (
    !chat.archived &&
    chat.askable !== false &&
    chat.problem?.kind !== "seat-removed" &&
    chat.problem?.kind !== "instance-full" &&
    chat.problem?.kind !== "unavailable"
  );
}

/** The previous question's delivery is unconfirmed: only that same sentence may be sent again. */
export function canRetry(chat: SideChat): boolean {
  return chat.questionUnconfirmed === true && !chat.archived && !chat.problem;
}

/** The current instance's first verified model, or none: never the previous instance's. */
export const defaultModel = (instance: SideInstance | undefined): string | undefined => instance?.models[0];
