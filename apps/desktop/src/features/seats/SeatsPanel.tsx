import { useEffect, useRef, useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { VENDOR_ICONS } from "@/features/instances/vendorIcons";
import {
  canChangeInstance,
  canDelete,
  canTune,
  modelsOf,
  overview,
  seatLine,
  seatState,
  validateNewSeatName,
  type OrchestrationRange,
  type SeatRow,
  type SeatsPage,
  type Tone,
} from "./seatsPageModel";
import "./seats.css";

/** `model` is left out when the host reports no models for the chosen instance. */
export type SeatTune = { instanceId: string; model?: string; effort: string; permission: string };
export type NewSeat = SeatTune & { name: string; template: string };

/** Host operations; a control is shown only when its operation exists. */
export type SeatActions = {
  create?: (seat: NewSeat) => Promise<void>;
  tune?: (seatId: string, tune: SeatTune) => Promise<void>;
  remove?: (seatId: string) => Promise<void>;
  setRange?: (range: OrchestrationRange) => Promise<void>;
};

export type SeatsSource = {
  /** Null while the host has no seat read model for this project. */
  read: () => Promise<SeatsPage | null>;
  actions: SeatActions;
};

function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

const Dot = ({ tone }: { tone: Tone }) => (
  <span className={`seats-dot${tone === "idle" ? "" : ` is-${tone}`}`} aria-hidden />
);

const Pill = ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { children: ReactNode }) => (
  <button type="button" className="ghost git-root-button" {...props}>
    {children}
  </button>
);

type RunFn = (key: string, operation: () => Promise<void>) => Promise<boolean>;

export function SeatsPanel({ source }: { source: SeatsSource }) {
  const [page, setPage] = useState<SeatsPage | null | undefined>(undefined);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [showRemoved, setShowRemoved] = useState(false);
  const busyRef = useRef(false);
  const readSeq = useRef(0);
  // Bumped when the source changes or the panel unmounts; older results are dropped.
  const epoch = useRef(0);
  const sourceRef = useRef(source);

  // Only the latest read of the current source is applied.
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
    // An operation still running against the old source never locks the new one.
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

  if (page === undefined && !loadError) {
    return (
      <div className="seats-panel">
        <div className="seats-help" role="status">
          正在读取席位…
        </div>
      </div>
    );
  }
  if (loadError) {
    return (
      <div className="seats-panel">
        <div className="seats-help seats-error" role="alert">
          读不到席位：{loadError}
        </div>
      </div>
    );
  }
  if (!page) {
    return (
      <div className="seats-panel">
        <div className="seats-overview">
          <span className="seats-overview-title">席位</span>
          <span className="seats-overview-text">席位数据还没接上，这里暂时看不到席位。</span>
        </div>
      </div>
    );
  }

  const direct = page.seats.filter((row) => row.layer === "direct" && row.state !== "REMOVED");
  const sub = page.seats.filter((row) => row.layer === "sub" && row.state !== "REMOVED");
  const removed = page.seats.filter((row) => row.state === "REMOVED");

  return (
    <div className="seats-panel">
      <div className="seats-overview">
        <span className="seats-overview-title">席位</span>
        <span className="seats-overview-text">{overview(page)}</span>
      </div>
      {actionError ? (
        <div className="seats-help seats-error" role="alert">
          {actionError}
        </div>
      ) : null}

      <div className="seats-group">
        <span>直属席位</span>
        <span>由你配置</span>
      </div>
      {direct.map((row) => (
        <SeatCard key={row.id} row={row} page={page} actions={source.actions} busy={busy} run={run} />
      ))}
      {source.actions.create ? (
        adding ? (
          <NewSeatForm page={page} create={source.actions.create} busy={busy} run={run} onDone={() => setAdding(false)} />
        ) : (
          <div className="git-root-actions">
            <Pill onClick={() => setAdding(true)}>添加直属席位</Pill>
          </div>
        )
      ) : null}

      <div className="seats-group">
        <span>下属席位</span>
        <span>由主控调度 · {sub.length} 个</span>
      </div>
      {sub.length === 0 ? <div className="seats-help">主控还没有建下属席位。</div> : null}
      {sub.map((row) => (
        <SeatCard key={row.id} row={row} page={page} actions={source.actions} busy={busy} run={run} />
      ))}

      {removed.length ? (
        <>
          {showRemoved
            ? removed.map((row) => (
                <SeatCard key={row.id} row={row} page={page} actions={source.actions} busy={busy} run={run} />
              ))
            : null}
          <button type="button" className="seats-more" onClick={() => setShowRemoved((value) => !value)}>
            {showRemoved ? "收起已删除的" : `已删除 ${removed.length} 个，点开看记录`}
          </button>
        </>
      ) : null}
    </div>
  );
}

function SeatCard({
  row,
  page,
  actions,
  busy,
  run,
}: {
  row: SeatRow;
  page: SeatsPage;
  actions: SeatActions;
  busy: string | null;
  run: RunFn;
}) {
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<"view" | "tune" | "range" | "delete">("view");
  const state = seatState(row);
  const line = seatLine(row);
  const locked = busy !== null;

  const toggle = () => {
    setOpen((value) => !value);
    setMode("view");
  };

  return (
    <div className={`seats-card${open ? " is-open" : ""}${row.state === "REMOVED" ? " is-removed" : ""}`}>
      <button type="button" className="seats-card-head" aria-expanded={open} onClick={toggle}>
        <span className="seats-card-top">
          <span className="seats-card-name">{row.name}</span>
          <span className="seats-card-state">
            <Dot tone={state.tone} />
            {state.label}
          </span>
        </span>
        <span className="seats-card-instance">
          <span
            className="seats-vendor-icon"
            style={{
              WebkitMaskImage: `url(${VENDOR_ICONS[row.instance.vendor]})`,
              maskImage: `url(${VENDOR_ICONS[row.instance.vendor]})`,
            }}
            aria-hidden
          />
          <span>
            {row.instance.name} · {row.model} {row.effort}
          </span>
        </span>
        {line ? <span className="seats-card-line">{line}</span> : null}
      </button>

      {open && mode === "view" ? (
        <>
          <dl className="seats-details">
            <dt>类型</dt>
            <dd>
              {row.term === "long" ? "长期" : "短期"} · {row.layer === "direct" ? "直属席位" : "下属席位"}
            </dd>
            <dt>权限</dt>
            <dd>{row.permission}</dd>
            {row.goal ? (
              <>
                <dt>当前目标</dt>
                <dd>{row.goal}</dd>
              </>
            ) : null}
            {row.pending !== undefined ? (
              <>
                <dt>待决问题</dt>
                <dd>{row.pending || "无"}</dd>
              </>
            ) : null}
            {row.reclaimCondition ? (
              <>
                <dt>回收条件</dt>
                <dd>{row.reclaimCondition}（主控设定）</dd>
              </>
            ) : null}
            {row.log?.length ? (
              <>
                <dt>最近动作</dt>
                <dd>
                  {row.log.map(([time, text], index) => (
                    <div key={`${time}-${index}`}>
                      <span className="seats-log-time">{time}</span>
                      {text}
                    </div>
                  ))}
                </dd>
              </>
            ) : null}
          </dl>
          {(canTune(row) && (actions.tune || (row.isLead && actions.setRange && page.range))) ||
          (canDelete(row) && actions.remove) ? (
            <div className="git-root-actions">
              {canTune(row) && actions.tune ? <Pill onClick={() => setMode("tune")}>调整</Pill> : null}
              {canTune(row) && row.isLead && actions.setRange && page.range ? (
                <Pill onClick={() => setMode("range")}>编排范围</Pill>
              ) : null}
              {canDelete(row) && actions.remove ? <Pill onClick={() => setMode("delete")}>删除席位</Pill> : null}
            </div>
          ) : null}
        </>
      ) : null}

      {open && mode === "tune" && actions.tune ? (
        <TuneForm
          row={row}
          page={page}
          locked={locked}
          onCancel={() => setMode("view")}
          onSave={(tune) =>
            void run(`${row.id}:tune`, () => actions.tune!(row.id, tune)).then((ok) => ok && setMode("view"))
          }
        />
      ) : null}

      {open && mode === "range" && actions.setRange && page.range ? (
        <RangeForm
          page={page}
          range={page.range}
          locked={locked}
          onCancel={() => setMode("view")}
          onSave={(range) =>
            void run("lead:range", () => actions.setRange!(range)).then((ok) => ok && setMode("view"))
          }
        />
      ) : null}

      {open && mode === "delete" && actions.remove ? (
        <>
          <div className="seats-confirm">
            删除“{row.name}”？
            {row.state === "WORKING" ? "它正在干活，会先请求停止，确认停下后再删除。" : ""}
            它的工作记录会保留，可以随时查看。
            {row.layer === "sub" ? "主控之后需要的话会重新建。" : ""}
          </div>
          <div className="git-root-actions">
            <Pill
              disabled={locked}
              onClick={() => void run(`${row.id}:remove`, () => actions.remove!(row.id)).then((ok) => ok && setOpen(false))}
            >
              确认删除
            </Pill>
            <Pill onClick={() => setMode("view")}>取消</Pill>
          </div>
        </>
      ) : null}
    </div>
  );
}

function TuneForm({
  row,
  page,
  locked,
  onSave,
  onCancel,
}: {
  row: SeatRow;
  page: SeatsPage;
  locked: boolean;
  onSave: (tune: SeatTune) => void;
  onCancel: () => void;
}) {
  const [instanceId, setInstanceId] = useState(row.instance.id);
  const [model, setModel] = useState(row.model);
  const models = modelsOf(page, instanceId, row.instance);
  const [effort, setEffort] = useState(row.effort);
  const [permission, setPermission] = useState(row.permission);
  const instanceLocked = !canChangeInstance(row);
  const choices = page.instances.some((item) => item.id === row.instance.id)
    ? page.instances
    : [row.instance, ...page.instances];

  return (
    <>
      <div className="seats-form">
        <label htmlFor={`${row.id}-instance`}>实例</label>
        <select
          id={`${row.id}-instance`}
          value={instanceId}
          disabled={instanceLocked}
          onChange={(event) => {
            const next = modelsOf(page, event.target.value, row.instance);
            setInstanceId(event.target.value);
            if (!next.includes(model)) setModel(next[0] ?? row.model);
          }}
        >
          {choices.map((item) => (
            <option key={item.id} value={item.id}>
              {item.name}
            </option>
          ))}
        </select>
        {models.length ? (
          <>
            <label htmlFor={`${row.id}-model`}>模型</label>
            <select id={`${row.id}-model`} value={model} onChange={(event) => setModel(event.target.value)}>
              {models.map((item) => (
                <option key={item}>{item}</option>
              ))}
            </select>
          </>
        ) : null}
        <label htmlFor={`${row.id}-effort`}>推理强度</label>
        <select id={`${row.id}-effort`} value={effort} onChange={(event) => setEffort(event.target.value)}>
          {page.efforts.map((item) => (
            <option key={item}>{item}</option>
          ))}
        </select>
        <label htmlFor={`${row.id}-permission`}>权限</label>
        <select id={`${row.id}-permission`} value={permission} onChange={(event) => setPermission(event.target.value)}>
          {page.permissions.map((item) => (
            <option key={item}>{item}</option>
          ))}
        </select>
      </div>
      {instanceLocked ? (
        <div className="seats-help">{row.instanceLockedReason ?? "现在不能给它换实例"}</div>
      ) : null}
      <div className="git-root-actions">
        <Pill
          disabled={locked}
          onClick={() => onSave({ instanceId, ...(models.length ? { model } : {}), effort, permission })}
        >
          保存
        </Pill>
        <Pill onClick={onCancel}>取消</Pill>
      </div>
    </>
  );
}

function RangeForm({
  page,
  range,
  locked,
  onSave,
  onCancel,
}: {
  page: SeatsPage;
  range: OrchestrationRange;
  locked: boolean;
  onSave: (range: OrchestrationRange) => void;
  onCancel: () => void;
}) {
  const [instanceIds, setInstanceIds] = useState<string[]>(range.instanceIds);
  const [maxPermission, setMaxPermission] = useState(range.maxPermission);
  const [maxConcurrent, setMaxConcurrent] = useState(range.maxConcurrent);
  const toggle = (id: string) =>
    setInstanceIds((current) => (current.includes(id) ? current.filter((item) => item !== id) : [...current, id]));

  return (
    <>
      <div className="seats-form">
        <span className="seats-help">能用的实例</span>
        <div className="seats-checks">
          {page.instances.map((item) => (
            <label key={item.id}>
              <input type="checkbox" checked={instanceIds.includes(item.id)} onChange={() => toggle(item.id)} />
              {item.name}
            </label>
          ))}
        </div>
        <label htmlFor="seats-range-permission">最高权限</label>
        <select id="seats-range-permission" value={maxPermission} onChange={(event) => setMaxPermission(event.target.value)}>
          {page.permissions.map((item) => (
            <option key={item}>{item}</option>
          ))}
        </select>
        <label htmlFor="seats-range-max">同时最多</label>
        <select
          id="seats-range-max"
          value={maxConcurrent}
          onChange={(event) => setMaxConcurrent(Number(event.target.value))}
        >
          {[1, 2, 3, 4, 6, 8].map((count) => (
            <option key={count} value={count}>
              {count} 个席位
            </option>
          ))}
        </select>
      </div>
      <div className="seats-help">主控只能在这个范围内建席位、派实例；超出的会被直接拒绝并记下来。</div>
      <div className="git-root-actions">
        <Pill disabled={locked} onClick={() => onSave({ instanceIds, maxPermission, maxConcurrent })}>
          保存
        </Pill>
        <Pill onClick={onCancel}>取消</Pill>
      </div>
    </>
  );
}

function NewSeatForm({
  page,
  create,
  busy,
  run,
  onDone,
}: {
  page: SeatsPage;
  create: NonNullable<SeatActions["create"]>;
  busy: string | null;
  run: RunFn;
  onDone: () => void;
}) {
  const [name, setName] = useState("");
  const [template, setTemplate] = useState(page.templates[0] ?? "");
  const [instanceId, setInstanceId] = useState(page.instances[0]?.id ?? "");
  const models = modelsOf(page, instanceId);
  const [model, setModel] = useState(models[0] ?? "");
  const [effort, setEffort] = useState(page.efforts[0] ?? "");
  const [permission, setPermission] = useState(page.permissions[0] ?? "");
  const [error, setError] = useState<string | null>(null);

  const submit = () => {
    const problem = validateNewSeatName(page, name);
    if (problem) {
      setError(problem);
      return;
    }
    if (!instanceId) {
      setError("现在没有能用的实例，先到设置里的实例页登录一个");
      return;
    }
    setError(null);
    void run("seat:create", () =>
      create({ name: name.trim(), template, instanceId, ...(models.length ? { model } : {}), effort, permission }),
    ).then((ok) => ok && onDone());
  };

  return (
    <div className="seats-card is-open">
      <span className="seats-card-name">添加直属席位</span>
      <div className="seats-form" style={{ borderTop: 0, paddingTop: 0 }}>
        <label htmlFor="seats-new-name">名字</label>
        <input
          id="seats-new-name"
          value={name}
          autoFocus
          placeholder="比如：审计 2"
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") submit();
            if (event.key === "Escape") onDone();
          }}
        />
        <label htmlFor="seats-new-template">职责</label>
        <select id="seats-new-template" value={template} onChange={(event) => setTemplate(event.target.value)}>
          {page.templates.map((item) => (
            <option key={item}>{item}</option>
          ))}
        </select>
        <label htmlFor="seats-new-instance">实例</label>
        <select
          id="seats-new-instance"
          value={instanceId}
          onChange={(event) => {
            setInstanceId(event.target.value);
            setModel(modelsOf(page, event.target.value)[0] ?? "");
          }}
        >
          {page.instances.map((item) => (
            <option key={item.id} value={item.id}>
              {item.name}
            </option>
          ))}
        </select>
        {models.length ? (
          <>
            <label htmlFor="seats-new-model">模型</label>
            <select id="seats-new-model" value={model} onChange={(event) => setModel(event.target.value)}>
              {models.map((item) => (
                <option key={item}>{item}</option>
              ))}
            </select>
          </>
        ) : null}
        <label htmlFor="seats-new-effort">推理强度</label>
        <select id="seats-new-effort" value={effort} onChange={(event) => setEffort(event.target.value)}>
          {page.efforts.map((item) => (
            <option key={item}>{item}</option>
          ))}
        </select>
        <label htmlFor="seats-new-permission">权限</label>
        <select id="seats-new-permission" value={permission} onChange={(event) => setPermission(event.target.value)}>
          {page.permissions.map((item) => (
            <option key={item}>{item}</option>
          ))}
        </select>
      </div>
      <div className="seats-help">
        职责从模板复制一份，之后可以在这个项目里单独改，不影响模板。只列出能用的实例（已登录、没停用）。
      </div>
      {error ? (
        <div className="seats-help seats-error" role="alert">
          {error}
        </div>
      ) : null}
      <div className="git-root-actions">
        <Pill disabled={busy !== null} onClick={submit}>
          添加
        </Pill>
        <Pill onClick={onDone}>取消</Pill>
      </div>
    </div>
  );
}
