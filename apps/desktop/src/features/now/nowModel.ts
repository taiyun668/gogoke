import type { VendorId } from "@/features/instances/instancePageModel";

/**
 * 正在进行: one dispatch from the lead, shown as a work tree pinned to the end
 * of the lead's output (design demo docs/design/demos/gogo-work-tree.html v8,
 * GOGO-整合设计 §四–§七).
 *
 * - A seat line carries no status word: running shows its latest action,
 *   returned shows the first sentence it handed back, failed shows what went wrong.
 * - When the lead closes the batch it folds to one line with one word
 *   (待处理 / 在运行 / 改动就绪, or none); the conversation row shows the same word.
 * - Layer 3 (session, turn, process ids, commit hashes, worktree paths) never
 *   reaches this model.
 */

export type SeatLineState =
  /** Dispatched; no action reported yet. */
  | "starting"
  | "running"
  /** Asked the lead a question; the lead handles it. */
  | "held"
  /** Finished its turn and waits for the lead's next instruction. */
  | "waiting"
  | "returned"
  | "failed"
  /** An audit verdict that no longer applies: the audited work changed after it. */
  | "stale"
  | "stuck"
  /** The dispatch was sent but no receipt came back. */
  | "unknown"
  | "stopping"
  | "stopped"
  /** The seat was removed; its record stays. */
  | "gone";

export type SeatStep = { verb: string; target?: string } | { note: string };

export type SeatLine = {
  /** Host identity; used for keys and actions only, never rendered. */
  id: string;
  name: string;
  vendor: VendorId;
  /** Instance name, model and effort, as the host reports them. */
  who?: string;
  state: SeatLineState;
  /** Latest action while running. */
  lastAction?: { verb: string; target?: string };
  /** The one sentence for the line when not running: the returned first sentence, the question, the failure. */
  say?: string;
  /** For a failure: what to do about it, shown in details. */
  fix?: string;
  /** The lead's instruction, as dispatched. */
  ask: string;
  /** Permission and write area the seat was given. */
  bounds?: string;
  /** For an audit: which version it checked. */
  checks?: string;
  steps: SeatStep[];
  /** What the seat handed back, as plain text lines. */
  report?: string[];
  /** Commit messages of what it handed back; hashes stay in layer 3. */
  commits?: string[];
  diff?: { added: number; removed: number };
  startedAt: number;
  endedAt?: number;
};

export type Landing = { commits: number; merged: boolean };

export type NowBatch = {
  id: string;
  /** Short name of the work, used on the folded line. */
  title: string;
  seats: SeatLine[];
  /** The lead closed this batch: it folds to one line and stays where it is in history. */
  closed: boolean;
  /** Where the handed-back changes are; absent when there are none. */
  landing?: Landing;
};

export const RUNNING: ReadonlySet<SeatLineState> = new Set(["starting", "running", "held", "stopping"]);
const SETTLED: ReadonlySet<SeatLineState> = new Set(["returned", "failed", "stopped", "gone", "stale"]);

/** Header counts, in the order the Owner reads them. */
const COUNT_LABEL: Array<[SeatLineState[], string]> = [
  [["starting", "running"], "处理中"],
  [["held"], "等主控处理"],
  [["waiting"], "等主控派活"],
  [["returned"], "已交回"],
  [["failed"], "失败"],
  [["stale"], "结论要重新核"],
  [["stuck"], "卡住了"],
  [["unknown"], "待核对"],
  [["stopping"], "已请求停止"],
  [["stopped"], "已停止"],
  [["gone"], "找不到了"],
];

export function batchCounts(batch: NowBatch): string {
  return COUNT_LABEL.map(([states, label]) => {
    const count = batch.seats.filter((seat) => states.includes(seat.state)).length;
    return count ? `${count} ${label}` : "";
  })
    .filter(Boolean)
    .join(" · ");
}

export const settledCount = (batch: NowBatch) => batch.seats.filter((seat) => SETTLED.has(seat.state)).length;
export const isRunning = (batch: NowBatch) => batch.seats.some((seat) => RUNNING.has(seat.state));

export function actionCount(batch: NowBatch): number {
  return batch.seats.reduce((sum, seat) => sum + seat.steps.filter((step) => "verb" in step).length, 0);
}

export function totalDiff(batch: NowBatch): { added: number; removed: number } | null {
  const withDiff = batch.seats.filter((seat) => seat.diff);
  if (!withDiff.length) return null;
  return withDiff.reduce(
    (sum, seat) => ({ added: sum.added + seat.diff!.added, removed: sum.removed + seat.diff!.removed }),
    { added: 0, removed: 0 },
  );
}

export type BatchWord = "待处理" | "在运行" | "改动就绪" | null;

/**
 * The one word for the folded line and the conversation row (GOGO-整合设计 §四),
 * first match wins. 待处理 has a single source: a real CLI question waiting.
 */
export function batchWord(batch: NowBatch | null, pendingInput: boolean): BatchWord {
  if (pendingInput) return "待处理";
  if (batch && !batch.closed && batch.seats.some((seat) => seat.state !== "gone" && !SETTLED.has(seat.state))) {
    return "在运行";
  }
  if (batch?.closed && batch.landing && batch.landing.commits > 0) return "改动就绪";
  return null;
}

export function landingText(landing: Landing): string {
  return `${landing.commits} 个提交 · ${landing.merged ? "已合并进主树" : "还在席位的工作树里，没合并"}`;
}

/** What a seat line says. Running lines show the latest action; others show one sentence. */
export function lineText(seat: SeatLine): { verb?: string; target?: string; say?: string } {
  if (seat.state === "running" && seat.lastAction) {
    return { verb: seat.lastAction.verb, target: seat.lastAction.target };
  }
  switch (seat.state) {
    case "starting":
      return { say: "已派出去，等它第一个动作" };
    case "stopping":
      return { say: "已请求停止，等它确认" };
    case "unknown":
      return { say: seat.say ?? "派活发出去了，没收到它的回执" };
    case "gone":
      return { say: seat.say ?? "找不到这个席位了：它已被删除，记录保留" };
    case "waiting":
      return { say: seat.say ?? "这一轮做完了，在等主控下一步" };
    default:
      return { say: seat.say ?? seat.ask };
  }
}

export const STATE_WORD: Record<SeatLineState, string> = {
  starting: "刚派出去",
  running: "运行中",
  held: "等主控处理",
  waiting: "等主控派活",
  returned: "已交回",
  failed: "失败",
  stale: "结论要重新核",
  stuck: "卡住了",
  unknown: "待核对",
  stopping: "已请求停止",
  stopped: "已停止",
  gone: "找不到了",
};

/** Lines shown before "另外 N 个已交回": live and problem lines first. */
export const VISIBLE_LINES = 6;
const ORDER: Record<SeatLineState, number> = {
  running: 0,
  starting: 1,
  held: 2,
  stuck: 3,
  unknown: 4,
  failed: 5,
  stale: 6,
  stopping: 7,
  waiting: 8,
  stopped: 9,
  returned: 10,
  gone: 11,
};

export function visibleSeats(batch: NowBatch, showAll: boolean): SeatLine[] {
  if (showAll || batch.seats.length <= VISIBLE_LINES) return batch.seats;
  return [...batch.seats].sort((a, b) => ORDER[a.state] - ORDER[b.state]).slice(0, VISIBLE_LINES - 1);
}

export function stepTally(seat: SeatLine): Array<{ verb: string; count: number }> {
  const counts = new Map<string, number>();
  for (const step of seat.steps) {
    if ("verb" in step) counts.set(step.verb, (counts.get(step.verb) ?? 0) + 1);
  }
  return [...counts].map(([verb, count]) => ({ verb, count }));
}

export function elapsed(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}
