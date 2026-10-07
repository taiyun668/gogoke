/**
 * 秘书长: the one global seat that serves the Owner across projects
 * (design 37 §7; GOGO-要做的和不做的 §八, Owner 2026-10-07).
 *
 * - One long conversation, same window as any work; what is its own lives in
 *   the right side panel (routines, settings).
 * - It never pops up: its left-column entry carries the state and a count.
 * - Every action it takes in a project leaves one line in its conversation.
 * - A project's own question stays in that project; the secretary only points to it.
 * Every value here is the host's; absent facts are hidden, never filled.
 */

export type EntryState =
  | { kind: "quiet"; lastActivity?: string }
  | { kind: "waiting"; count: number }
  | { kind: "working"; doing?: string }
  /** No instance chosen yet; it cannot work. */
  | { kind: "unset" }
  /** Its instance cannot be used now. */
  | { kind: "down"; reason: string; recoversAt?: string };

export type Tone = "idle" | "unread" | "processing" | "warn";

export function entryLine(state: EntryState): { tone: Tone; text: string } {
  switch (state.kind) {
    case "quiet":
      return { tone: "idle", text: state.lastActivity ? `最近：${state.lastActivity}` : "没有在做的事" };
    case "waiting":
      return { tone: "unread", text: `${state.count} 件等你` };
    case "working":
      return { tone: "processing", text: state.doing ?? "在做事" };
    case "unset":
      return { tone: "warn", text: "还没设置：选一个实例它才能干活" };
    case "down":
      return { tone: "warn", text: `用不了：${state.reason}${state.recoversAt ? `，${state.recoversAt} 恢复` : ""}` };
  }
}

export type Delivery = "steered" | "new-turn" | "failed" | "unknown";

/** One line in its conversation for one thing it did or received in a project. */
export type ActionLine = {
  id: string;
  kind:
    /** It sent something to a seat in a project. */
    | "sent"
    /** A seat in a project answered it. */
    | "received"
    /** It opened a new work in a project. */
    | "opened-work"
    /** A project has something for the Owner; the secretary only points to it. */
    | "pointer"
    /** One of its routines ran. */
    | "routine";
  /** Plain text: "gogoke 的主控", "短视频". */
  target?: string;
  text: string;
  delivery?: Delivery;
  /** What was actually sent or received, verbatim; layer 2. */
  verbatim?: string;
  at?: string;
  error?: string;
  /** The host can open the work this line refers to. */
  canOpen?: boolean;
};

export function deliveryText(delivery: Delivery | undefined): string | null {
  switch (delivery) {
    case "steered":
      return "插进了它正在跑的这一轮";
    case "new-turn":
      return "它开始了新的一轮";
    case "failed":
      return "没送到";
    case "unknown":
      return "未能确认送达";
    default:
      return null;
  }
}

export const ACTION_MARK: Record<ActionLine["kind"], string> = {
  sent: "↗",
  received: "↙",
  "opened-work": "＋",
  pointer: "→",
  routine: "⏱",
};

export type Routine = {
  id: string;
  name: string;
  /** The host's own words for when it runs, e.g. "每天 9:00". */
  schedule: string;
  nextRun?: string;
  lastRun?: { at: string; ok: boolean; note?: string };
  paused: boolean;
};

export type SettingsChoice = { id: string; name: string; models?: string[] };

export type SecretarySettings = {
  instanceId?: string;
  model?: string;
  effort?: string;
  permission?: string;
  instances: SettingsChoice[];
  efforts: string[];
  permissions: string[];
  /** What it can and cannot do under the current configuration, as the host states it. */
  can?: string[];
  cannot?: string[];
};

export type SecretaryPage = {
  entry: EntryState;
  routines: Routine[];
  /** Routines were paused because the Owner was away; the host says for how long. */
  pausedWhileAway?: { awayFor: string };
  settings: SecretarySettings;
};

export function routineLine(routine: Routine): string {
  return routine.paused ? `${routine.schedule} · 已暂停` : `${routine.schedule}${routine.nextRun ? ` · 下次 ${routine.nextRun}` : ""}`;
}

export function lastRunLine(routine: Routine): { text: string; failed: boolean } {
  if (!routine.lastRun) return { text: "还没跑过", failed: false };
  if (routine.lastRun.ok) return { text: `上次 ${routine.lastRun.at} · 跑完了`, failed: false };
  return { text: `上次 ${routine.lastRun.at} · ${routine.lastRun.note ?? "没跑成，宿主没给出原因"}`, failed: true };
}

/** Models offered are the chosen instance's verified models only. */
export const modelsFor = (settings: SecretarySettings, instanceId: string | undefined) =>
  settings.instances.find((item) => item.id === instanceId)?.models ?? [];
