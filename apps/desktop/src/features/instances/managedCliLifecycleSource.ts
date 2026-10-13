import { design37ExactManagedCliFrame, design37ManagedCliAction, design37UserFrame } from "@/services/tauri";
import { createDesign37ManagedInstanceSource } from "@/services/design37ManagedInstances";
import { pageFromManaged, type CliCopy, type CliState, type InstancePage, type VendorId } from "./instancePageModel";
import type { InstancePageSource } from "./InstancesPage";

type Driver = Exclude<VendorId, "antigravity">;
type Switch = "upgrade" | "rollback";
type Pending = { operation: Switch; frame: string; requestId: string; targetVersion: string };
// This map survives a page/source recreation in this UI process. It is deliberately not durable.
const pending = new Map<Driver, Pending>();
const drivers: Driver[] = ["codex", "claude", "opencode", "grok"];
const states: CliState[] = ["NOT_INSTALLED", "DOWNLOADING", "INSTALLING", "INSTALL_FAILED", "STAGED", "PROBED", "PROBE_UNKNOWN", "BLOCKED", "READY", "UPGRADING", "UPGRADE_FAILED", "UNINSTALLING"];
const object = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const revision = (v: unknown): v is string => typeof v === "string" && /^[1-9][0-9]*$/.test(v);
const version = (v: unknown): v is string => typeof v === "string" && /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(v);
const optional = (v: unknown, valid: (v: unknown) => boolean): boolean => v === undefined || v === null || valid(v);
const nullableVersion = (v: unknown): v is string | null => v === null || version(v);
const nonnegative = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;

function compareVersion(a: string, b: string): number {
  const left = a.split(".").map(BigInt), right = b.split(".").map(BigInt);
  for (let i = 0; i < 3; i++) {
    if (left[i] > right[i]) return 1;
    if (left[i] < right[i]) return -1;
  }
  return 0;
}

type Status = { state: CliState; revision: string | null; version: string | null;
  preparedReplacement: boolean | null;
  previousVersion: string | null; verifiedOtherVersions: string[]; officialVersion: string | null;
  checkedAt: string | null; progressBytes?: number; raw: string | null };

function parseStatus(value: unknown, driverId: Driver): Status {
  if (!object(value) || value.schema !== "gogoke.37.managed-cli.v1" || value.command !== "status" ||
      value.driverId !== driverId || !states.includes(value.state as CliState) ||
      !optional(value.previousVersion, nullableVersion) ||
      !optional(value.officialVersion, (v) => v === null || (typeof v === "string" && v.length > 0)) ||
      !optional(value.checkedAt, revision) || !optional(value.progressBytes, nonnegative) ||
      ![true, false, null].includes(value.preparedReplacement as boolean | null) ||
      !optional(value.raw, (v) => typeof v === "string") ||
      (value.verifiedOtherVersions !== undefined &&
       (!Array.isArray(value.verifiedOtherVersions) || !value.verifiedOtherVersions.every(version) ||
        new Set(value.verifiedOtherVersions).size !== value.verifiedOtherVersions.length))) {
    throw new Error(`Invalid native ${driverId} CLI status.`);
  }
  if (value.state === "NOT_INSTALLED") {
    if ((value.revision !== null && !revision(value.revision)) ||
        value.version !== null || value.preparedReplacement !== null) {
      throw new Error(`Invalid native ${driverId} CLI absence.`);
    }
  } else if (!revision(value.revision) || !nullableVersion(value.version)) {
    throw new Error(`Invalid native ${driverId} CLI version or revision.`);
  }
  return { state: value.state as CliState, revision: value.revision as string | null,
    preparedReplacement: value.preparedReplacement as boolean | null,
    version: value.version as string | null, previousVersion: (value.previousVersion ?? null) as string | null,
    verifiedOtherVersions: (value.verifiedOtherVersions ?? []) as string[],
    officialVersion: (value.officialVersion ?? null) as string | null,
    checkedAt: (value.checkedAt ?? null) as string | null,
    ...(value.progressBytes === undefined || value.progressBytes === null ? {} : { progressBytes: value.progressBytes as number }),
    raw: (value.raw ?? null) as string | null };
}

async function status(driverId: Driver): Promise<Status> {
  return parseStatus(await design37UserFrame({ schema: "gogoke.37.managed-cli.v1", command: "status", driverId }), driverId);
}

function target(s: Status, operation: Switch): string | null {
  if (s.preparedReplacement === null || !s.version) return null;
  if (s.preparedReplacement) {
    if (!["STAGED", "PROBED", "BLOCKED", "UPGRADE_FAILED", "UPGRADING"].includes(s.state) || !s.previousVersion) return null;
    const direction = compareVersion(s.version, s.previousVersion);
    return (operation === "upgrade" ? direction > 0 : direction < 0) ? s.version : null;
  }
  if (s.state !== "READY") return null;
  if (operation === "rollback") return s.previousVersion && compareVersion(s.previousVersion, s.version) < 0 ? s.previousVersion : null;
  return s.verifiedOtherVersions.filter((v) => compareVersion(v, s.version!) > 0)
    .sort((a, b) => compareVersion(b, a))[0] ?? null;
}

function display(s: Status, driverId: Driver): CliCopy {
  const millis = s.checkedAt === null ? null : Number(s.checkedAt);
  if (millis !== null && (!Number.isSafeInteger(millis) || !Number.isFinite(new Date(millis).getTime()))) {
    throw new Error(`Invalid native ${driverId} CLI check time.`);
  }
  const checkedAt = millis === null ? undefined : new Date(millis).toLocaleString("zh-CN");
  return { state: s.state, version: (s.preparedReplacement ? s.previousVersion : s.version) ?? undefined,
    previousVersion: target(s, "rollback") ?? undefined,
    verifiedVersion: target(s, "upgrade") ?? undefined, officialVersion: s.officialVersion ?? undefined,
    checkedAt, progressBytes: s.progressBytes, raw: s.raw ?? undefined,
    preparedAction: s.preparedReplacement ? (target(s, "upgrade") ? "upgrade" : target(s, "rollback") ? "rollback" : undefined) : undefined,
    pendingAction: pending.get(driverId)?.operation };
}

function validatePrepared(value: unknown, driverId: Driver, operation: Switch, requestId: string, targetVersion: string): string {
  if (!object(value) || value.schema !== "gogoke.37.managed-cli-prepared.v1" || value.command !== operation ||
      value.driverId !== driverId || value.targetVersion !== targetVersion ||
      !revision(value.expectedRevision) || typeof value.finalFrame !== "string") {
    throw new Error("Native CLI preparation identity is invalid.");
  }
  let frame: unknown;
  try { frame = JSON.parse(value.finalFrame); } catch { throw new Error("Native CLI final frame is invalid."); }
  if (!object(frame) || Object.keys(frame).length !== 6 || frame.schema !== "gogoke.37.managed-cli.v1" ||
      frame.command !== operation || frame.driverId !== driverId || frame.requestId !== requestId ||
      frame.targetVersion !== targetVersion || frame.expectedRevision !== value.expectedRevision) {
    throw new Error("Native CLI final frame differs from preparation.");
  }
  return value.finalFrame;
}

function validateSwitchReceipt(value: unknown, driverId: Driver, intent: Pending): void {
  if (!object(value) || value.schema !== "gogoke.37.managed-cli.v1" || value.command !== intent.operation ||
      value.driverId !== driverId || !["APPLIED", "REPLAYED"].includes(String(value.status)) ||
      !nonnegative(value.migrated) || value.migrated === 0 ||
      !Array.isArray(value.receipts) || value.receipts.length !== value.migrated) {
    throw new Error("Native CLI switch receipt is unsettled.");
  }
  const seen = new Set<string>();
  for (const receipt of value.receipts) {
    if (!object(receipt) || receipt.schema !== "gogoke.37.managed-switch-witness.v1" ||
        receipt.status !== "APPLIED" || receipt.driverId !== driverId ||
        receipt.requestId !== intent.requestId || receipt.version !== intent.targetVersion ||
        typeof receipt.instanceId !== "string" || !receipt.instanceId || seen.has(receipt.instanceId) ||
        !revision(receipt.revision) || !revision(receipt.copyPreviousRevision) ||
        typeof receipt.requestFingerprint !== "string" || !receipt.requestFingerprint ||
        typeof receipt.homeIdentity !== "string" || !receipt.homeIdentity ||
        typeof receipt.programDigest !== "string" || !receipt.programDigest) {
      throw new Error("Native CLI original switch receipt is inconsistent.");
    }
    seen.add(receipt.instanceId);
  }
}

async function finalSwitch(driverId: Driver, intent: Pending): Promise<void> {
  const receipt = await design37ExactManagedCliFrame(intent.frame);
  validateSwitchReceipt(receipt, driverId, intent);
  pending.delete(driverId);
}

export function createManagedCliLifecycleSource(): InstancePageSource {
  const base = createDesign37ManagedInstanceSource(pageFromManaged);
  const asDriver = (vendor: VendorId): Driver => {
    if (!drivers.includes(vendor as Driver)) throw new Error("Unsupported managed CLI.");
    return vendor as Driver;
  };
  const switchCli = async (vendor: VendorId, operation: Switch): Promise<void> => {
    const driverId = asDriver(vendor);
    if (pending.has(driverId)) throw new Error("上次升级或退回的结果待确认，请先读回结果。");
    const fresh = await status(driverId);
    const targetVersion = target(fresh, operation);
    if (!fresh.revision || !targetVersion) throw new Error("当前没有已验证的目标版本，请刷新状态后重试。");
    const requestId = `ui-${crypto.randomUUID()}`;
    const prepared = await design37ManagedCliAction({ driverId, requestId, operation, targetVersion });
    const frame = validatePrepared(prepared, driverId, operation, requestId, targetVersion);
    const intent = { operation, frame, requestId, targetVersion };
    pending.set(driverId, intent);
    await finalSwitch(driverId, intent);
  };
  const simple = async (vendor: VendorId, operation: "check-updates" | "uninstall"): Promise<void> => {
    const driverId = asDriver(vendor);
    if (pending.has(driverId)) throw new Error("上次升级或退回的结果待确认，请先读回结果。");
    const reply = await design37ManagedCliAction({ driverId, requestId: `ui-${crypto.randomUUID()}`, operation });
    if (!object(reply) || reply.schema !== "gogoke.37.managed-cli.v1" || reply.command !== operation || reply.driverId !== driverId) {
      throw new Error("Native CLI action reply is invalid.");
    }
  };
  return {
    ...base,
    read: async (): Promise<InstancePage> => {
      const page = await base.read();
      const statuses = await Promise.all(drivers.map(status));
      for (const [index, driverId] of drivers.entries()) {
        const section = page.sections.find((item) => item.vendor === driverId);
        if (section) section.cli = display(statuses[index], driverId);
      }
      return page;
    },
    actions: { ...base.actions,
      upgradeCli: (vendor: VendorId) => switchCli(vendor, "upgrade"),
      rollbackCli: (vendor: VendorId) => switchCli(vendor, "rollback"),
      reconcileCli: async (vendor: VendorId) => {
        const driverId = asDriver(vendor);
        const intent = pending.get(driverId);
        if (!intent) throw new Error("没有可读回的上次操作。");
        await finalSwitch(driverId, intent);
      },
      checkCliUpdates: (vendor: VendorId) => simple(vendor, "check-updates"),
      uninstallCli: (vendor: VendorId) => simple(vendor, "uninstall"),
    },
  };
}
