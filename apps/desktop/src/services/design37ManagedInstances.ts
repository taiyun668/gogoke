import { invoke } from "@tauri-apps/api/core";
import { readDesign37InstancesSnapshot, type Design37Instance } from "@/features/seats/design37Instances";
import { design37UserConfiguration } from "./tauri";

type Vendor = "codex" | "claude" | "opencode" | "grok";
const VENDORS: readonly Vendor[] = ["codex", "claude", "opencode", "grok"];
const LABEL: Record<Vendor, string> = {
  codex: "Codex", claude: "Claude Code", opencode: "OpenCode", grok: "Grok Build",
};
type CliState = "NOT_INSTALLED" | "DOWNLOADING" | "INSTALLING" | "INSTALL_FAILED" |
  "STAGED" | "PROBED" | "PROBE_UNKNOWN" | "BLOCKED" | "READY" | "UPGRADING" |
  "UPGRADE_FAILED" | "UNINSTALLING";
const CLI_STATES: readonly CliState[] = ["NOT_INSTALLED", "DOWNLOADING", "INSTALLING",
  "INSTALL_FAILED", "STAGED", "PROBED", "PROBE_UNKNOWN", "BLOCKED", "READY",
  "UPGRADING", "UPGRADE_FAILED", "UNINSTALLING"];

export type ManagedCliView = {
  state: CliState;
  version?: string;
  previousVersion?: string;
  officialVersion?: string;
  checkedAt?: string;
  progressBytes?: number;
  raw?: string;
};
export type ManagedInstanceView = {
  id: string;
  name: string;
  state: "CLI_MISSING" | "NOT_LOGGED_IN" | "LOGGING_IN" | "LOGIN_FAILED" |
    "LOGIN_UNKNOWN" | "READY" | "CONFIG_UNKNOWN" | "ERROR";
  enabled?: boolean;
  /** UI may show profile writes only when native has a CAS revision and fields. */
  profileReady: boolean;
  account?: string;
  plan?: string;
  provider?: string;
  cap?: number;
  models?: string;
  modelsSource?: string;
  modelsObservedAt?: string;
  lastConfirmed?: string;
  checkFailed?: string;
  raw?: string;
  login?: { deviceCode?: string; authorizationUrl?: string;
    browser: "opened" | "failed" | "not-requested"; startedAt: number };
  loginUnsettled?: boolean;
  loginNote?: string;
  // No E/H occupancy read is wired here; absence is unknown, never zero.
  seats?: string[];
  runningSessions?: number;
};
export type ManagedInstancePage = {
  sections: Array<{ vendor: Vendor | "antigravity"; cli?: ManagedCliView;
    instances: ManagedInstanceView[] }>;
};

type Profile = {
  instanceId: string; driverId: Vendor; name?: string; enabled?: boolean;
  provider?: string; profileRevision?: string; cap?: number; account?: string;
  plan?: string; lastConfirmed?: string; checkFailed?: string;
  models?: string[]; modelsSource?: string; modelsObservedAt?: string;
};
type Management = { profiles: Profile[]; cli: Map<Vendor, ManagedCliView> };
const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const vendor = (value: unknown): value is Vendor =>
  typeof value === "string" && (VENDORS as readonly string[]).includes(value);
function optionalString(value: unknown, name: string): string | undefined {
  if (value === null || value === undefined) return undefined;
  if (typeof value !== "string") throw new Error(`Invalid native ${name}.`);
  return value || undefined;
}
function positiveInteger(value: unknown, name: string): number | undefined {
  if (value === null || value === undefined) return undefined;
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 1) {
    throw new Error(`Invalid native ${name}.`);
  }
  return value;
}
function hostTime(value: unknown, name: string): string | undefined {
  const raw = optionalString(value, name);
  if (raw === undefined) return undefined;
  if (!/^[1-9][0-9]*$/.test(raw)) throw new Error(`Invalid native ${name}.`);
  const ms = Number(raw);
  if (!Number.isSafeInteger(ms) || !Number.isFinite(new Date(ms).getTime())) {
    throw new Error(`Invalid native ${name}.`);
  }
  return new Date(ms).toLocaleString("zh-CN");
}
function parseManagement(value: unknown): Management {
  if (!object(value) || value.schema !== "gogoke.37.instance-management.v1" ||
      !Array.isArray(value.profiles) || !Array.isArray(value.cli)) {
    throw new Error("Native instance management schema is unavailable.");
  }
  const profiles: Profile[] = [];
  const seen = new Set<string>();
  for (const item of value.profiles) {
    if (!object(item) || typeof item.instanceId !== "string" || !item.instanceId ||
        !vendor(item.driverId) || seen.has(item.instanceId)) {
      throw new Error("Invalid native instance profile.");
    }
    seen.add(item.instanceId);
    const enabled = item.enabled;
    if (enabled !== null && enabled !== undefined && typeof enabled !== "boolean") {
      throw new Error("Invalid native instance enabled state.");
    }
    const models = item.models;
    if (models !== undefined && (!Array.isArray(models) ||
        !models.every((model) => typeof model === "string" && model.length > 0))) {
      throw new Error("Invalid native verified model list.");
    }
    profiles.push({
      instanceId: item.instanceId, driverId: item.driverId,
      name: optionalString(item.name, "name"),
      enabled: enabled === null ? undefined : enabled as boolean | undefined,
      provider: optionalString(item.provider, "provider"),
      profileRevision: optionalString(item.profileRevision, "profile revision"),
      cap: positiveInteger(item.cap, "concurrency cap"),
      account: optionalString(item.account, "masked account"),
      plan: optionalString(item.plan, "account plan"),
      lastConfirmed: hostTime(item.lastConfirmed, "account confirmation time"),
      checkFailed: optionalString(item.checkFailed, "check failure"),
      models: models as string[] | undefined,
      modelsSource: optionalString(item.modelsSource, "model source"),
      modelsObservedAt: hostTime(item.modelsObservedAt, "model observation time"),
    });
  }
  const cli = new Map<Vendor, ManagedCliView>();
  for (const item of value.cli) {
    if (!object(item) || !vendor(item.driverId) || cli.has(item.driverId) ||
        typeof item.state !== "string" || !(CLI_STATES as readonly string[]).includes(item.state)) {
      throw new Error("Invalid native managed CLI copy.");
    }
    cli.set(item.driverId, {
      state: item.state as CliState,
      version: optionalString(item.version, "CLI version"),
      previousVersion: optionalString(item.previousVersion, "previous CLI version"),
      officialVersion: optionalString(item.officialVersion, "official version notice"),
      checkedAt: hostTime(item.checkedAt, "CLI check time"),
      progressBytes: item.progressBytes === 0 ? 0 : positiveInteger(item.progressBytes, "CLI bytes"),
      raw: optionalString(item.raw, "CLI raw error"),
    });
  }
  return { profiles, cli };
}

async function readManagement(): Promise<Management> {
  return parseManagement(await design37UserConfiguration<unknown>("instance-management-read"));
}
function fromLogin(instance: Design37Instance, profile: Profile): ManagedInstanceView["state"] {
  if (instance.state === "NOT_INSTALLED") return "CLI_MISSING";
  const login = instance.login;
  if (login?.state === "PENDING" && !login.settled) return "LOGGING_IN";
  if (login?.state === "ERROR") return "LOGIN_FAILED";
  if (login?.state === "UNKNOWN") return "LOGIN_UNKNOWN";
  if (instance.state === "ERROR") return "ERROR";
  if (instance.state === "LOGGED_IN") {
    return profile.enabled === true && profile.cap !== undefined ? "READY" : "CONFIG_UNKNOWN";
  }
  return "NOT_LOGGED_IN";
}
function joinedRow(instance: Design37Instance | undefined, profile: Profile, ordinal: number): ManagedInstanceView {
  const login = instance?.login;
  const row: ManagedInstanceView = {
    id: profile.instanceId,
    name: profile.name ?? `${LABEL[profile.driverId]} 实例${ordinal > 1 ? ` ${ordinal}` : ""}`,
    state: instance ? fromLogin(instance, profile) : "LOGIN_UNKNOWN",
    enabled: profile.enabled,
    profileReady: profile.profileRevision !== undefined &&
      profile.name !== undefined && profile.enabled !== undefined,
    account: profile.account, plan: profile.plan, provider: profile.provider,
    cap: profile.cap,
    models: profile.models === undefined ? undefined :
      profile.models.length ? profile.models.join("、") : "已验证无可用模型",
    modelsSource: profile.modelsSource, modelsObservedAt: profile.modelsObservedAt,
    lastConfirmed: profile.lastConfirmed, checkFailed: profile.checkFailed,
  };
  if (login?.state === "PENDING" && !login.settled) {
    row.login = {
      deviceCode: login.deviceCode, authorizationUrl: login.authorizationUrl,
      browser: { OPENED: "opened", FAILED: "failed", NOT_REQUESTED: "not-requested" }[login.browserState],
      startedAt: login.startedAt,
    };
    row.loginUnsettled = true;
  } else if (login?.state === "ERROR") {
    row.raw = login.error || login.output || undefined;
    row.loginUnsettled = !login.settled;
  } else if (login?.state === "CANCELLED" && instance?.state !== "LOGGED_IN") {
    row.loginNote = "上次登录已取消，可以重新登录";
  }
  return row;
}
async function readPage(): Promise<ManagedInstancePage> {
  const snapshot = readDesign37InstancesSnapshot(await invoke<unknown>("gogoke_design37_instances"));
  const management = await readManagement();
  const instances = new Map(snapshot.instances.map((row) => [row.instanceId, row]));
  const sections: ManagedInstancePage["sections"] = VENDORS.map((item) => ({
    vendor: item,
    cli: management.cli.get(item),
    instances: [],
  }));
  sections.push({ vendor: "antigravity", instances: [] });
  for (const profile of management.profiles) {
    const section = sections.find((item) => item.vendor === profile.driverId)!;
    section.instances.push(joinedRow(instances.get(profile.instanceId), profile, section.instances.length + 1));
  }
  return { sections };
}

async function currentProfile(id: string): Promise<Profile> {
  const profile = (await readManagement()).profiles.find((item) => item.instanceId === id);
  if (!profile) throw new Error("Native instance is absent or already removed.");
  return profile;
}
async function updateProfile(id: string, change: (current: Profile) => Pick<Profile,"name"|"enabled">): Promise<void> {
  const current = await currentProfile(id);
  const next = change(current);
  if (!current.profileRevision || !next.name || next.enabled === undefined) {
    throw new Error("Native instance profile is not configured for this action.");
  }
  await design37UserConfiguration("instance-profile", {
    instanceId: id, name: next.name, enabled: next.enabled,
    provider: current.provider ?? null,
    expectedProfileRevision: current.profileRevision,
    requestId: `ui-${crypto.randomUUID()}`,
  });
}

/** USER bridge only. Unsupported actions are absent rather than advertised. */
export function createDesign37ManagedInstanceSource<Page>(project: (value: ManagedInstancePage) => Page) {
  return {
    read: async (): Promise<Page> => project(await readPage()),
    actions: {
      login: async (id: string): Promise<void> => {
        await invoke("gogoke_design37_instance_login", { instanceId: id });
      },
      cancelLogin: async (id: string): Promise<void> => {
        await invoke("gogoke_design37_instance_cancel", { instanceId: id });
      },
      rename: (id: string, name: string) => updateProfile(id, (current) => ({
        name, enabled: current.enabled,
      })),
      enable: (id: string) => updateProfile(id, (current) => ({
        name: current.name, enabled: true,
      })),
      disable: (id: string) => updateProfile(id, (current) => ({
        name: current.name, enabled: false,
      })),
      setCap: async (id: string, cap: number): Promise<void> => {
        if (!Number.isSafeInteger(cap) || cap < 1) throw new Error("Invalid instance capacity.");
        await currentProfile(id);
        await design37UserConfiguration("instance-concurrency-cap", { instanceId: id, value: cap });
      },
      installCli: async (driverId: string): Promise<void> => {
        if (!vendor(driverId)) throw new Error("Unsupported managed CLI.");
        await invoke("gogoke_design37_install_cli", {
          request: { driverId, requestId: `ui-${crypto.randomUUID()}` },
        });
      },
    },
  };
}
