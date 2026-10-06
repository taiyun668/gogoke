import { useEffect, useRef, useState, type ReactNode } from "react";
import { SettingsSection } from "@/features/design-system/components/settings/SettingsPrimitives";
import {
  PopoverMenuItem,
  PopoverSurface,
} from "@/features/design-system/components/popover/PopoverPrimitives";
import { VENDOR_ICONS as ICONS } from "./vendorIcons";
import {
  VENDORS,
  cliSummary,
  cliUsable,
  instanceSummary,
  pageSummary,
  primaryAction,
  runningSessions,
  seatNames,
  type CliCopy,
  type InstancePage,
  type InstanceRow,
  type Tone,
  type VendorId,
  type VendorSection,
} from "./instancePageModel";
import "./instances.css";

/**
 * Host operations. A control is rendered only when its operation exists, so the
 * page never offers an action the host cannot perform.
 */
export type InstanceActions = {
  login?: (id: string) => Promise<void>;
  cancelLogin?: (id: string) => Promise<void>;
  check?: (id: string) => Promise<void>;
  enable?: (id: string) => Promise<void>;
  disable?: (id: string) => Promise<void>;
  remove?: (id: string) => Promise<void>;
  rename?: (id: string, name: string) => Promise<void>;
  setCap?: (id: string, cap: number) => Promise<void>;
  openFolder?: (id: string) => Promise<void>;
  create?: (vendor: VendorId, input: { name: string; provider?: string }) => Promise<void>;
  installCli?: (vendor: VendorId) => Promise<void>;
  upgradeCli?: (vendor: VendorId) => Promise<void>;
  rollbackCli?: (vendor: VendorId) => Promise<void>;
  checkCliUpdates?: (vendor: VendorId) => Promise<void>;
  retryCliSelfTest?: (vendor: VendorId) => Promise<void>;
  uninstallCli?: (vendor: VendorId) => Promise<void>;
};

export type InstancePageSource = {
  read: () => Promise<InstancePage>;
  actions: InstanceActions;
  /** Provider choices for CLIs that connect to another vendor's models. */
  providers?: string[];
  /** False while the host cannot take a name at creation; the name input is then hidden. */
  createTakesName?: boolean;
  /** Whether the host can create an instance in this section; always when omitted. */
  canCreate?: (section: VendorSection) => boolean;
};

const CAP_MIN = 1;
const CAP_MAX = 8;
const DEFAULT_CAP = 4;

function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

function elapsed(since: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - since) / 1000));
  return seconds < 60 ? `${seconds} 秒` : `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
}

function Dot({ tone }: { tone: Tone }) {
  return <span className={`instances-dot${tone === "idle" ? "" : ` is-${tone}`}`} aria-hidden />;
}

function Row({
  title,
  subtitle,
  controls,
  children,
  disabled = false,
}: {
  title: ReactNode;
  subtitle: ReactNode;
  controls?: ReactNode;
  children?: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className={`settings-toggle-row instances-row${disabled ? " is-disabled" : ""}`}>
      <div className="instances-row-main">
        <div className="settings-toggle-title">{title}</div>
        <div className="settings-toggle-subtitle instances-row-sub">{subtitle}</div>
      </div>
      {controls ? <div className="settings-agents-actions">{controls}</div> : null}
      {children}
    </div>
  );
}

function RawText({ raw, open, onToggle }: { raw?: string; open: boolean; onToggle: () => void }) {
  if (!raw) return null;
  return (
    <>
      {" "}
      <button type="button" className="instances-link" onClick={onToggle} aria-expanded={open}>
        {open ? "收起原话" : "查看原话"}
      </button>
    </>
  );
}

export function InstancesPage({ source }: { source: InstancePageSource }) {
  const [page, setPage] = useState<InstancePage | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const busyRef = useRef(false);
  const readingRef = useRef(false);
  const generation = useRef(0);

  const refresh = async () => {
    if (readingRef.current) return;
    readingRef.current = true;
    const mine = generation.current;
    try {
      const next = await source.read();
      if (mine === generation.current) {
        setPage(next);
        setLoadError(null);
      }
    } catch (cause) {
      if (mine === generation.current) setLoadError(errorText(cause));
    } finally {
      readingRef.current = false;
    }
  };

  // The host owns login and install progress; the page only reads it back.
  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => {
      setNow(Date.now());
      if (!busyRef.current) void refresh();
    }, 1000);
    return () => window.clearInterval(timer);
  }, [source]);

  const run = async (key: string, operation: () => Promise<void>) => {
    if (busyRef.current) return;
    busyRef.current = true;
    generation.current += 1;
    setBusy(key);
    setActionError(null);
    try {
      await operation();
    } catch (cause) {
      setActionError(errorText(cause));
    } finally {
      busyRef.current = false;
      setBusy(null);
      void refresh();
    }
  };

  const summary = page ? pageSummary(page) : null;

  return (
    <SettingsSection
      title="实例"
      subtitle="每个实例是一个登好的账号，和你平时用的 CLI 互不影响。想换账号，就删掉实例再新建一个。"
    >
      {!page && !loadError ? (
        <div className="settings-help" role="status">
          正在读取实例…
        </div>
      ) : null}
      {loadError ? (
        <div className="settings-help settings-help-error" role="alert">
          读不到实例：{loadError}
        </div>
      ) : null}
      {summary ? (
        <div className="settings-help" role="status">
          {summary.usable} 个可以用
          {summary.disabled ? `，${summary.disabled} 个停用` : ""}
          {summary.needsOwner.length ? (
            <>
              。<span className="instances-warn-text">要处理：{summary.needsOwner.join("、")}</span>
            </>
          ) : (
            "，都正常"
          )}
        </div>
      ) : null}
      {actionError ? (
        <div className="settings-help settings-help-error" role="alert">
          {actionError}
        </div>
      ) : null}
      {page?.sections.map((section) => (
        <VendorGroup
          key={section.vendor}
          section={section}
          source={source}
          busy={busy}
          now={now}
          run={run}
        />
      ))}
    </SettingsSection>
  );
}

type RunFn = (key: string, operation: () => Promise<void>) => Promise<void>;

function VendorGroup({
  section,
  source,
  busy,
  now,
  run,
}: {
  section: VendorSection;
  source: InstancePageSource;
  busy: string | null;
  now: number;
  run: RunFn;
}) {
  const info = VENDORS[section.vendor];
  const [creating, setCreating] = useState(false);
  const [removedName, setRemovedName] = useState<string | null>(null);
  const actions = source.actions;
  // An unknown CLI (host has not reported one) does not block creating the first instance.
  const usable = section.cli === undefined || cliUsable(section.cli);
  const canCreate = Boolean(actions.create) && (source.canCreate?.(section) ?? true);

  return (
    <div className="instances-group" aria-label={info.label}>
      <div className="settings-subsection-title instances-group-title">
        <span
          className="instances-vendor-icon"
          style={{ WebkitMaskImage: `url(${ICONS[section.vendor]})`, maskImage: `url(${ICONS[section.vendor]})` }}
          aria-hidden
        />
        {info.label}
      </div>
      {info.unsupported ? (
        <Row
          title={info.label}
          subtitle={
            <>
              <Dot tone="idle" />
              暂不支持：{info.unsupported}
            </>
          }
        />
      ) : (
        <>
          <CliRow section={section} actions={actions} busy={busy} run={run} />
          {section.instances.map((row) => (
            <InstanceRowView
              key={row.id}
              row={row}
              vendor={section.vendor}
              actions={actions}
              busy={busy}
              now={now}
              run={run}
              onRemoved={setRemovedName}
              onNewInstance={() => setCreating(true)}
            />
          ))}
          {removedName ? (
            <div className="settings-help" role="status">
              已删除“{removedName}”。
            </div>
          ) : null}
          {canCreate && actions.create ? (
            creating ? (
              <NewInstanceForm
                section={section}
                providers={source.providers}
                takesName={source.createTakesName !== false}
                busy={busy}
                run={run}
                create={actions.create}
                onDone={() => setCreating(false)}
              />
            ) : (
              <div className="settings-agents-actions">
                <button type="button" className="ghost" disabled={!usable} onClick={() => setCreating(true)}>
                  新建 {info.label} 实例
                </button>
              </div>
            )
          ) : null}
        </>
      )}
    </div>
  );
}

function CliRow({
  section,
  actions,
  busy,
  run,
}: {
  section: VendorSection;
  actions: InstanceActions;
  busy: string | null;
  run: RunFn;
}) {
  const vendor = section.vendor;
  const info = VENDORS[vendor];
  const cli: CliCopy = section.cli ?? { state: "READY" };
  const [details, setDetails] = useState(false);
  const [rawOpen, setRawOpen] = useState(false);
  const running = runningSessions(section);
  const summary: { tone: Tone; text: string } = section.cli
    ? cliSummary(cli, running)
    : { tone: "idle", text: "还没检查过，新建实例时会检查" };
  const key = (op: string) => `${vendor}:${op}`;
  const call = (op: string, fn?: (v: VendorId) => Promise<void>) =>
    fn ? () => void run(key(op), () => fn(vendor)) : undefined;

  let primary: ReactNode = null;
  if (cli.state === "NOT_INSTALLED" && actions.installCli) {
    primary = (
      <button type="button" className="ghost" disabled={busy !== null} onClick={call("install", actions.installCli)}>
        安装
      </button>
    );
  } else if (cli.state === "INSTALL_FAILED" && actions.installCli) {
    primary = (
      <button type="button" className="ghost" disabled={busy !== null} onClick={call("install", actions.installCli)}>
        重试
      </button>
    );
  } else if (cli.state === "BLOCKED" && actions.retryCliSelfTest) {
    primary = (
      <button type="button" className="ghost" disabled={busy !== null} onClick={call("selftest", actions.retryCliSelfTest)}>
        {busy === key("selftest") ? "正在试…" : "再试一次"}
      </button>
    );
  } else if (cli.state === "READY" && cli.verifiedVersion && actions.upgradeCli) {
    primary = (
      <button
        type="button"
        className="ghost"
        disabled={busy !== null || running > 0}
        onClick={call("upgrade", actions.upgradeCli)}
      >
        升级
      </button>
    );
  }

  return (
    <Row
      title={`${info.label} CLI`}
      subtitle={
        <>
          <Dot tone={summary.tone} />
          {summary.text}
          {["INSTALL_FAILED", "BLOCKED", "UPGRADE_FAILED"].includes(cli.state) ? (
            <RawText raw={cli.raw} open={rawOpen} onToggle={() => setRawOpen((v) => !v)} />
          ) : null}
        </>
      }
      controls={
        <>
          {primary}
          <button type="button" className="ghost" aria-expanded={details} onClick={() => setDetails((v) => !v)}>
            {details ? "收起" : "详情"}
          </button>
        </>
      }
    >
      {cli.progress !== undefined && ["INSTALLING", "UPGRADING"].includes(cli.state) ? (
        <div className="instances-progress instances-full" role="progressbar" aria-valuenow={cli.progress} aria-valuemin={0} aria-valuemax={100}>
          <span style={{ width: `${cli.progress}%` }} />
        </div>
      ) : null}
      {rawOpen && cli.raw ? <pre className="instances-raw instances-full">{cli.raw}</pre> : null}
      {details ? (
        <dl className="instances-details instances-full">
          <dt>版本</dt>
          <dd>
            {cli.version ?? "—"}
            {cli.verifiedVersion ? `（可升级到 ${cli.verifiedVersion}）` : ""}
            {cli.officialVersion ? `；官方已有 ${cli.officialVersion}，gogoke 还没验证` : ""}
          </dd>
          <dt>能力</dt>
          <dd>
            插话：{info.steer} · 提问：{info.questions} · 登录：{info.login}
          </dd>
          <dt>检查更新</dt>
          <dd>
            {cli.checkedAt ?? "还没检查过"}
            {actions.checkCliUpdates ? (
              <>
                {" "}
                <button type="button" className="instances-link" disabled={busy !== null} onClick={call("check", actions.checkCliUpdates)}>
                  {busy === key("check") ? "正在检查…" : "现在检查"}
                </button>
              </>
            ) : null}
            {cli.previousVersion && actions.rollbackCli ? (
              <>
                {" · "}
                <button type="button" className="instances-link" disabled={busy !== null || running > 0} onClick={call("rollback", actions.rollbackCli)}>
                  退回 {cli.previousVersion}
                </button>
              </>
            ) : null}
            {section.instances.length === 0 && cliUsable(cli) && actions.uninstallCli ? (
              <>
                {" · "}
                <button type="button" className="instances-link" disabled={busy !== null} onClick={call("uninstall", actions.uninstallCli)}>
                  卸载
                </button>
              </>
            ) : null}
          </dd>
        </dl>
      ) : null}
    </Row>
  );
}

const PRIMARY_LABEL = {
  login: "登录",
  relogin: "重新登录",
  "login-same-account": "用原账号重新登录",
  check: "检测",
  "cancel-login": "取消登录",
  enable: "启用",
} as const;

function InstanceRowView({
  row,
  vendor,
  actions,
  busy,
  now,
  run,
  onRemoved,
  onNewInstance,
}: {
  row: InstanceRow;
  vendor: VendorId;
  actions: InstanceActions;
  busy: string | null;
  now: number;
  run: RunFn;
  onRemoved: (name: string) => void;
  onNewInstance: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [details, setDetails] = useState(false);
  const [rawOpen, setRawOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [nameDraft, setNameDraft] = useState(row.name);
  const [confirm, setConfirm] = useState<"remove" | "disable" | "blocked" | null>(null);
  const [copied, setCopied] = useState(false);
  const menuRef = useRef<HTMLSpanElement>(null);
  const summary = instanceSummary(row);
  const primary = primaryAction(row);
  const key = (op: string) => `${row.id}:${op}`;
  const label = VENDORS[vendor].label;
  const locked = busy !== null;

  useEffect(() => {
    if (!menuOpen) return;
    const close = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setMenuOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [menuOpen]);

  const primaryHandler = (): (() => Promise<void>) | undefined => {
    switch (primary) {
      case "login":
      case "relogin":
      case "login-same-account":
        return actions.login ? () => actions.login!(row.id) : undefined;
      case "check":
        return actions.check ? () => actions.check!(row.id) : undefined;
      case "cancel-login":
        return actions.cancelLogin ? () => actions.cancelLogin!(row.id) : undefined;
      case "enable":
        return actions.enable ? () => actions.enable!(row.id) : undefined;
      default:
        return undefined;
    }
  };
  const handler = primaryHandler();
  const blockedByUse = row.seats.length > 0 || (row.runningSessions ?? 0) > 0;

  const copyCode = async () => {
    if (!row.login?.deviceCode) return;
    try {
      await navigator.clipboard.writeText(row.login.deviceCode);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
    }
  };

  const title = renaming ? (
    <span className="instances-form">
      <input
        className="settings-input settings-input--compact"
        value={nameDraft}
        aria-label="名字"
        autoFocus
        onChange={(event) => setNameDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") setRenaming(false);
          if (event.key === "Enter" && nameDraft.trim() && actions.rename) {
            void run(key("rename"), () => actions.rename!(row.id, nameDraft.trim())).then(() => setRenaming(false));
          }
        }}
      />
      <button
        type="button"
        className="ghost"
        disabled={locked || !nameDraft.trim()}
        onClick={() =>
          actions.rename &&
          void run(key("rename"), () => actions.rename!(row.id, nameDraft.trim())).then(() => setRenaming(false))
        }
      >
        保存
      </button>
      <button type="button" className="ghost" onClick={() => setRenaming(false)}>
        取消
      </button>
    </span>
  ) : (
    row.name
  );

  return (
    <Row
      disabled={!row.enabled}
      title={title}
      subtitle={
        <>
          <Dot tone={summary.tone} />
          {summary.text}
          {row.state === "LOGGING_IN" && row.login ? (
            <> · 已等 {elapsed(row.login.startedAt, now)}，授权完会自己变成“可以用”，关掉设置也不会中断</>
          ) : null}
          {row.state === "WRONG_ACCOUNT" && actions.create ? (
            <>
              。要用那个账号就{" "}
              <button type="button" className="instances-link" onClick={onNewInstance}>
                新建一个实例
              </button>
            </>
          ) : null}
          {row.enabled && row.state === "READY" && row.checkFailed ? (
            <>
              <br />
              状态可能不是最新的：上次确认是 {row.lastConfirmed ?? "之前"}，这次没检测成功
            </>
          ) : null}
          {row.enabled && row.settledLeftover ? (
            <>
              <br />
              上次 gogoke 没正常关闭，留下的进程已经自动收尾
            </>
          ) : null}
          {["LOGIN_FAILED", "ERROR"].includes(row.state) ? (
            <RawText raw={row.raw} open={rawOpen} onToggle={() => setRawOpen((v) => !v)} />
          ) : null}
        </>
      }
      controls={
        <>
          {primary && handler ? (
            <button type="button" className="ghost" disabled={locked} onClick={() => void run(key(primary), handler)}>
              {busy === key(primary) && primary === "check" ? "正在检测…" : PRIMARY_LABEL[primary]}
            </button>
          ) : null}
          {(
            <span className="instances-menu-anchor" ref={menuRef}>
              <button
                type="button"
                className="ghost icon-button"
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                aria-label={`${row.name} 的更多操作`}
                onClick={() => setMenuOpen((v) => !v)}
              >
                <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                  <circle cx="5" cy="12" r="1.8" />
                  <circle cx="12" cy="12" r="1.8" />
                  <circle cx="19" cy="12" r="1.8" />
                </svg>
              </button>
              {menuOpen ? (
                <PopoverSurface className="instances-menu" role="menu">
                  <PopoverMenuItem role="menuitem" onClick={() => { setDetails((v) => !v); setMenuOpen(false); }}>
                    {details ? "收起详情" : "详情"}
                  </PopoverMenuItem>
                  {actions.rename ? (
                    <PopoverMenuItem role="menuitem" onClick={() => { setNameDraft(row.name); setRenaming(true); setMenuOpen(false); }}>
                      改名字
                    </PopoverMenuItem>
                  ) : null}
                  {actions.openFolder ? (
                    <PopoverMenuItem role="menuitem" onClick={() => { setMenuOpen(false); void run(key("folder"), () => actions.openFolder!(row.id)); }}>
                      打开它的文件夹
                    </PopoverMenuItem>
                  ) : null}
                  {row.state === "READY" && row.enabled && actions.disable ? (
                    <PopoverMenuItem role="menuitem" onClick={() => { setConfirm("disable"); setMenuOpen(false); }}>
                      停用
                    </PopoverMenuItem>
                  ) : null}
                  {actions.remove ? (
                    <>
                      <hr className="instances-menu-separator" />
                      <PopoverMenuItem
                        role="menuitem"
                        className="is-danger"
                        onClick={() => { setConfirm(blockedByUse ? "blocked" : "remove"); setMenuOpen(false); }}
                      >
                        删除实例
                      </PopoverMenuItem>
                    </>
                  ) : null}
                </PopoverSurface>
              ) : null}
            </span>
          )}
        </>
      }
    >
      {row.state === "LOGGING_IN" && row.login ? (
        <div className="instances-code-line instances-full">
          <span>
            {row.login.browserOpened ? "在浏览器里授权，" : "浏览器没自动打开，请打开授权页；"}需要输入代码时填：
          </span>
          {row.login.deviceCode ? <span className="instances-code">{row.login.deviceCode}</span> : null}
          {row.login.deviceCode ? (
            <button type="button" className="ghost" onClick={() => void copyCode()}>
              {copied ? "已复制" : "复制代码"}
            </button>
          ) : null}
          {!row.login.browserOpened && row.login.authorizationUrl ? (
            <a className="instances-link" href={row.login.authorizationUrl} target="_blank" rel="noreferrer">
              打开授权页
            </a>
          ) : null}
        </div>
      ) : null}
      {rawOpen && row.raw ? <pre className="instances-raw instances-full">{row.raw}</pre> : null}
      {confirm === "remove" && actions.remove ? (
        <div className="instances-confirm instances-full">
          <span className="settings-help">
            删除“{row.name}”？它的登录和会话记录会一起删掉，不能恢复。你平时用的 {label} 和厂商账号都不受影响。
          </span>
          <span className="settings-agents-actions">
            <button
              type="button"
              className="ghost instances-danger"
              disabled={locked}
              onClick={() => void run(key("remove"), () => actions.remove!(row.id)).then(() => onRemoved(row.name))}
            >
              确认删除
            </button>
            <button type="button" className="ghost" onClick={() => setConfirm(null)}>
              取消
            </button>
          </span>
        </div>
      ) : null}
      {confirm === "disable" && actions.disable ? (
        <div className="instances-confirm instances-full">
          <span className="settings-help">
            停用“{row.name}”？主控不再给它派活，登录保留，随时能启用。
            {row.seats.length ? `${seatNames(row)} 要在席位页换一个实例。` : ""}
          </span>
          <span className="settings-agents-actions">
            <button
              type="button"
              className="ghost"
              disabled={locked}
              onClick={() => void run(key("disable"), () => actions.disable!(row.id)).then(() => setConfirm(null))}
            >
              停用
            </button>
            <button type="button" className="ghost" onClick={() => setConfirm(null)}>
              取消
            </button>
          </span>
        </div>
      ) : null}
      {confirm === "blocked" ? (
        <div className="settings-help instances-full" role="status">
          {seatNames(row) || "有会话"} 还在用它，先在席位页换掉，才能删除。{" "}
          <button type="button" className="instances-link" onClick={() => setConfirm(null)}>
            知道了
          </button>
        </div>
      ) : null}
      {details ? (
        <dl className="instances-details instances-full">
          <dt>账号</dt>
          <dd>
            {row.account ?? "登录后才知道"}
            {row.plan ? ` · ${row.plan}` : ""}
            {row.provider ? ` · 经 ${label} 连 ${row.provider}` : ""}
          </dd>
          <dt>能用的模型</dt>
          <dd>{row.models ?? "登录后才知道"}</dd>
          <dt>并发上限</dt>
          <dd>
            {row.cap !== undefined && actions.setCap ? (
              <span className="settings-agents-stepper" role="group" aria-label="并发上限">
                <button
                  type="button"
                  className="ghost settings-agents-stepper-button"
                  aria-label="减少"
                  disabled={locked || row.cap <= CAP_MIN}
                  onClick={() => void run(key("cap"), () => actions.setCap!(row.id, row.cap! - 1))}
                >
                  ▼
                </button>
                <span className="settings-agents-stepper-value" aria-live="polite" aria-atomic="true">
                  {row.cap}
                </span>
                <button
                  type="button"
                  className="ghost settings-agents-stepper-button"
                  aria-label="增加"
                  disabled={locked || row.cap >= CAP_MAX}
                  onClick={() => void run(key("cap"), () => actions.setCap!(row.id, row.cap! + 1))}
                >
                  ▲
                </button>
              </span>
            ) : (
              <span>{row.cap ?? "—"}</span>
            )}{" "}
            <span className="settings-help">同一时间最多开几个会话</span>
          </dd>
          <dt>上次确认</dt>
          <dd>
            {row.lastConfirmed ?? "—"}
            {actions.check ? (
              <>
                {" "}
                <button
                  type="button"
                  className="instances-link"
                  disabled={locked}
                  title="只查登录状态，不花额度"
                  onClick={() => void run(key("check"), () => actions.check!(row.id))}
                >
                  检测
                </button>
              </>
            ) : null}
          </dd>
          {row.log?.length ? (
            <>
              <dt>最近记录</dt>
              <dd className="instances-log">
                {row.log.map(([time, text], index) => (
                  <div key={`${time}-${index}`}>
                    <span className="instances-log-time">{time}</span>
                    {text}
                  </div>
                ))}
              </dd>
            </>
          ) : null}
        </dl>
      ) : null}
    </Row>
  );
}

function NewInstanceForm({
  section,
  providers,
  takesName,
  busy,
  run,
  create,
  onDone,
}: {
  section: VendorSection;
  providers?: string[];
  takesName: boolean;
  busy: string | null;
  run: RunFn;
  create: NonNullable<InstanceActions["create"]>;
  onDone: () => void;
}) {
  const info = VENDORS[section.vendor];
  const fallback = `${info.label} ${section.instances.length + 1}`;
  const [name, setName] = useState("");
  const [provider, setProvider] = useState(providers?.[0] ?? "");
  const [error, setError] = useState<string | null>(null);
  const taken = new Set(section.instances.map((row) => row.name));

  const submit = () => {
    const finalName = name.trim() || fallback;
    if (taken.has(finalName)) {
      setError("已经有叫这个名字的实例了，换一个吧");
      return;
    }
    setError(null);
    void run(`${section.vendor}:create`, () =>
      create(section.vendor, { name: finalName, ...(info.needsProvider ? { provider } : {}) }),
    ).then(onDone);
  };

  return (
    <Row
      title={`新建 ${info.label} 实例`}
      subtitle={`建好后直接开始登录。并发上限默认 ${DEFAULT_CAP}，之后在详情里改。`}
      controls={
        <>
          <button type="button" className="ghost" disabled={busy !== null} onClick={submit}>
            新建并登录
          </button>
          <button type="button" className="ghost" onClick={onDone}>
            取消
          </button>
        </>
      }
    >
      <div className="instances-form instances-full">
        {takesName ? (
        <input
          className="settings-input"
          value={name}
          autoFocus
          placeholder={`名字，可以不填（默认叫“${fallback}”）`}
          aria-label="名字"
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") submit();
            if (event.key === "Escape") onDone();
          }}
        />
        ) : null}
        {info.needsProvider && providers?.length ? (
          <select
            className="settings-select"
            value={provider}
            aria-label="连哪家的模型"
            onChange={(event) => setProvider(event.target.value)}
          >
            {providers.map((option) => (
              <option key={option} value={option}>
                连 {option}
              </option>
            ))}
          </select>
        ) : null}
        {error ? (
          <div className="settings-help settings-help-error instances-full" role="alert">
            {error}
          </div>
        ) : null}
      </div>
    </Row>
  );
}
