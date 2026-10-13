import { beforeEach, describe, expect, it, vi } from "vitest";
import { createManagedCliLifecycleSource } from "./managedCliLifecycleSource";

const mocks = vi.hoisted(() => ({
  user: vi.fn(), host: vi.fn(), exact: vi.fn(), login: vi.fn(),
}));
vi.mock("@/services/tauri", () => ({
  design37UserFrame: mocks.user, design37ManagedCliAction: mocks.host,
  design37ExactManagedCliFrame: mocks.exact,
}));
vi.mock("@/services/design37ManagedInstances", () => ({
  createDesign37ManagedInstanceSource: (project: (value: unknown) => unknown) => ({
    read: async () => project({ sections: [{ vendor: "codex", instances: [{ id: "original-instance", name: "原实例", state: "READY" }] }] }),
    actions: { login: mocks.login },
  }),
}));

const status = (driverId: string, extra: Record<string, unknown> = {}) => ({
  schema: "gogoke.37.managed-cli.v1", command: "status", driverId,
  state: "READY", version: "2.0.0", revision: "7", previousVersion: "1.0.0",
  preparedReplacement: false,
  verifiedOtherVersions: ["1.0.0", "3.0.0"], officialVersion: "9.0.0", checkedAt: null, raw: null,
  ...extra,
});
const reply = (command: string, driverId: string, requestId: string, targetVersion: string) => ({
  schema: "gogoke.37.managed-cli.v1", command, driverId, status: "APPLIED", migrated: 1,
  receipts: [{ schema: "gogoke.37.managed-switch-witness.v1", status: "APPLIED", driverId,
    requestId, version: targetVersion, instanceId: "original-instance", revision: "8",
    copyPreviousRevision: "7", requestFingerprint: "fingerprint", homeIdentity: "home", programDigest: "digest" }],
});

describe("managed CLI lifecycle source", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("crypto", { randomUUID: () => "test-uuid" });
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId));
    mocks.login.mockResolvedValue(undefined);
  });

  it("keeps existing instances/actions and never offers an unverified or older version", async () => {
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId, {
      verifiedOtherVersions: ["1.0.0"], officialVersion: "9.0.0-preview",
    }));
    const source = createManagedCliLifecycleSource();
    const page = await source.read();
    expect(page.sections[0].instances[0].name).toBe("原实例");
    expect(page.sections[0].cli).toMatchObject({ version: "2.0.0", officialVersion: "9.0.0-preview" });
    expect(page.sections[0].cli?.verifiedVersion).toBeUndefined();
    await source.actions.login?.("original-instance");
    expect(mocks.login).toHaveBeenCalledWith("original-instance");
    await expect(source.actions.upgradeCli?.("codex")).rejects.toThrow("已验证");
    expect(mocks.host).not.toHaveBeenCalled();
  });

  it("keeps original final-frame whitespace and reads back only those bytes after a lost reply", async () => {
    const source = createManagedCliLifecycleSource();
    const frame = ` { "schema" : "gogoke.37.managed-cli.v1", "command":"upgrade", "driverId":"codex", "requestId":"ui-test-uuid", "targetVersion":"3.0.0", "expectedRevision":"7" } `;
    mocks.host.mockResolvedValue({ schema: "gogoke.37.managed-cli-prepared.v1", command: "upgrade",
      driverId: "codex", targetVersion: "3.0.0", expectedRevision: "7", finalFrame: frame });
    mocks.exact.mockRejectedValueOnce(new Error("reply lost"))
      .mockResolvedValueOnce(reply("upgrade", "codex", "ui-test-uuid", "3.0.0"));
    await expect(source.actions.upgradeCli?.("codex")).rejects.toThrow("reply lost");
    expect(mocks.exact).toHaveBeenCalledWith(frame);
    const page = await createManagedCliLifecycleSource().read();
    expect(page.sections[0].cli?.pendingAction).toBe("upgrade");
    const before = mocks.user.mock.calls.length;
    await createManagedCliLifecycleSource().actions.reconcileCli?.("codex");
    expect(mocks.user.mock.calls.length).toBe(before);
    expect(mocks.host).toHaveBeenCalledTimes(1);
    expect(mocks.exact).toHaveBeenNthCalledWith(2, frame);
    expect((await source.read()).sections[0].cli?.pendingAction).toBeUndefined();
  });

  it("does not clear a pending original frame on an inconsistent receipt", async () => {
    const source = createManagedCliLifecycleSource();
    const frame = JSON.stringify({ schema: "gogoke.37.managed-cli.v1", command: "rollback",
      driverId: "claude", requestId: "ui-test-uuid", targetVersion: "1.0.0", expectedRevision: "7" });
    mocks.host.mockResolvedValue({ schema: "gogoke.37.managed-cli-prepared.v1", command: "rollback",
      driverId: "claude", targetVersion: "1.0.0", expectedRevision: "7", finalFrame: frame });
    mocks.exact.mockResolvedValueOnce({ ...reply("rollback", "claude", "ui-test-uuid", "1.0.0"), migrated: 2 });
    await expect(source.actions.rollbackCli?.("claude")).rejects.toThrow("unsettled");
    await expect(source.actions.rollbackCli?.("claude")).rejects.toThrow("待确认");
    expect(mocks.host).toHaveBeenCalledTimes(1);
  });

  it("rejects a prepared frame whose six original fields differ", async () => {
    const source = createManagedCliLifecycleSource();
    mocks.host.mockResolvedValue({ schema: "gogoke.37.managed-cli-prepared.v1", command: "upgrade",
      driverId: "grok", targetVersion: "3.0.0", expectedRevision: "7", finalFrame: JSON.stringify({
        schema: "gogoke.37.managed-cli.v1", command: "upgrade", driverId: "grok",
        requestId: "different", targetVersion: "3.0.0", expectedRevision: "7",
      }) });
    await expect(source.actions.upgradeCli?.("grok")).rejects.toThrow("differs");
    expect(mocks.exact).not.toHaveBeenCalled();
  });

  it("uses the effective previous version for a native prepared replacement", async () => {
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId, frame.driverId === "opencode" ? {
      state: "PROBED", version: "3.0.0", previousVersion: "2.0.0", preparedReplacement: true,
      verifiedOtherVersions: ["1.0.0"],
    } : {}));
    const source = createManagedCliLifecycleSource();
    const page = await source.read();
    expect(page.sections.find((s) => s.vendor === "opencode")?.cli).toMatchObject({
      state: "PROBED", version: "2.0.0", verifiedVersion: "3.0.0", preparedAction: "upgrade",
    });
  });

  it("passes check and uninstall to Host and preserves backend refusal", async () => {
    const source = createManagedCliLifecycleSource();
    mocks.host.mockResolvedValueOnce({ schema: "gogoke.37.managed-cli.v1", command: "check-updates", driverId: "grok" })
      .mockRejectedValueOnce(new Error("native instance source is still active"));
    await source.actions.checkCliUpdates?.("grok");
    await expect(source.actions.uninstallCli?.("grok")).rejects.toThrow("native instance source is still active");
    expect(mocks.host).toHaveBeenNthCalledWith(1, { driverId: "grok", requestId: "ui-test-uuid", operation: "check-updates" });
    expect(mocks.host).toHaveBeenNthCalledWith(2, { driverId: "grok", requestId: "ui-test-uuid", operation: "uninstall" });
  });

  it("accepts both absent and retained-revision NOT_INSTALLED status without offering a switch", async () => {
    mocks.user.mockImplementation(async (frame: { driverId: string }) => {
      if (frame.driverId === "codex") return status("codex", { state: "NOT_INSTALLED", revision: "4",
        version: null, preparedReplacement: null, previousVersion: null,
        verifiedOtherVersions: [], officialVersion: "4.1.0-preview", checkedAt: "1000", progressBytes: 0 });
      if (frame.driverId === "claude") return status("claude", { state: "NOT_INSTALLED", revision: "5",
        version: null, preparedReplacement: null, previousVersion: null,
        verifiedOtherVersions: [], officialVersion: null, checkedAt: null, progressBytes: 0 });
      return status(frame.driverId, { state: "NOT_INSTALLED", revision: null, version: null,
        preparedReplacement: null, previousVersion: null, verifiedOtherVersions: [] });
    });
    const source = createManagedCliLifecycleSource();
    const page = await source.read();
    expect(page.sections.find((s) => s.vendor === "codex")?.cli).toMatchObject({
      state: "NOT_INSTALLED", officialVersion: "4.1.0-preview", progressBytes: 0 });
    expect(page.sections.find((s) => s.vendor === "claude")?.cli?.state).toBe("NOT_INSTALLED");
    expect(page.sections.find((s) => s.vendor === "grok")?.cli?.state).toBe("NOT_INSTALLED");
    await expect(source.actions.upgradeCli?.("codex")).rejects.toThrow("已验证");
    expect(mocks.host).not.toHaveBeenCalled();
  });

  it("does not accept an empty successful-looking switch receipt or discard its original frame", async () => {
    const source = createManagedCliLifecycleSource();
    const frame = JSON.stringify({ schema: "gogoke.37.managed-cli.v1", command: "upgrade",
      driverId: "opencode", requestId: "ui-test-uuid", targetVersion: "3.0.0", expectedRevision: "8" });
    mocks.host.mockResolvedValue({ schema: "gogoke.37.managed-cli-prepared.v1", command: "upgrade",
      driverId: "opencode", targetVersion: "3.0.0", expectedRevision: "8", finalFrame: frame });
    mocks.exact.mockResolvedValue({ schema: "gogoke.37.managed-cli.v1", command: "upgrade",
      driverId: "opencode", status: "APPLIED", migrated: 0, receipts: [] });
    await expect(source.actions.upgradeCli?.("opencode")).rejects.toThrow("unsettled");
    expect((await createManagedCliLifecycleSource().read()).sections.find((s) => s.vendor === "opencode")?.cli?.pendingAction).toBe("upgrade");
    await expect(source.actions.reconcileCli?.("opencode")).rejects.toThrow("unsettled");
    expect(mocks.exact).toHaveBeenNthCalledWith(2, frame);
    expect(mocks.host).toHaveBeenCalledTimes(1);
  });
});
