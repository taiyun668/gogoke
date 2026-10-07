import type { SeatsPage } from "@/features/seats/seatsPageModel";
import {
  createDesign37SideChatSource,
  type Design37SideChatPage,
  type SideChatView,
  type UserSideOperation,
} from "@/services/design37SideChats";
import { createDesign37SeatsSource } from "@/services/tauri";
import type { SideChatSource } from "./SideChatPanel";
import type { SideChat, SideChatPage, SideItem } from "./sideChatModel";

/** A host time (epoch milliseconds or an ISO string) as HH:MM; anything else is shown as given. */
export function clock(value: string): string {
  const ms = /^[0-9]+$/.test(value) ? Number(value) : Date.parse(value);
  if (!Number.isFinite(ms)) return value;
  const date = new Date(ms);
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

function order(value: string): number {
  const ms = /^[0-9]+$/.test(value) ? Number(value) : Date.parse(value);
  return Number.isFinite(ms) ? ms : 0;
}

function chatFrom(view: SideChatView, seats: SeatsPage | null): SideChat {
  const seat = seats?.seats.find((row) => row.id === view.seatId && row.state !== "REMOVED");
  const timeline: Array<{ at: number; item: SideItem }> = [
    ...view.messages.map((message) => ({
      at: order(message.occurredAt),
      item:
        message.role === "user"
          ? ({ kind: "user", id: message.id, text: message.text } as SideItem)
          : ({ kind: "answer", id: message.id, text: message.text, basis: clock(message.occurredAt) } as SideItem),
    })),
    ...view.transfers.map((transfer) => ({
      at: order(transfer.createdAt),
      item:
        transfer.direction === "SIDE_TO_LEAD"
          ? ({
              kind: "sent-to-lead",
              id: transfer.id,
              text: transfer.body,
              at: clock(transfer.createdAt),
              result: transfer.state,
              ...(transfer.reason ? { error: transfer.reason } : {}),
            } as SideItem)
          : ({
              kind: "from-lead",
              id: transfer.id,
              text: transfer.body,
              at: clock(transfer.createdAt),
              delivery: transfer.state,
              ...(transfer.reason ? { error: transfer.reason } : {}),
            } as SideItem),
    })),
  ].sort((a, b) => a.at - b.at);
  const last = [...view.messages.map((m) => m.occurredAt), ...view.transfers.map((t) => t.createdAt)].sort(
    (a, b) => order(a) - order(b),
  ).pop();

  return {
    id: view.id,
    title: view.title,
    seatId: view.seatId,
    instanceId: view.host?.instanceId ?? seat?.instance.id ?? "",
    model: view.host?.model,
    effort: view.host?.effort,
    updatedAt: last ? clock(last) : undefined,
    archived: view.state === "ARCHIVED",
    answering: view.host?.answering ?? false,
    askable: view.host?.canAsk === true,
    questionUnconfirmed: view.host?.questionUnresolved === true,
    items: timeline.map((entry) => entry.item),
    // Only a read seats page can say a seat is gone; an unreadable one says nothing.
    problem: seats && !seat ? { kind: "seat-removed" } : !view.host && view.state === "ACTIVE" ? { kind: "unavailable" } : undefined,
  };
}

/**
 * Project the host's side chat facts. Lead rounds, segment counts and sync mode
 * are not reported by the host, so they stay absent; seat and instance names come
 * from the seats page because the side chat facts carry identities only.
 */
export function projectSideChats(side: Design37SideChatPage, seats: SeatsPage | null): SideChatPage {
  return {
    seats: (seats?.seats ?? [])
      .filter((row) => row.layer === "direct" && !row.isLead && row.state !== "REMOVED")
      .map((row) => ({ id: row.id, name: row.name, defaultInstanceId: row.instance.id, permission: row.permission })),
    instances: (seats?.instances ?? []).map((item) => ({
      id: item.id,
      name: item.name,
      vendor: item.vendor,
      models: item.models ?? [],
    })),
    efforts: seats?.efforts ?? [],
    chats: side.chats.map((view) => chatFrom(view, seats)),
  };
}

/** Side chat panel source on the host bridge; only the operations the bridge has are passed on. */
export function createDesign37SideChatPanelSource(domainId: string, execute: UserSideOperation): SideChatSource {
  const side = createDesign37SideChatSource(domainId, execute);
  const seats = createDesign37SeatsSource<SeatsPage>(domainId);
  return {
    read: async () => {
      const facts = await side.read();
      let seatPage: SeatsPage | null = null;
      try {
        seatPage = await seats.read();
      } catch {
        seatPage = null;
      }
      return projectSideChats(facts, seatPage);
    },
    actions: {
      ask: side.actions.ask,
      stop: side.actions.stop,
      archive: side.actions.archive,
      restore: side.actions.restore,
      remove: side.actions.remove,
    },
  };
}
