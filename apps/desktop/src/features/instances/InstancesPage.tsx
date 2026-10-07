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
  /** First-time name and enabled state (and provider for OpenCode) for an instance the host has no profile for. */
  configureProfile?: (id: string, input: { name: string; enabled: boolean; provider?: string }) => Promise<void>;
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
  const readSeq = useRef(0);
  // Bumped when the source changes or the page unmounts; results from an older epoch are dropped.
  const epoch = useRef(0);
  const sourceRef = useRef(source);

  // Only the latest read of the current source is applied: an older poll, or a read
  // started by an operation on a previous source, never overwrites it.
  const refresh = async () => {
    const mine = ++readSeq.current;
    const era = epoch.current;
    try {
      const next = await sourceRef.current.read();
      if (mine === readSeq.current && era === epoch.current) {
        setPage(next);
        setLoadError(null);
      }
    } catch (cause) {
      if (mine === readSeq.current && era === epoch.current) setLoadError(errorText(cause));
    }
  };

  // The host owns login and install progress; the page only reads it back.
  useEffect(() => {
    epoch.current += 1;
    sourceRef.current = source;
    setPage(null);
    setLoadError(null);
    setActionError(null);
    void refresh();
    const timer = window.setInterval(() => {
      setNow(Date.now());
      if (!busyRef.current) void refresh();
    }, 1000);
    return () => {
      window.clearInterval(timer);
      epoch.current += 1;
    };
  }, [source]);

  /** Resolves true only when the operation succeeded; success UI waits for it. */
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
      busyRef.current = false;
      setBusy(null);
      if (era === epoch.current) void refresh();
    }
    return ok && era === epoch.current;
  };

  // Rows from an earlier read stay visible after a failed read, but nothing can be done to them.
  const stale = page !== null && loadError !== null;
  const summary = page && !stale ? pageSummary(page) : null;

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
          {stale ? `读不到最新状态：${loadError}。下面是上次读到的，现在不能操作。` : `读不到实例：${loadError}`}
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
          busy={stale ? "stale" : busy}
          now={now}
          run={run}
        />
      ))}
    </SettingsSection>
  );
}

type RunFn = (key: string, operation: () => Promise<void>) => Promise<boolean>;

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
              providers={source.providers}
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
                <button type="button" className="ghost" disabled={!usable || busy !== null} onClick={() => setCreating(true)}>
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

const CLI_RAW_STATES = ["INSTALL_FAILED", "BLOCKED", "UPGRADE_FAILED", "PROBE_UNKNOWN"];

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
  // Placeholder for an unreported copy; no version, upgrade or uninstall is offered from it.
  const cli: CliCopy = section.cli ?? { state: "READY" };
  const [details, setDetails] = useState(false);
  const [rawOpen, setRawOpen] = useState(false);
  const running = runningSessions(section);
  const summary: { tone: Tone; text: string } = section.cli
    ? cliSummary(cli, running)
    : { tone: "idle", text: "宿主还没报告这份 CLI 的情况" };
  const key = (op: string) => `${vendor}:${op}`;
  const call = (op: string, fn?: (v: VendorId) => Promise<void>) =>
    fn ? () => void run(key(op), () => fn(vendor)) : undefined;

  const install = (label: string) =>
    actions.installCli ? (
      <button type="button" className="ghost" disabled={busy !== null} onClick={call("install", actions.installCli)}>
        {label}
      </button>
    ) : null;

  let primary: ReactNode = null;
  if (!section.cli) {
    primary = null;
  } else if (cli.state === "NOT_INSTALLED") {
    primary = install("安装");
  } else if (cli.state === "INSTALL_FAILED") {
    primary = install("重试");
  } else if (cli.state === "STAGED" || cli.state === "PROBED") {
    primary = install("继续安装");
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
          {CLI_RAW_STATES.includes(cli.state) ? (
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
      {rawOpen && cli.raw ? <pre className="instances-raw instances-full">{cli.raw}</pre> : null}
      {details ? (
        <dl className="instances-details instances-full">
          {section.cli ? (
            <>
              <dt>版本</dt>
              <dd>
                {cli.version ?? "—"}
                {cli.verifiedVersion ? `（可升级到 ${cli.verifiedVersion}）` : ""}
                {cli.officialVersion ? `；官方已有 ${cli.officialVersion}，gogoke 还没验证` : ""}
              </dd>
            </>
          ) : null}
          <dt>能力</dt>
          <dd>
            插话：{info.steer} · 提问：{info.questions} · 登录：{info.login}
          </dd>
          {section.cli ? (
            <>
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
                {section.instances.length === 0 && cli.state === "READY" && actions.uninstallCli ? (
                  <>
                    {" · "}
                    <button type="button" className="instances-link" disabled={busy !== null} onClick={call("uninstall", actions.uninstallCli)}>
                      卸载
                    </button>
                  </>
                ) : null}
              </dd>
            </>
          ) : null}
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
  configure: "设置",
} as const;

function InstanceRowView({
  row,
  vendor,
  actions,
  providers,
  busy,
  now,
  run,
  onRemoved,
  onNewInstance,
}: {
  row: InstanceRow;
  vendor: VendorId;
  actions: InstanceActions;
  providers?: string[];
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
  const [configuring, setConfiguring] = useState(false);
  const [nameDraft, setNameDraft] = useState(row.name);
  const [confirm, setConfirm] = useState<"remove" | "disable" | "blocked" | null>(null);
  const [copied, setCopied] = useState(false);
  const menuRef = useRef<HTMLSpanElement>(null);
  const summary = instanceSummary(row, { canCheck: Boolean(actions.check) });
  const primary = primaryAction(row);
  const key = (op: string) => `${row.id}:${op}`;
  const label = VENDORS[vendor].label;
  const locked = busy !== null;
  const canRename = Boolean(actions.rename) && row.profileReady === true;
  const canDisable = Boolean(actions.disable) && row.profileReady === true && row.enabled === true;

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
        return actions.enable && row.profileReady ? () => actions.enable!(row.id) : undefined;
      default:
        return undefined;
    }
  };
  const handler = primaryHandler();
  const showConfigure = primary === "configure" && !row.profileReady && Boolean(actions.configureProfile);
  const blockedByUse = (row.seats?.length ?? 0) > 0 || (row.runningSessions ?? 0) > 0;

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
          if (event.key === "Enter" && nameDraft.trim() && actions.rename && !locked) {
            void run(key("rename"), () => actions.rename!(row.id, nameDraft.trim())).then((ok) => ok && setRenaming(false));
          }
        }}
      />
      <button
        type="button"
        className="ghost"
        disabled={locked || !nameDraft.trim()}
        onClick={() =>
          actions.rename &&
          void run(key("rename"), () => actions.rename!(row.id, nameDraft.trim())).then((ok) => ok && setRenaming(false))
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
      disabled={row.enabled === false}
      title={title}
      subtitle={
        <>
          <Dot tone={summary.tone} />
          {summary.text}
          {row.state === "LOGGING_IN" && row.login ? (
            <> · 已等 {elapsed(row.login.startedAt, now)}，授权完这里会自己更新，关掉设置也不会中断</>
          ) : null}
          {row.state === "WRONG_ACCOUNT" && actions.create ? (
            <>
              。要用那个账号就{" "}
              <button type="button" className="instances-link" onClick={onNewInstance}>
                新建一个实例
              </button>
            </>
          ) : null}
          {row.enabled !== false && row.state === "READY" && row.checkFailed ? (
            <>
              <br />
              状态可能不是最新的：上次确认是 {row.lastConfirmed ?? "之前"}，这次没确认成功
            </>
          ) : null}
          {row.seatIssues?.length ? (
            <>
              <br />
              {new Set(row.seatIssues.map((issue) => issue.seat)).size} 个席位的会话出了问题，原话在详情里
            </>
          ) : null}
          {row.enabled !== false && row.settledLeftover ? (
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
          {showConfigure && !configuring ? (
            <button type="button" className="ghost" disabled={locked} onClick={() => setConfiguring(true)}>
              {PRIMARY_LABEL.configure}
            </button>
          ) : null}
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
                {canRename ? (
                  <PopoverMenuItem role="menuitem" onClick={() => { setNameDraft(row.name); setRenaming(true); setMenuOpen(false); }}>
                    改名字
                  </PopoverMenuItem>
                ) : null}
                {actions.openFolder ? (
                  <PopoverMenuItem role="menuitem" onClick={() => { setMenuOpen(false); void run(key("folder"), () => actions.openFolder!(row.id)); }}>
                    打开它的文件夹
                  </PopoverMenuItem>
                ) : null}
                {canDisable ? (
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
        </>
      }
    >
      {configuring && actions.configureProfile ? (
        <ConfigureForm
          row={row}
          vendor={vendor}
          providers={providers}
          busy={busy}
          run={run}
          configure={actions.configureProfile}
          onDone={() => setConfiguring(false)}
        />
      ) : null}
      {row.state === "LOGGING_IN" && row.login ? (
        <div className="instances-code-line instances-full">
          <span>
            {row.login.browser === "opened"
              ? "已请浏览器打开授权页。"
              : row.login.browser === "failed"
                ? "浏览器没能自动打开。"
                : "还没打开授权页。"}
            {row.login.deviceCode ? "需要输入代码时填：" : ""}
          </span>
          {row.login.deviceCode ? <span className="instances-code">{row.login.deviceCode}</span> : null}
          {row.login.deviceCode ? (
            <button type="button" className="ghost" onClick={() => void copyCode()}>
              {copied ? "已复制" : "复制代码"}
            </button>
          ) : null}
          {row.login.authorizationUrl ? (
            <a className="instances-link" href={row.login.authorizationUrl} target="_blank" rel="noreferrer">
              {row.login.browser === "opened" ? "没看到的话，打开授权页" : "打开授权页"}
            </a>
          ) : null}
        </div>
      ) : null}
      {rawOpen && row.raw ? <pre className="instances-raw instances-full">{row.raw}</pre> : null}
      {confirm === "remove" && actions.remove ? (
        <div className="instances-confirm instances-full">
          <span className="settings-help">
            删除“{row.name}”？删除后它不再出现，主控也不会再用它。它的登录凭据、本机目录和记录会留在本机，不会被删掉。你平时用的 {label} 和厂商账号都不受影响。
          </span>
          <span className="settings-agents-actions">
            <button
              type="button"
              className="ghost instances-danger"
              disabled={locked}
              onClick={() => void run(key("remove"), () => actions.remove!(row.id)).then((ok) => ok && onRemoved(row.name))}
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
            {row.seats?.length ? `${seatNames(row)} 要在席位页换一个实例。` : ""}
          </span>
          <span className="settings-agents-actions">
            <button
              type="button"
              className="ghost"
              disabled={locked}
              onClick={() => void run(key("disable"), () => actions.disable!(row.id)).then((ok) => ok && setConfirm(null))}
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
          {row.account ? (
            <>
              <dt>账号</dt>
              <dd>
                {row.account}
                {row.plan ? ` · ${row.plan}` : ""}
                {row.provider ? ` · 经 ${label} 连 ${row.provider}` : ""}
              </dd>
            </>
          ) : null}
          {row.models ? (
            <>
              <dt>能用的模型</dt>
              <dd>
                {row.models}
                {row.modelsSource || row.modelsObservedAt ? (
                  <span className="settings-help">
                    {" "}
                    （{[row.modelsSource, row.modelsObservedAt].filter(Boolean).join(" · ")}）
                  </span>
                ) : null}
              </dd>
            </>
          ) : null}
          {row.seatIssues?.length ? (
            <>
              <dt>会话问题</dt>
              <dd className="instances-log">
                {row.seatIssues.map((issue, index) => (
                  <div key={`${issue.seat}-${index}`}>{issue.reason}</div>
                ))}
              </dd>
            </>
          ) : null}
          <dt>并发上限</dt>
          <dd>
            <CapControl row={row} locked={locked} setCap={actions.setCap ? (cap) => run(key("cap"), () => actions.setCap!(row.id, cap)) : undefined} />{" "}
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

/** The cap is the host's value; an unset cap is chosen explicitly, never filled with a default. */
function CapControl({
  row,
  locked,
  setCap,
}: {
  row: InstanceRow;
  locked: boolean;
  setCap?: (cap: number) => Promise<boolean>;
}) {
  if (!setCap) return <span>{row.cap ?? "还没设"}</span>;
  if (row.cap === undefined) {
    return (
      <select
        className="settings-select settings-select--compact"
        aria-label="并发上限"
        value=""
        disabled={locked}
        onChange={(event) => event.target.value && void setCap(Number(event.target.value))}
      >
        <option value="">还没设，选一个</option>
        {Array.from({ length: CAP_MAX - CAP_MIN + 1 }, (_, index) => CAP_MIN + index).map((value) => (
          <option key={value} value={value}>
            {value}
          </option>
        ))}
      </select>
    );
  }
  const cap = row.cap;
  return (
    <span className="settings-agents-stepper" role="group" aria-label="并发上限">
      <button
        type="button"
        className="ghost settings-agents-stepper-button"
        aria-label="减少"
        disabled={locked || cap <= CAP_MIN}
        onClick={() => void setCap(cap - 1)}
      >
        ▼
      </button>
      <span className="settings-agents-stepper-value" aria-live="polite" aria-atomic="true">
        {cap}
      </span>
      <button
        type="button"
        className="ghost settings-agents-stepper-button"
        aria-label="增加"
        disabled={locked || cap >= CAP_MAX}
        onClick={() => void setCap(cap + 1)}
      >
        ▲
      </button>
    </span>
  );
}

/** First-time profile: the Owner gives the name and enabled state; nothing is filled in for them. */
function ConfigureForm({
  row,
  vendor,
  providers,
  busy,
  run,
  configure,
  onDone,
}: {
  row: InstanceRow;
  vendor: VendorId;
  providers?: string[];
  busy: string | null;
  run: RunFn;
  configure: NonNullable<InstanceActions["configureProfile"]>;
  onDone: () => void;
}) {
  const needsProvider = VENDORS[vendor].needsProvider === true;
  const [name, setName] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [provider, setProvider] = useState(row.provider ?? "");
  const [error, setError] = useState<string | null>(null);
  const providerChoices = providers ?? [];

  const submit = () => {
    if (!name.trim()) {
      setError("给它起个名字");
      return;
    }
    if (needsProvider && !provider) {
      setError("选一下它连哪家的模型");
      return;
    }
    setError(null);
    void run(`${row.id}:configure`, () =>
      configure(row.id, { name: name.trim(), enabled, ...(needsProvider ? { provider } : {}) }),
    ).then((ok) => ok && onDone());
  };

  return (
    <div className="instances-form instances-full">
      <input
        className="settings-input settings-input--compact"
        value={name}
        autoFocus
        placeholder={`名字，比如 ${row.name}`}
        aria-label="名字"
        onChange={(event) => setName(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") submit();
          if (event.key === "Escape") onDone();
        }}
      />
      {needsProvider ? (
        providerChoices.length ? (
          <select className="settings-select" value={provider} aria-label="连哪家的模型" onChange={(event) => setProvider(event.target.value)}>
            <option value="">连哪家的模型</option>
            {providerChoices.map((option) => (
              <option key={option} value={option}>
                连 {option}
              </option>
            ))}
          </select>
        ) : (
          <span className="settings-help">还不知道它能连哪家的模型，暂时设不了。</span>
        )
      ) : null}
      <label className="instances-check">
        <input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />
        启用（主控可以给它派活）
      </label>
      <button
        type="button"
        className="ghost"
        disabled={busy !== null || (needsProvider && !providerChoices.length)}
        onClick={submit}
      >
        保存
      </button>
      <button type="button" className="ghost" onClick={onDone}>
        取消
      </button>
      {error ? (
        <div className="settings-help settings-help-error instances-full" role="alert">
          {error}
        </div>
      ) : null}
    </div>
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
  const [name, setName] = useState("");
  const [provider, setProvider] = useState("");
  const [error, setError] = useState<string | null>(null);
  const taken = new Set(section.instances.map((row) => row.name));
  const providerChoices = providers ?? [];
  const blocked = info.needsProvider === true && !providerChoices.length;

  const submit = () => {
    const finalName = name.trim();
    if (takesName && !finalName) {
      setError("给它起个名字");
      return;
    }
    if (takesName && taken.has(finalName)) {
      setError("已经有叫这个名字的实例了，换一个吧");
      return;
    }
    if (info.needsProvider && !provider) {
      setError("选一下它连哪家的模型");
      return;
    }
    setError(null);
    void run(`${section.vendor}:create`, () =>
      create(section.vendor, { name: finalName, ...(info.needsProvider ? { provider } : {}) }),
    ).then((ok) => ok && onDone());
  };

  return (
    <Row
      title={`新建 ${info.label} 实例`}
      subtitle="建好后直接开始登录。并发上限在详情里设。"
      controls={
        <>
          <button type="button" className="ghost" disabled={busy !== null || blocked} onClick={submit}>
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
            placeholder="名字，比如：Plus 1 号"
            aria-label="名字"
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") submit();
              if (event.key === "Escape") onDone();
            }}
          />
        ) : null}
        {info.needsProvider ? (
          providerChoices.length ? (
            <select
              className="settings-select"
              value={provider}
              aria-label="连哪家的模型"
              onChange={(event) => setProvider(event.target.value)}
            >
              <option value="">连哪家的模型</option>
              {providerChoices.map((option) => (
                <option key={option} value={option}>
                  连 {option}
                </option>
              ))}
            </select>
          ) : (
            <span className="settings-help">还不知道 {info.label} 能连哪家的模型，暂时建不了。</span>
          )
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
