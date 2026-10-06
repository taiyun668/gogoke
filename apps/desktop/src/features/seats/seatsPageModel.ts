import type { VendorId } from "@/features/instances/instancePageModel";

/**
 * View model for the project's seat page (right side panel).
 *
 * Two layers (design 37 §2b0): 直属席位 are configured by the Owner (at least the
 * lead); 下属席位 are created and scheduled by the lead inside the range the
 * Owner set. The Owner can tune or delete any seat except the lead, which can
 * only be tuned; only 直属席位 can be added from here.
 *
 * Display layers: layer 1 shows name, state, serving instance and one plain
 * line of what it is doing; layer 2 (opened card) shows term, permission, goal,
 * pending question, reclaim condition and recent actions; session and process
 * identifiers are never shown.
 */

export type SeatLayer = "direct" | "sub";
export type SeatTerm = "long" | "short";
export type SeatState =
  | "IDLE"
  | "WORKING"
  | "SWITCHING"
  | "STOP_REQUESTED"
  | "STUCK"
  | "REMOVING"
  | "REMOVED";

export type SeatInstanceRef = { id: string; name: string; vendor: VendorId };

export type SeatRow = {
  /** Host identity; used for actions only. */
  id: string;
  name: string;
  layer: SeatLayer;
  /** The project lead: cannot be deleted. */
  isLead?: boolean;
  term: SeatTerm;
  state: SeatState;
  instance: SeatInstanceRef;
  previousInstance?: SeatInstanceRef;
  model: string;
  effort: string;
  permission: string;
  doing?: string;
  lastActivity?: string;
  stuckFor?: string;
  goal?: string;
  pending?: string;
  reclaimCondition?: string;
  removedNote?: string;
  log?: Array<[string, string]>;
};

export type OrchestrationRange = {
  instanceIds: string[];
  maxPermission: string;
  maxConcurrent: number;
};

export type SeatsPage = {
  running: number;
  /** Concurrent seat limit from local resources. */
  limit: number;
  seats: SeatRow[];
  /** Instances that may serve a seat (logged in and enabled). */
  instances: SeatInstanceRef[];
  models: string[];
  efforts: string[];
  permissions: string[];
  templates: string[];
  range?: OrchestrationRange;
};

export const BUSY_STATES: ReadonlySet<SeatState> = new Set(["WORKING", "STOP_REQUESTED", "SWITCHING"]);
export const RUNNING_STATES: ReadonlySet<SeatState> = new Set(["WORKING", "STOP_REQUESTED", "SWITCHING", "STUCK"]);

export type Tone = "ok" | "warn" | "err" | "busy" | "idle";

export function seatState(row: SeatRow): { tone: Tone; label: string } {
  switch (row.state) {
    case "IDLE":
      return { tone: "idle", label: "空闲" };
    case "WORKING":
      return { tone: "busy", label: "工作中" };
    case "SWITCHING":
      return { tone: "busy", label: "换实例中" };
    case "STOP_REQUESTED":
      return { tone: "warn", label: "已请求停止" };
    case "STUCK":
      return { tone: "err", label: "卡住了" };
    case "REMOVING":
      return { tone: "warn", label: "删除中" };
    case "REMOVED":
      return { tone: "idle", label: "已删除" };
  }
}

/** Layer-1 sentence: what the seat is doing, or what is happening to it. */
export function seatLine(row: SeatRow): string {
  switch (row.state) {
    case "WORKING":
      return row.doing ?? "工作中";
    case "IDLE":
      return row.lastActivity ? `空闲 · 上次：${row.lastActivity}` : "空闲，等主控派活";
    case "SWITCHING":
      return row.previousInstance
        ? `正在从 ${row.previousInstance.name} 换到 ${row.instance.name}`
        : `正在换到 ${row.instance.name}`;
    case "STOP_REQUESTED":
      return "等它确认停下。停下之前，不会往它的工作区派新活";
    case "STUCK":
      return `${row.doing ?? "卡住了"}${row.stuckFor ? ` · 已经 ${row.stuckFor}` : ""}。主控会按升级链处理`;
    case "REMOVING":
      return "等它停下后删除，记录保留";
    case "REMOVED":
      return `已删除${row.removedNote ? ` · ${row.removedNote}` : ""}`;
  }
}

export const canDelete = (row: SeatRow) => !row.isLead && row.state !== "REMOVING" && row.state !== "REMOVED";
export const canTune = (row: SeatRow) => row.state !== "REMOVING" && row.state !== "REMOVED";
/** A busy seat keeps its instance until the current turn ends. */
export const canChangeInstance = (row: SeatRow) => !BUSY_STATES.has(row.state);

export function overview(page: SeatsPage): string {
  const stuck = page.seats.filter((row) => row.state === "STUCK").map((row) => row.name);
  return `${page.running} 个在跑，上限 ${page.limit} 个（按本机资源）${stuck.length ? ` · 卡住：${stuck.join("、")}` : ""}`;
}

export function validateNewSeatName(page: SeatsPage, name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return "给它起个名字";
  if (page.seats.some((row) => row.state !== "REMOVED" && row.name === trimmed)) {
    return "这个项目里已经有叫这个名字的席位了";
  }
  return null;
}
