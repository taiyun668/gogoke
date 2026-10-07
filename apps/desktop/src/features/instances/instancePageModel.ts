import type { ManagedInstancePage } from "@/services/design37ManagedInstances";

/**
 * View model for Settings > 实例. One section per vendor: gogoke's own CLI copy
 * first, then one row per instance (one account each).
 *
 * Display layers (Owner 2026-10-06):
 * - layer 1, always visible: can it be used, who uses it, does the Owner need to act;
 * - layer 2, behind 详情: account, models, cap, CLI version, capabilities, log, raw text;
 * - layer 3, never shown: internal ids, paths, digests, revisions.
 * Fields the host does not report stay undefined and are shown as unknown or
 * hidden, never guessed.
 */

export type VendorId = "codex" | "claude" | "opencode" | "grok" | "antigravity";

export type CliState =
  | "NOT_INSTALLED"
  | "DOWNLOADING"
  | "INSTALLING"
  | "INSTALL_FAILED"
  /** Downloaded but not activated; the original install can continue. */
  | "STAGED"
  /** Self-tested but not activated; the original install can continue. */
  | "PROBED"
  /** The self-test stopped without a settled result; not installed, not a network failure. */
  | "PROBE_UNKNOWN"
  | "BLOCKED"
  | "READY"
  | "UPGRADING"
  | "UPGRADE_FAILED"
  | "UNINSTALLING";

export type CliCopy = {
  state: CliState;
  version?: string;
  /** A newer version gogoke has verified; the only source of an upgrade offer. */
  verifiedVersion?: string;
  /** Newer official version gogoke has not verified; shown in details only. */
  officialVersion?: string;
  previousVersion?: string;
  checkedAt?: string;
  /** Bytes downloaded so far; not a percentage. */
  progressBytes?: number;
  raw?: string;
};

export type InstanceState =
  /** The CLI this instance runs on is not installed; it cannot log in yet. */
  | "CLI_MISSING"
  | "NOT_LOGGED_IN"
  | "LOGGING_IN"
  | "LOGIN_FAILED"
  | "WRONG_ACCOUNT"
  | "LOGIN_UNKNOWN"
  | "READY"
  /** Logged in, but its name, enabled state or cap is not set; it gets no work yet. */
  | "CONFIG_UNKNOWN"
  | "EXPIRED"
  | "ERROR";

export type InstanceLogin = {
  deviceCode?: string;
  authorizationUrl?: string;
  /** "opened" means the host asked the browser to open the page, not that it loaded. */
  browser: "opened" | "failed" | "not-requested";
  startedAt: number;
};

/** A problem in one seat's session; it does not change the instance's own state. `seat` is an id, never shown. */
export type SeatIssue = { seat: string; reason: string };

export type InstanceRow = {
  /** Host identifier; used for actions only, never rendered. */
  id: string;
  /** The stored name, or a neutral display name while none is stored. */
  name: string;
  state: InstanceState;
  /** Unknown until the host has a profile; no enable/disable is offered then. */
  enabled?: boolean;
  /** The host can take rename/enable/disable writes for this instance. */
  profileReady?: boolean;
  account?: string;
  plan?: string;
  /** For CLIs that do not ship their own models (OpenCode). */
  provider?: string;
  otherAccount?: string;
  /** Seats using it; undefined when the host has not settled who uses it. */
  seats?: string[];
  runningSessions?: number;
  cap?: number;
  models?: string;
  modelsSource?: string;
  modelsObservedAt?: string;
  lastConfirmed?: string;
  /** Set when the latest check failed; the state shown is the last confirmed one. */
  checkFailed?: string;
  /** A previous unclean exit was settled from holder-gone proof. */
  settledLeftover?: boolean;
  quotaResets?: string;
  raw?: string;
  seatIssues?: SeatIssue[];
  login?: InstanceLogin;
  /** The host still holds an unsettled login request; only cancel is offered. */
  loginUnsettled?: boolean;
  /** One-line outcome of the last login request, e.g. cancelled. */
  loginNote?: string;
  log?: Array<[string, string]>;
};

export type VendorSection = {
  vendor: VendorId;
  cli?: CliCopy;
  instances: InstanceRow[];
};

export type InstancePage = { sections: VendorSection[] };

export type VendorInfo = {
  label: string;
  steer: string;
  questions: string;
  login: string;
  /** Present when gogoke cannot keep one account per instance for this vendor. */
  unsupported?: string;
  /** The CLI connects to another vendor's models. */
  needsProvider?: boolean;
};

export const VENDORS: Record<VendorId, VendorInfo> = {
  codex: { label: "Codex", steer: "原生插话", questions: "原生问题卡", login: "设备码" },
  claude: { label: "Claude Code", steer: "原生插话", questions: "原生问题卡", login: "浏览器授权" },
  opencode: {
    label: "OpenCode",
    steer: "打断后续接",
    questions: "gogoke 问题卡",
    login: "设备码",
    needsProvider: true,
  },
  grok: { label: "Grok Build", steer: "打断后续接", questions: "gogoke 问题卡", login: "浏览器授权" },
  antigravity: {
    label: "Antigravity",
    steer: "—",
    questions: "—",
    login: "—",
    unsupported: "做不到一个实例一个账号，所以暂时不支持",
  },
};

export const VENDOR_ORDER: VendorId[] = ["codex", "claude", "opencode", "grok", "antigravity"];

export const NEEDS_OWNER: ReadonlySet<InstanceState> = new Set([
  "CLI_MISSING",
  "CONFIG_UNKNOWN",
  "LOGIN_FAILED",
  "WRONG_ACCOUNT",
  "LOGIN_UNKNOWN",
  "EXPIRED",
  "ERROR",
]);

const CLI_NEEDS_OWNER: ReadonlySet<CliState> = new Set(["INSTALL_FAILED", "BLOCKED", "STAGED", "PROBED", "PROBE_UNKNOWN"]);

export type Tone = "ok" | "warn" | "err" | "busy" | "idle";

/** What the page can offer, from the host's actions; text never promises an absent one. */
export type Capabilities = { canCheck: boolean };

export function cliUsable(cli: CliCopy | undefined): boolean {
  return cli !== undefined && ["READY", "UPGRADING", "UPGRADE_FAILED"].includes(cli.state);
}

const seatShortName = (seat: string) => seat.split(" / ")[1] ?? seat;
export const seatNames = (row: InstanceRow) => (row.seats ?? []).map(seatShortName).join("、");

/** Layer-1 line for an instance: one tone dot and one plain sentence. */
export function instanceSummary(row: InstanceRow, can: Capabilities = { canCheck: false }): { tone: Tone; text: string } {
  if (row.enabled === false) return { tone: "idle", text: "已停用，主控不会派活给它" };
  switch (row.state) {
    case "READY": {
      if (row.seats === undefined) return { tone: "ok", text: "可以用" };
      const use = row.seats.length
        ? `${seatNames(row)} 在用${row.runningSessions ? ` · ${row.runningSessions} 个会话在跑` : ""}`
        : "空闲";
      return { tone: "ok", text: `可以用 · ${use}` };
    }
    case "CONFIG_UNKNOWN":
      return { tone: "warn", text: "登上了，但名字、启用或并发上限还没设好，设好之前主控不会派活给它" };
    case "CLI_MISSING":
      return { tone: "warn", text: "它要用的 CLI 还没装，装好才能登录" };
    case "NOT_LOGGED_IN":
      return { tone: "idle", text: row.loginNote ?? "还没登录" };
    case "LOGGING_IN":
      return { tone: "busy", text: "正在登录" };
    case "LOGIN_FAILED":
      return { tone: "err", text: "这次没登上" };
    case "WRONG_ACCOUNT":
      return {
        tone: "err",
        text: `登成了另一个账号${row.otherAccount ? `（${row.otherAccount}）` : ""}，没有采用，原来的登录不变`,
      };
    case "LOGIN_UNKNOWN":
      return {
        tone: "warn",
        text: can.canCheck ? "没能确认登没登上，先不当成登上了。检测一下，不花额度" : "没能确认登没登上，先不当成登上了",
      };
    case "EXPIRED":
      return { tone: "warn", text: "登录过期了 · 用同一个账号重新登一次就好，会话记录都还在" };
    case "ERROR": {
      const head = row.quotaResets ? `额度用完了，${row.quotaResets}` : "出错了";
      const seats = row.seats?.length ? ` · ${seatNames(row)} 先停在当前这一轮` : "";
      return { tone: "err", text: `${head}${seats}` };
    }
  }
}

export type PrimaryAction = "login" | "relogin" | "login-same-account" | "check" | "cancel-login" | "enable" | "configure";

/** The one button shown for the current state; everything else lives in the 更多 menu. */
export function primaryAction(row: InstanceRow): PrimaryAction | null {
  if (row.loginUnsettled) return "cancel-login";
  if (row.enabled === false) return "enable";
  switch (row.state) {
    case "NOT_LOGGED_IN":
      return "login";
    case "LOGGING_IN":
      return "cancel-login";
    case "LOGIN_FAILED":
    case "EXPIRED":
      return "relogin";
    case "WRONG_ACCOUNT":
      return "login-same-account";
    case "LOGIN_UNKNOWN":
    case "ERROR":
      return "check";
    case "READY":
      return row.checkFailed ? "check" : null;
    case "CONFIG_UNKNOWN":
      return "configure";
    case "CLI_MISSING":
      return null;
  }
}

function megabytes(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function cliSummary(cli: CliCopy, runningInVendor: number): { tone: Tone; text: string } {
  switch (cli.state) {
    case "NOT_INSTALLED":
      return { tone: "idle", text: "还没装。装在 gogoke 自己的目录里，不碰你平时用的那份" };
    case "DOWNLOADING":
      return { tone: "busy", text: `正在下载${cli.progressBytes !== undefined ? ` · 已下载 ${megabytes(cli.progressBytes)}` : ""}` };
    case "INSTALLING":
      return { tone: "busy", text: "正在安装" };
    case "INSTALL_FAILED":
      return { tone: "err", text: "没装成" };
    case "STAGED":
      return { tone: "warn", text: "下载好了，还没装完" };
    case "PROBED":
      return { tone: "warn", text: "装好也自检过了，还没启用" };
    case "PROBE_UNKNOWN":
      return { tone: "warn", text: "自检停下了，没能确认通没通过，先不当成装好了" };
    case "BLOCKED":
      return { tone: "err", text: "装好了，但 Windows 不让它启动，这家的实例暂时都用不了" };
    case "UPGRADING":
      return { tone: "busy", text: "正在升级。升好之前旧版本照常保留，这家暂不开新会话" };
    case "UPGRADE_FAILED":
      return { tone: "warn", text: "升级没成功，还在用原来的版本，一切照常" };
    case "UNINSTALLING":
      return { tone: "busy", text: "正在卸载" };
    case "READY":
      if (!cli.verifiedVersion) return { tone: "ok", text: "正常" };
      return {
        tone: "ok",
        text: `有新版本可以升级，会影响这家所有实例${
          runningInVendor ? `。有 ${runningInVendor} 个会话在跑，停下后才能升` : ""
        }`,
      };
  }
}

export function runningSessions(section: VendorSection): number {
  return section.instances.reduce((sum, row) => sum + (row.runningSessions ?? 0), 0);
}

export function pageSummary(page: InstancePage): { usable: number; disabled: number; needsOwner: string[] } {
  let usable = 0;
  let disabled = 0;
  const needsOwner: string[] = [];
  for (const section of page.sections) {
    for (const row of section.instances) {
      if (row.enabled === false) disabled += 1;
      else if (row.state === "READY") usable += 1;
      if (row.enabled !== false && NEEDS_OWNER.has(row.state)) needsOwner.push(row.name);
    }
    if (section.cli && CLI_NEEDS_OWNER.has(section.cli.state)) {
      needsOwner.push(`${VENDORS[section.vendor].label} CLI`);
    }
  }
  return { usable, disabled, needsOwner };
}

/** Project the host's managed instance page; every field is the host's or absent. */
export function pageFromManaged(page: ManagedInstancePage): InstancePage {
  return {
    sections: VENDOR_ORDER.map((vendor) => {
      const section = page.sections.find((item) => item.vendor === vendor);
      return {
        vendor,
        cli: section?.cli ? { ...section.cli } : undefined,
        instances: (section?.instances ?? []).map((row) => ({ ...row })),
      };
    }),
  };
}
