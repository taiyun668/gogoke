import type {
  Design37Instance,
  Design37InstancesSnapshot,
} from "@/features/seats/design37Instances";

/**
 * View model for Settings > 实例. One section per vendor: the vendor's own CLI
 * copy first, then one row per instance (one account each).
 *
 * Display layers (Owner 2026-10-06):
 * - layer 1, always visible: can it be used, who uses it, does the Owner need to act;
 * - layer 2, behind 详情: account, models, cap, CLI version, capabilities, log, raw text;
 * - layer 3, never shown: internal ids, paths, digests, revisions.
 * Fields the host does not report stay undefined and are shown as unknown, never guessed.
 */

export type VendorId = "codex" | "claude" | "opencode" | "grok" | "antigravity";

export type CliState =
  | "NOT_INSTALLED"
  | "INSTALLING"
  | "INSTALL_FAILED"
  | "BLOCKED"
  | "READY"
  | "UPGRADING"
  | "UPGRADE_FAILED";

export type CliCopy = {
  state: CliState;
  version?: string;
  /** A newer version gogoke has verified; the only source of an upgrade offer. */
  verifiedVersion?: string;
  /** Newer official version gogoke has not verified; shown in details only. */
  officialVersion?: string;
  previousVersion?: string;
  checkedAt?: string;
  progress?: number;
  raw?: string;
};

export type InstanceState =
  | "NOT_LOGGED_IN"
  | "LOGGING_IN"
  | "LOGIN_FAILED"
  | "WRONG_ACCOUNT"
  | "LOGIN_UNKNOWN"
  | "READY"
  | "EXPIRED"
  | "ERROR";

export type InstanceLogin = {
  deviceCode?: string;
  authorizationUrl?: string;
  browserOpened: boolean;
  startedAt: number;
};

export type InstanceRow = {
  /** Host identifier; used for actions only, never rendered. */
  id: string;
  name: string;
  state: InstanceState;
  enabled: boolean;
  account?: string;
  plan?: string;
  /** For CLIs that do not ship their own models (OpenCode). */
  provider?: string;
  otherAccount?: string;
  seats: string[];
  runningSessions?: number;
  cap?: number;
  models?: string;
  lastConfirmed?: string;
  /** Set when the latest check failed; the state shown is the last confirmed one. */
  checkFailed?: string;
  /** A previous unclean exit was settled from holder-gone proof. */
  settledLeftover?: boolean;
  quotaResets?: string;
  raw?: string;
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
  "LOGIN_FAILED",
  "WRONG_ACCOUNT",
  "LOGIN_UNKNOWN",
  "EXPIRED",
  "ERROR",
]);

export type Tone = "ok" | "warn" | "err" | "busy" | "idle";

export function cliUsable(cli: CliCopy | undefined): boolean {
  return cli !== undefined && ["READY", "UPGRADING", "UPGRADE_FAILED"].includes(cli.state);
}

const seatShortName = (seat: string) => seat.split(" / ")[1] ?? seat;
export const seatNames = (row: InstanceRow) => row.seats.map(seatShortName).join("、");

/** Layer-1 line for an instance: one tone dot and one plain sentence. */
export function instanceSummary(row: InstanceRow): { tone: Tone; text: string } {
  if (!row.enabled) return { tone: "idle", text: "已停用，主控不会派活给它" };
  switch (row.state) {
    case "READY": {
      const use = row.seats.length
        ? `${seatNames(row)} 在用${row.runningSessions ? ` · ${row.runningSessions} 个会话在跑` : ""}`
        : "空闲";
      return { tone: "ok", text: `可以用 · ${use}` };
    }
    case "NOT_LOGGED_IN":
      return { tone: "idle", text: row.loginNote ?? "还没登录。点登录会打开浏览器，授权一下就行" };
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
      return { tone: "warn", text: "没能确认登没登上，先不当成登上了。检测一下，不花额度" };
    case "EXPIRED":
      return { tone: "warn", text: "登录过期了 · 用同一个账号重新登一次就好，会话记录都还在" };
    case "ERROR": {
      const head = row.quotaResets ? `额度用完了，${row.quotaResets}` : "出错了";
      const seats = row.seats.length ? ` · ${seatNames(row)} 先停在当前这一轮` : "";
      return { tone: "err", text: `${head}${seats}` };
    }
  }
}

export type PrimaryAction = "login" | "relogin" | "login-same-account" | "check" | "cancel-login" | "enable";

/** The one button shown for the current state; everything else lives in the 更多 menu. */
export function primaryAction(row: InstanceRow): PrimaryAction | null {
  if (row.loginUnsettled) return "cancel-login";
  if (!row.enabled) return "enable";
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
  }
}

export function cliSummary(cli: CliCopy, runningInVendor: number): { tone: Tone; text: string } {
  switch (cli.state) {
    case "NOT_INSTALLED":
      return { tone: "idle", text: "还没装。装在 gogoke 自己的目录里，不碰你平时用的那份" };
    case "INSTALLING":
      return { tone: "busy", text: "正在安装，通常一两分钟" };
    case "INSTALL_FAILED":
      return { tone: "err", text: "没装成，多半是网络问题" };
    case "BLOCKED":
      return { tone: "err", text: "装好了，但 Windows 不让它启动，这家的实例暂时都用不了" };
    case "UPGRADING":
      return { tone: "busy", text: "正在升级。升好之前旧版本照常保留，这家暂不开新会话" };
    case "UPGRADE_FAILED":
      return { tone: "warn", text: "升级没成功，还在用原来的版本，一切照常" };
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
      if (!row.enabled) disabled += 1;
      else if (row.state === "READY") usable += 1;
      if (row.enabled && NEEDS_OWNER.has(row.state)) needsOwner.push(row.name);
    }
    if (section.cli && ["INSTALL_FAILED", "BLOCKED"].includes(section.cli.state)) {
      needsOwner.push(`${VENDORS[section.vendor].label} CLI`);
    }
  }
  return { usable, disabled, needsOwner };
}

function vendorOf(driverId: string): VendorId | null {
  return (VENDOR_ORDER as string[]).includes(driverId) ? (driverId as VendorId) : null;
}

/**
 * Adapt today's host snapshot (gogoke.37.instance-page.v1). It reports state,
 * version, a verified newer version, login progress and per-seat runtime issues;
 * account, models, cap, seats and enable state are not reported yet and stay unknown.
 */
export function pageFromDesign37(snapshot: Design37InstancesSnapshot): InstancePage {
  const sections = new Map<VendorId, VendorSection>();
  for (const vendor of VENDOR_ORDER) sections.set(vendor, { vendor, instances: [] });
  for (const instance of snapshot.instances) {
    const vendor = vendorOf(instance.driverId);
    if (!vendor) continue;
    const section = sections.get(vendor)!;
    if (instance.state === "NOT_INSTALLED") {
      section.cli = { state: "NOT_INSTALLED" };
    } else if (!section.cli || section.cli.state === "NOT_INSTALLED") {
      section.cli = { state: "READY", version: instance.version, verifiedVersion: instance.newVersion };
    }
    section.instances.push(rowFromDesign37(instance));
  }
  return { sections: VENDOR_ORDER.map((vendor) => sections.get(vendor)!) };
}

function rowFromDesign37(instance: Design37Instance): InstanceRow {
  const login = instance.login;
  const issues = instance.runtimeIssues ?? [];
  const row: InstanceRow = {
    id: instance.instanceId,
    name: instance.instanceId,
    state: "NOT_LOGGED_IN",
    enabled: true,
    seats: [],
  };
  if (login && login.state === "PENDING" && !login.settled) {
    row.state = "LOGGING_IN";
    row.login = {
      deviceCode: login.deviceCode,
      authorizationUrl: login.authorizationUrl,
      browserOpened: login.browserState !== "FAILED",
      startedAt: login.startedAt,
    };
    return row;
  }
  if (login?.state === "ERROR") {
    row.state = "LOGIN_FAILED";
    row.raw = login.error || login.output || undefined;
    row.loginUnsettled = !login.settled;
    return row;
  }
  if (login?.state === "CANCELLED" && instance.state !== "LOGGED_IN") row.loginNote = "上次登录已取消，可以重新登录";
  if (login?.state === "UNKNOWN") {
    row.state = "LOGIN_UNKNOWN";
    return row;
  }
  switch (instance.state) {
    case "LOGGED_IN":
      row.state = issues.length ? "ERROR" : "READY";
      break;
    case "ERROR":
      row.state = "ERROR";
      break;
    default:
      row.state = "NOT_LOGGED_IN";
  }
  if (issues.length) row.raw = issues.map((issue) => issue.reason).join("\n");
  return row;
}
