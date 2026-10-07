import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ACTION_MARK,
  deliveryText,
  entryLine,
  lastRunLine,
  modelsFor,
  routineLine,
  type ActionLine,
  type EntryState,
  type Routine,
  type SecretaryPage,
  type SecretarySettings,
} from "./secretaryModel";
import "./secretary.css";

const ICON = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="M4 7h16M4 12h10M4 17h7" />
    <circle cx="18" cy="16" r="3" />
  </svg>
);

const DOT: Record<string, string> = { unread: "unread", processing: "processing" };

/** The one fixed row at the top of the left column. It never pops anything up. */
export function SecretaryEntry({ state, active = false, onOpen }: { state: EntryState; active?: boolean; onOpen: () => void }) {
  const line = entryLine(state);
  return (
    <button type="button" className={`sec-entry${active ? " is-active" : ""}`} aria-current={active ? "page" : undefined} onClick={onOpen}>
      <span className="sec-avatar">
        {ICON}
        {DOT[line.tone] ? <span className={`thread-status ${DOT[line.tone]}`} aria-hidden /> : null}
      </span>
      <span className="sec-entry-main">
        <span className="sec-entry-name">秘书长</span>
        <span className={`sec-entry-sub${line.tone === "warn" ? " is-warn" : ""}`}>{line.text}</span>
      </span>
    </button>
  );
}

/** One line in its conversation for one action in a project; the verbatim text is one click away. */
export function SecretaryActionLine({ line, onOpen }: { line: ActionLine; onOpen?: (id: string) => void }) {
  const [open, setOpen] = useState(false);
  const delivery = deliveryText(line.delivery);
  const unsure = line.delivery === "unknown" || line.delivery === "failed";
  return (
    <div className={`sec-act${unsure ? " is-unsure" : ""}`}>
      <button type="button" className="sec-act-main" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
        <span className="sec-act-mark" aria-hidden>
          {ACTION_MARK[line.kind]}
        </span>
        <span>
          {line.target ? <b>{line.target}</b> : null}
          {line.target ? "：" : ""}
          {line.text}
          {delivery ? <span className="sec-act-res"> · {delivery}</span> : null}
        </span>
      </button>
      {line.canOpen && onOpen ? (
        <button type="button" className="sec-act-open" onClick={() => onOpen(line.id)}>
          {line.kind === "pointer" ? "去回答 →" : "打开 →"}
        </button>
      ) : null}
      {open ? (
        <div className="sec-act-detail">
          {line.verbatim ? <>“{line.verbatim}”</> : "原话没有记下来。"}
          {line.error ? <span className="sec-error"> · {line.error}</span> : null}
          {line.at ? <span className="sec-help"> · {line.at}</span> : null}
        </div>
      ) : null}
    </div>
  );
}

/** Host operations; a control is shown only when its operation exists. */
export type SecretaryActions = {
  pauseRoutine?: (id: string) => Promise<void>;
  resumeRoutine?: (id: string) => Promise<void>;
  deleteRoutine?: (id: string) => Promise<void>;
  resumeAll?: () => Promise<void>;
  openRoutineRecord?: (id: string) => void;
  saveSettings?: (input: { instanceId: string; model?: string; effort?: string; permission?: string }) => Promise<void>;
};

export type SecretarySource = {
  /** Null while the host has no secretary read model. */
  read: () => Promise<SecretaryPage | null>;
  actions: SecretaryActions;
};

type Tab = "routines" | "settings";
type RunFn = (key: string, operation: () => Promise<void>) => Promise<boolean>;

function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

const TAB_ICON: Record<Tab, ReactNode> = {
  routines: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 8v4l3 2" />
    </svg>
  ),
  settings: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden>
      <path d="M4 7h10M18 7h2M4 17h4M12 17h8" />
      <circle cx="16" cy="7" r="2" />
      <circle cx="10" cy="17" r="2" />
    </svg>
  ),
};
const TAB_NAME: Record<Tab, string> = { routines: "定时任务", settings: "设置" };

/** The secretary's right side panel: its own things, where a work keeps Git, side chat and seats. */
export function SecretaryPanel({ source, initialTab }: { source: SecretarySource; initialTab?: Tab }) {
  const [page, setPage] = useState<SecretaryPage | null | undefined>(undefined);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>(initialTab ?? "routines");
  const busyRef = useRef(false);
  const readSeq = useRef(0);
  const epoch = useRef(0);
  const sourceRef = useRef(source);

  const refresh = async () => {
    const mine = ++readSeq.current;
    const era = epoch.current;
    try {
      const next = await sourceRef.current.read();
      if (mine !== readSeq.current || era !== epoch.current) return;
      setPage(next);
      setLoadError(null);
    } catch (cause) {
      if (mine === readSeq.current && era === epoch.current) setLoadError(errorText(cause));
    }
  };

  useEffect(() => {
    epoch.current += 1;
    sourceRef.current = source;
    busyRef.current = false;
    setBusy(null);
    setPage(undefined);
    setLoadError(null);
    setActionError(null);
    void refresh();
    const timer = window.setInterval(() => {
      if (!busyRef.current) void refresh();
    }, 2000);
    return () => {
      window.clearInterval(timer);
      epoch.current += 1;
    };
  }, [source]);

  const run: RunFn = async (key, operation) => {
    if (busyRef.current) return false;
    busyRef.current = true;
    const era = epoch.current;
    setBusy(key);
    setActionError(null);
    let ok = true;
    try {
      await operation();
    } catch (cause) {
      ok = false;
      if (era === epoch.current) setActionError(errorText(cause));
    } finally {
      if (era === epoch.current) {
        busyRef.current = false;
        setBusy(null);
        void refresh();
      }
    }
    return ok && era === epoch.current;
  };

  // Rows from an earlier read stay visible after a failed read, but nothing can be done to them.
  const locked = busy !== null || (page !== null && page !== undefined && loadError !== null);

  return (
    <div className="sec-panel">
      <div className="sec-panel-head">
        <div className="panel-tabs" role="tablist" aria-label="秘书长面板">
          {(["routines", "settings"] as Tab[]).map((id) => (
            <button
              key={id}
              type="button"
              role="tab"
              className={`panel-tab${id === tab ? " is-active" : ""}`}
              aria-selected={id === tab}
              aria-label={TAB_NAME[id]}
              title={TAB_NAME[id]}
              onClick={() => setTab(id)}
            >
              <span className="panel-tab-icon">{TAB_ICON[id]}</span>
            </button>
          ))}
        </div>
        <span className="sec-panel-title">{TAB_NAME[tab]}</span>
      </div>
      {page === undefined && !loadError ? <div className="sec-help" role="status">正在读取…</div> : null}
      {loadError ? (
        <div className="sec-help sec-error" role="alert">
          {page ? `读不到最新状态：${loadError}。下面是上次读到的，现在不能操作。` : `读不到秘书长：${loadError}`}
        </div>
      ) : null}
      {page === null ? <div className="sec-help">秘书长的数据还没接上。</div> : null}
      {actionError ? (
        <div className="sec-help sec-error" role="alert">
          {actionError}
        </div>
      ) : null}
      {page
        ? tab === "routines"
          ? <Routines page={page} actions={source.actions} locked={locked} run={run} />
          : <Settings key={page.settings.instanceId ?? "unset"} settings={page.settings} actions={source.actions} locked={locked} run={run} />
        : null}
    </div>
  );
}

function Routines({ page, actions, locked, run }: { page: SecretaryPage; actions: SecretaryActions; locked: boolean; run: RunFn }) {
  const [confirm, setConfirm] = useState<string | null>(null);
  return (
    <>
      {page.pausedWhileAway ? (
        <div className="sec-banner">
          你有 {page.pausedWhileAway.awayFor} 没打开 gogoke，定时任务我都先停了，省额度。
          {actions.resumeAll ? (
            <button type="button" className="ghost" disabled={locked} onClick={() => void run("resume-all", actions.resumeAll!)}>
              都恢复
            </button>
          ) : null}
        </div>
      ) : null}
      {page.routines.length === 0 ? (
        <div className="sec-help">
          定时任务是它按时间自己去做的事。跟它说一句就能建，比如“每天早上 9 点汇总各项目进展”。
        </div>
      ) : null}
      {page.routines.map((routine) => (
        <RoutineCard
          key={routine.id}
          routine={routine}
          actions={actions}
          locked={locked}
          confirming={confirm === routine.id}
          onConfirm={(value) => setConfirm(value ? routine.id : null)}
          run={run}
        />
      ))}
    </>
  );
}

function RoutineCard({
  routine,
  actions,
  locked,
  confirming,
  onConfirm,
  run,
}: {
  routine: Routine;
  actions: SecretaryActions;
  locked: boolean;
  confirming: boolean;
  onConfirm: (value: boolean) => void;
  run: RunFn;
}) {
  const last = lastRunLine(routine);
  const toggle = routine.paused ? actions.resumeRoutine : actions.pauseRoutine;
  return (
    <div className={`sec-routine${routine.paused ? " is-paused" : ""}`}>
      <span className="sec-routine-name">{routine.name}</span>
      <span className="sec-routine-when">{routineLine(routine)}</span>
      <span className={`sec-routine-last${last.failed ? " is-err" : ""}`}>{last.text}</span>
      {confirming && actions.deleteRoutine ? (
        <div className="sec-confirm">
          删掉“{routine.name}”？以后不再跑，已经跑过的记录还在。
          <span className="sec-routine-actions">
            <button
              type="button"
              className="sec-link is-danger"
              disabled={locked}
              onClick={() => void run(`delete:${routine.id}`, () => actions.deleteRoutine!(routine.id)).then((ok) => ok && onConfirm(false))}
            >
              确认删除
            </button>
            <button type="button" className="sec-link" onClick={() => onConfirm(false)}>
              取消
            </button>
          </span>
        </div>
      ) : (
        <span className="sec-routine-actions">
          {toggle ? (
            <button type="button" className="sec-link" disabled={locked} onClick={() => void run(`toggle:${routine.id}`, () => toggle(routine.id))}>
              {routine.paused ? "恢复" : "暂停"}
            </button>
          ) : null}
          {actions.openRoutineRecord ? (
            <button type="button" className="sec-link" onClick={() => actions.openRoutineRecord!(routine.id)}>
              看它的记录
            </button>
          ) : null}
          {actions.deleteRoutine ? (
            <button type="button" className="sec-link is-danger" disabled={locked} onClick={() => onConfirm(true)}>
              删除
            </button>
          ) : null}
        </span>
      )}
    </div>
  );
}

function Settings({
  settings,
  actions,
  locked,
  run,
}: {
  settings: SecretarySettings;
  actions: SecretaryActions;
  locked: boolean;
  run: RunFn;
}) {
  const [instanceId, setInstanceId] = useState(settings.instanceId ?? "");
  const models = modelsFor(settings, instanceId);
  const [model, setModel] = useState(settings.model && models.includes(settings.model) ? settings.model : models[0]);
  const [effort, setEffort] = useState(settings.effort ?? settings.efforts[0]);
  const [permission, setPermission] = useState(settings.permission ?? settings.permissions[0]);
  const editable = Boolean(actions.saveSettings);

  const save = () => {
    if (!instanceId || !actions.saveSettings) return;
    const chosen = model && models.includes(model) ? model : undefined;
    void run("settings", () =>
      actions.saveSettings!({
        instanceId,
        ...(chosen ? { model: chosen } : {}),
        ...(effort ? { effort } : {}),
        ...(permission ? { permission } : {}),
      }),
    );
  };

  return (
    <>
      <div className="sec-form">
        <label htmlFor="sec-instance">实例</label>
        <select
          id="sec-instance"
          className="settings-select"
          value={instanceId}
          disabled={!editable || locked}
          onChange={(event) => {
            setInstanceId(event.target.value);
            setModel(modelsFor(settings, event.target.value)[0]);
          }}
        >
          {!settings.instanceId ? <option value="">选一个实例</option> : null}
          {settings.instances.map((item) => (
            <option key={item.id} value={item.id}>
              {item.name}
            </option>
          ))}
        </select>
        {models.length ? (
          <>
            <label htmlFor="sec-model">模型</label>
            <select id="sec-model" className="settings-select" value={model} disabled={!editable || locked} onChange={(event) => setModel(event.target.value)}>
              {models.map((item) => (
                <option key={item}>{item}</option>
              ))}
            </select>
          </>
        ) : null}
        {settings.efforts.length ? (
          <>
            <label htmlFor="sec-effort">推理强度</label>
            <select id="sec-effort" className="settings-select" value={effort} disabled={!editable || locked} onChange={(event) => setEffort(event.target.value)}>
              {settings.efforts.map((item) => (
                <option key={item}>{item}</option>
              ))}
            </select>
          </>
        ) : null}
        {settings.permissions.length ? (
          <>
            <label htmlFor="sec-permission">权限</label>
            <select
              id="sec-permission"
              className="settings-select"
              value={permission}
              disabled={!editable || locked}
              onChange={(event) => setPermission(event.target.value)}
            >
              {settings.permissions.map((item) => (
                <option key={item}>{item}</option>
              ))}
            </select>
          </>
        ) : null}
      </div>
      {editable ? (
        <div>
          <button type="button" className="ghost" disabled={locked || !instanceId} onClick={save}>
            保存
          </button>
        </div>
      ) : null}
      {settings.can?.length || settings.cannot?.length ? (
        <>
          <div className="sec-help sec-can-title">它能做的</div>
          <ul className="sec-can">
            {settings.can?.map((item) => <li key={item}>{item}</li>)}
            {settings.cannot?.map((item) => (
              <li key={item} className="is-no">
                {item}
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </>
  );
}
