import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import {
  PopoverMenuItem,
  PopoverSurface,
} from "@/features/design-system/components/popover/PopoverPrimitives";
import { VENDOR_ICONS } from "@/features/instances/vendorIcons";
import {
  RUNNING,
  STATE_WORD,
  actionCount,
  batchCounts,
  batchWord,
  elapsed,
  isRunning,
  landingText,
  lineText,
  settledCount,
  stepTally,
  totalDiff,
  visibleSeats,
  type NowBatch,
  type SeatLine,
  type SeatLineState,
} from "./nowModel";
import "./now.css";

/** Host operations; a control is rendered only when its operation exists. */
export type NowActions = {
  /** Open the seat's handed-back commits in the Git panel. */
  openInGit?: (seatId: string) => void;
  /** Open the seat's full record, read-only. */
  openRecord?: (seatId: string) => void;
  /** Merge the batch's changes into the main tree. */
  merge?: (batchId: string) => void;
};

const BOT = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden>
    <rect x="4" y="8" width="16" height="12" rx="3" />
    <path d="M12 4v4M9 14h.01M15 14h.01" />
  </svg>
);
const CARET = (
  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
    <path d="M6 9l6 6 6-6" />
  </svg>
);
const CHEV = (
  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
    <path d="M9 6l6 6-6 6" />
  </svg>
);

function Glyph({ state }: { state: SeatLineState }) {
  const stroke = { fill: "none", stroke: "currentColor", strokeWidth: 2.3, strokeLinecap: "round" as const };
  switch (state) {
    case "starting":
    case "running":
      return <span className="working-spinner" aria-hidden />;
    case "returned":
      return (
        <svg viewBox="0 0 24 24" {...stroke} aria-hidden>
          <path d="M6 12.5l4 4L18 8" />
        </svg>
      );
    case "failed":
      return (
        <svg viewBox="0 0 24 24" {...stroke} aria-hidden>
          <path d="M8 8l8 8M16 8l-8 8" />
        </svg>
      );
    case "held":
      return (
        <svg viewBox="0 0 24 24" {...stroke} strokeLinejoin="round" aria-hidden>
          <path d="M5 12h9M11 7l5 5-5 5" />
        </svg>
      );
    case "stale":
      return (
        <svg viewBox="0 0 24 24" {...stroke} strokeLinejoin="round" aria-hidden>
          <path d="M19 12a7 7 0 1 1-2.05-4.95M19 5v4h-4" />
        </svg>
      );
    case "stuck":
      return (
        <svg viewBox="0 0 24 24" {...stroke} aria-hidden>
          <path d="M9 7v10M15 7v10" />
        </svg>
      );
    case "waiting":
      return (
        <svg viewBox="0 0 24 24" {...stroke} aria-hidden>
          <path d="M7 12h.01M12 12h.01M17 12h.01" />
        </svg>
      );
    case "stopping":
    case "stopped":
      return (
        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
          <rect x="8" y="8" width="8" height="8" rx="1.5" opacity={state === "stopping" ? 0.55 : 1} />
        </svg>
      );
    case "gone":
      return (
        <svg viewBox="0 0 24 24" {...stroke} aria-hidden>
          <path d="M6 12h12" />
        </svg>
      );
    case "unknown":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeDasharray="3 3" aria-hidden>
          <circle cx="12" cy="12" r="8" />
        </svg>
      );
  }
}

function Diff({ diff }: { diff: { added: number; removed: number } }) {
  return (
    <span className="now-diff" aria-label={`增加 ${diff.added} 行，删除 ${diff.removed} 行`}>
      <b>+{diff.added}</b> <i>−{diff.removed}</i>
    </span>
  );
}

function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return undefined;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

type Wire = { d: string; x2: number; y2: number; state: SeatLineState };

/**
 * The lead's current dispatch, pinned as the last block of the lead's output.
 * Open while it runs; once the lead closes it, it folds to one line that stays
 * where it is in history.
 */
export function NowBlock({
  batch,
  actions = {},
  frozenAt,
}: {
  batch: NowBatch;
  actions?: NowActions;
  /** Set when the host cannot be read: the time of the last read, shown instead of live state. */
  frozenAt?: string;
}) {
  const [open, setOpen] = useState(!batch.closed);
  const [selected, setSelected] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);
  const touched = useRef(false);
  const running = isRunning(batch) && !frozenAt;
  const now = useNow(running);

  // Follow the batch closing unless the Owner already chose open or closed.
  useEffect(() => {
    if (!touched.current) setOpen(!batch.closed);
  }, [batch.closed]);

  const toggle = () => {
    touched.current = true;
    setOpen((value) => !value);
  };

  const word = batchWord(batch, false);
  const diff = totalDiff(batch);

  if (batch.closed && !open) {
    return (
      <div className="now is-folded" data-batch={batch.id}>
        <div className="now-head">
          <span className="now-title">
            {BOT}
            {batch.title}
          </span>
          <span className="now-counts">
            {word ? <span className={`now-word${word === "改动就绪" ? " is-ready" : ""}`}>{word}</span> : null}
            {batch.landing ? <> · {landingText(batch.landing)}</> : null}
            {diff ? <> · <Diff diff={diff} /></> : null}
          </span>
          {batch.landing && !batch.landing.merged && actions.merge ? (
            <button type="button" className="ghost now-act" onClick={() => actions.merge!(batch.id)}>
              并进来
            </button>
          ) : null}
          <button type="button" className="now-link now-quiet" aria-expanded={false} onClick={toggle}>
            展开
          </button>
        </div>
      </div>
    );
  }

  const started = Math.min(...batch.seats.map((seat) => seat.startedAt));
  const ended = Math.max(0, ...batch.seats.map((seat) => seat.endedAt ?? 0));
  const clock = Number.isFinite(started) ? elapsed((running ? now : ended || now) - started) : "";
  const actionsDone = actionCount(batch);
  const settled = settledCount(batch);
  const seats = visibleSeats(batch, showAll);
  const hidden = batch.seats.length - seats.length;
  const current = batch.seats.find((seat) => seat.id === selected) ?? null;

  return (
    <div className={`now${open ? " is-open" : ""}${frozenAt ? " is-frozen" : ""}`} data-batch={batch.id}>
      <button type="button" className="now-head" aria-expanded={open} onClick={toggle}>
        <span className="now-title">
          {BOT}
          {batch.closed ? batch.title : "正在进行"}
        </span>
        <span className="now-counts">{batchCounts(batch)}</span>
        <span className="now-summary">{[actionsDone ? `${actionsDone} 个动作` : "", clock].filter(Boolean).join(" · ")}</span>
        <span className="now-caret">{CARET}</span>
      </button>
      <div
        className="now-progress"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={batch.seats.length}
        aria-valuenow={settled}
        aria-label={`${settled} / ${batch.seats.length} 个席位已收口`}
      >
        <i style={{ transform: `scaleX(${batch.seats.length ? settled / batch.seats.length : 0})` }} />
      </div>
      {frozenAt ? <div className="now-frozen">读不到宿主了。下面是 {frozenAt} 最后一次读到的样子，之后的情况不知道。</div> : null}
      {open ? (
        <>
          <Graph seats={seats} running={running} selected={selected} onSelect={(id) => setSelected((value) => (value === id ? null : id))} />
          {hidden > 0 ? (
            <div className="now-more">
              <button type="button" className="now-link" onClick={() => setShowAll(true)}>
                另外 {hidden} 个已交回，点开看
              </button>
            </div>
          ) : null}
          {current ? <Panel seat={current} now={now} actions={actions} /> : null}
        </>
      ) : null}
    </div>
  );
}

function Graph({
  seats,
  running,
  selected,
  onSelect,
}: {
  seats: SeatLine[];
  running: boolean;
  selected: string | null;
  onSelect: (id: string) => void;
}) {
  const graphRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const nodes = useRef(new Map<string, HTMLElement>());
  const [wires, setWires] = useState<Wire[]>([]);
  const [port, setPort] = useState<{ x: number; y: number } | null>(null);

  // Wires are measured from the rendered rows, so wrapped text and fonts cannot skew them.
  useLayoutEffect(() => {
    const graph = graphRef.current;
    if (!graph) return undefined;
    const measure = () => {
      const root = rootRef.current;
      const box = graph.getBoundingClientRect();
      const from = root?.getBoundingClientRect();
      if (!from || !from.width || !box.width) {
        setWires([]);
        setPort(null);
        return;
      }
      const x1 = from.right - box.left;
      const y1 = from.top + from.height / 2 - box.top;
      setPort({ x: x1, y: y1 });
      setWires(
        seats.flatMap((seat) => {
          const el = nodes.current.get(seat.id);
          if (!el) return [];
          const to = el.getBoundingClientRect();
          const x2 = to.left - box.left + 2;
          const y2 = to.top + to.height / 2 - box.top;
          const bend = Math.max((x2 - x1) / 2, 8);
          return [{ d: `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`, x2, y2, state: seat.state }];
        }),
      );
    };
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(graph);
    return () => observer.disconnect();
  }, [seats]);

  return (
    <div className="now-graph" ref={graphRef}>
      <div className="now-root" ref={rootRef} data-running={running}>
        {BOT}主控
      </div>
      <svg className="now-wires" aria-hidden>
        {wires.map((wire, index) => (
          <g key={index} data-s={wire.state}>
            <path className="now-wire" d={wire.d} />
            {running && RUNNING.has(wire.state) ? <path className="now-flow" d={wire.d} /> : null}
            <circle className="now-end" cx={wire.x2} cy={wire.y2} r={2} />
          </g>
        ))}
        {port ? <circle className="now-port" cx={port.x} cy={port.y} r={2.5} /> : null}
      </svg>
      <div className="now-nodes">
        {seats.map((seat) => {
          const line = lineText(seat);
          return (
            <button
              key={seat.id}
              type="button"
              className="now-node"
              data-s={seat.state}
              aria-expanded={selected === seat.id}
              aria-label={`${seat.name} ${STATE_WORD[seat.state]}`}
              title={seat.ask}
              ref={(el) => {
                if (el) nodes.current.set(seat.id, el);
                else nodes.current.delete(seat.id);
              }}
              onClick={() => onSelect(seat.id)}
            >
              <span className="now-avatar">
                <Glyph state={seat.state} />
              </span>
              <span className="now-name">{seat.name}</span>
              <span
                className="now-vendor"
                style={{ WebkitMaskImage: `url(${VENDOR_ICONS[seat.vendor]})`, maskImage: `url(${VENDOR_ICONS[seat.vendor]})` }}
                aria-hidden
              />
              <span className="now-line">
                {line.verb ? (
                  <>
                    <span className="now-verb">{line.verb}</span>
                    {line.target ? <span className="now-target">{line.target}</span> : null}
                  </>
                ) : (
                  <span className="now-say">{line.say}</span>
                )}
              </span>
              {seat.diff && seat.state !== "running" ? <Diff diff={seat.diff} /> : null}
              <span className="now-chev">{CHEV}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function Panel({ seat, now, actions }: { seat: SeatLine; now: number; actions: NowActions }) {
  const tally = stepTally(seat);
  const time = elapsed((seat.endedAt ?? now) - seat.startedAt);
  // Without a report, the latest eight steps are enough to see what it is doing.
  const steps = seat.report ? seat.steps : seat.steps.slice(-8);
  const facts: Array<[string, ReactNode]> = [
    ["谁接的", seat.who ? `${seat.name} · ${seat.who}` : seat.name],
    ["派的活", seat.ask],
  ];
  if (seat.bounds) facts.push(["边界", seat.bounds]);
  if (seat.checks) facts.push(["核对的版本", <span className={seat.state === "stale" ? "now-warn" : undefined}>{seat.checks}</span>]);

  return (
    <div className="now-panel">
      <div className="now-panel-head">
        <span className="now-panel-title">
          {seat.name} <em>{STATE_WORD[seat.state]}</em>
        </span>
        <span className="now-steps">
          {tally.map((item) => (
            <span key={item.verb} className="now-step" data-write={["改", "写", "提交"].includes(item.verb)}>
              {item.verb} {item.count}
            </span>
          ))}
        </span>
        <span className="now-panel-time">{time}</span>
      </div>
      <dl className="now-facts">
        {facts.map(([label, value]) => (
          <div key={label} className="now-fact">
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      {seat.state === "failed" && seat.say ? (
        <p className="now-fail">
          {seat.say}
          {seat.fix ? <span>{seat.fix}</span> : null}
        </p>
      ) : null}
      {steps.length ? (
        <div className="now-log">
          {steps.map((step, index) =>
            "verb" in step ? (
              <div key={index} className="now-log-line">
                <span className="now-log-verb">{step.verb}</span>
                {step.target ? <span className="now-log-target">{step.target}</span> : null}
              </div>
            ) : (
              <div key={index} className="now-log-line now-log-note">
                {step.note}
              </div>
            ),
          )}
        </div>
      ) : null}
      {seat.report?.length || seat.commits?.length ? (
        <div className="now-report">
          {seat.report?.map((line, index) => <div key={index}>{line}</div>)}
          {seat.commits?.length ? (
            <ul>
              {seat.commits.map((message, index) => (
                <li key={index}>{message}</li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
      {(seat.commits?.length && actions.openInGit) || actions.openRecord ? (
        <div className="now-panel-links">
          {seat.commits?.length && actions.openInGit ? (
            <button type="button" className="now-link" onClick={() => actions.openInGit!(seat.id)}>
              在 Git 里看这些提交
            </button>
          ) : null}
          {actions.openRecord ? (
            <button type="button" className="now-link" onClick={() => actions.openRecord!(seat.id)}>
              看它的完整记录（只读）
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/** One line above the composer, shown only while the block is out of view. */
export function NowPin({ batch, onJump }: { batch: NowBatch; onJump: () => void }) {
  const running = isRunning(batch);
  return (
    <button type="button" className="now-pin" onClick={onJump}>
      {running ? <span className="working-spinner" aria-hidden /> : null}
      <b>正在进行</b>
      <span className="now-pin-counts">{batchCounts(batch)}</span>
      <span className="now-pin-jump">回到这里 ↓</span>
    </button>
  );
}

/** Reports whether an element is in view, for showing NowPin only when the block is not. */
export function useInView(ref: RefObject<Element | null>): boolean {
  const [inView, setInView] = useState(true);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") return undefined;
    const observer = new IntersectionObserver(([entry]) => setInView(entry.isIntersecting));
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return inView;
}

/**
 * The composer's stop control while something runs: stop the lead's turn, or
 * stop the whole work (lead and dispatched seats). Only offered kinds render;
 * the result is reported by the host ("已请求停止" first, "已停止" on the fact).
 */
export function StopMenu({
  onStopTurn,
  onStopWork,
  disabled = false,
}: {
  onStopTurn?: () => void;
  onStopWork?: () => void;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return undefined;
    const close = (event: MouseEvent) => {
      if (!anchor.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);

  if (!onStopTurn && !onStopWork) return null;
  const pick = (fn: () => void) => () => {
    setOpen(false);
    fn();
  };

  return (
    <span className="now-stop" ref={anchor}>
      <button
        type="button"
        className="ghost"
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        停止
      </button>
      {open ? (
        <PopoverSurface className="now-stop-menu" role="menu">
          {onStopTurn ? (
            <PopoverMenuItem role="menuitem" onClick={pick(onStopTurn)}>
              停这一轮
              <small>只停主控现在这一轮，席位接着干</small>
            </PopoverMenuItem>
          ) : null}
          {onStopWork ? (
            <PopoverMenuItem role="menuitem" onClick={pick(onStopWork)}>
              停这件事
              <small>主控和派出去的席位都停；改了的不退回</small>
            </PopoverMenuItem>
          ) : null}
        </PopoverSurface>
      ) : null}
    </span>
  );
}

