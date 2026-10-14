import { beforeEach, describe, expect, it, vi } from "vitest";

let createManagedCliLifecycleSource: typeof import("./managedCliLifecycleSource").createManagedCliLifecycleSource;

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
const prepared = (command: string, driverId: string, requestId: string, targetVersion: string, expectedRevision = "7") => ({
  schema: "gogoke.37.managed-cli-prepared.v1", command, driverId, targetVersion, expectedRevision,
  finalFrame: JSON.stringify({ schema: "gogoke.37.managed-cli.v1", command, driverId,
    requestId, targetVersion, expectedRevision }),
});
const nativePending = (requestId: string, operation: string, targetVersion: string, phase = "BEGUN") => ({
  requestId, operation, targetVersion, phase,
});
const nativeFailed = (requestId: string, operation: string, targetVersion: string) => ({
  requestId, operation, targetVersion, phase: "FAILED",
});
const recoveryUuid = vi.fn(() => "recovery-uuid");

describe("managed CLI lifecycle source", () => {
  beforeEach(async () => {
    vi.resetModules();
    vi.resetAllMocks();
    ({ createManagedCliLifecycleSource } = await import("./managedCliLifecycleSource"));
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
    expect(mocks.user.mock.calls.length).toBe(before + 1);
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

describe("managed CLI same-request preparation recovery", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.resetAllMocks();
    recoveryUuid.mockReturnValue("recovery-uuid");
    vi.stubGlobal("crypto", { randomUUID: recoveryUuid });
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId));
    mocks.login.mockResolvedValue(undefined);
  });

  it("retains the original intent before a lost preparation reply and resumes with the same ID", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    const source = create();
    mocks.host.mockRejectedValueOnce(new Error("preparation reply lost"))
      .mockResolvedValueOnce(prepared("upgrade", "codex", "ui-recovery-uuid", "3.0.0"));
    mocks.exact.mockResolvedValue(reply("upgrade", "codex", "ui-recovery-uuid", "3.0.0"));

    await expect(source.actions.upgradeCli?.("codex")).rejects.toThrow("preparation reply lost");
    expect((await create().read()).sections[0].cli?.pendingAction).toBe("upgrade");
    expect(mocks.exact).not.toHaveBeenCalled();
    await create().actions.reconcileCli?.("codex");

    const original = { driverId: "codex", requestId: "ui-recovery-uuid",
      operation: "upgrade", targetVersion: "3.0.0" };
    expect(mocks.host).toHaveBeenCalledTimes(2);
    expect(mocks.host).toHaveBeenNthCalledWith(1, original);
    expect(mocks.host).toHaveBeenNthCalledWith(2, original);
    expect(recoveryUuid).toHaveBeenCalledTimes(1);
    expect(mocks.exact).toHaveBeenCalledWith(prepared("upgrade", "codex", "ui-recovery-uuid", "3.0.0").finalFrame);
    expect((await source.read()).sections[0].cli?.pendingAction).toBeUndefined();
  });

  it("restores only native open-journal identity after a module reload and waits for explicit reconciliation", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.host.mockRejectedValueOnce(new Error("preparation reply lost"))
      .mockResolvedValueOnce(prepared("rollback", "claude", "ui-recovery-uuid", "1.0.0"));
    mocks.exact.mockResolvedValue(reply("rollback", "claude", "ui-recovery-uuid", "1.0.0"));
    await expect(create().actions.rollbackCli?.("claude")).rejects.toThrow("preparation reply lost");

    vi.resetModules();
    const { createManagedCliLifecycleSource: createAfterReload } = await import("./managedCliLifecycleSource");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "claude" ? { pendingAction: nativePending("ui-recovery-uuid", "rollback", "1.0.0", "STAGED") } :
        { pendingAction: null }));
    const restored = createAfterReload();
    expect((await restored.read()).sections.find((s) => s.vendor === "claude")?.cli?.pendingAction).toBe("rollback");
    expect(mocks.host).toHaveBeenCalledTimes(1);
    expect(mocks.exact).not.toHaveBeenCalled();
    await expect(restored.actions.upgradeCli?.("claude")).rejects.toThrow("待确认");
    await restored.actions.reconcileCli?.("claude");
    expect(mocks.host).toHaveBeenNthCalledWith(2, { driverId: "claude", requestId: "ui-recovery-uuid",
      operation: "rollback", targetVersion: "1.0.0" });
    expect(recoveryUuid).toHaveBeenCalledTimes(1);
    expect(mocks.exact).toHaveBeenCalledTimes(1);
  });

  it("restores the original native request grammar including dot slash and colon", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    const requestId = "native.original/instance:1";
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "codex" ? { pendingAction: nativePending(requestId, "upgrade", "3.0.0", "PROBED") } : {}));
    mocks.host.mockResolvedValue(prepared("upgrade", "codex", requestId, "3.0.0"));
    mocks.exact.mockResolvedValue(reply("upgrade", "codex", requestId, "3.0.0"));
    const source = create();
    await source.read();
    await source.actions.reconcileCli?.("codex");
    expect(mocks.host).toHaveBeenCalledWith({ driverId: "codex", requestId, operation: "upgrade", targetVersion: "3.0.0" });
    expect(recoveryUuid).not.toHaveBeenCalled();
  });

  it("refuses a changed native identity and retains the original local request", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.host.mockRejectedValueOnce(new Error("preparation reply lost"))
      .mockResolvedValueOnce(prepared("upgrade", "opencode", "ui-recovery-uuid", "3.0.0"));
    mocks.exact.mockResolvedValue(reply("upgrade", "opencode", "ui-recovery-uuid", "3.0.0"));
    const source = create();
    await expect(source.actions.upgradeCli?.("opencode")).rejects.toThrow("preparation reply lost");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "opencode" ? { pendingAction: nativePending("other-id", "upgrade", "3.0.0") } : {}));
    await expect(source.read()).rejects.toThrow("differs from the original");
    await expect(source.actions.reconcileCli?.("opencode")).rejects.toThrow("differs from the original");
    expect(mocks.host).toHaveBeenCalledTimes(1);
    expect(mocks.exact).not.toHaveBeenCalled();

    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "opencode" ? { pendingAction: nativePending("ui-recovery-uuid", "upgrade", "3.0.0") } : {}));
    await source.actions.reconcileCli?.("opencode");
    expect(mocks.host).toHaveBeenNthCalledWith(2, { driverId: "opencode", requestId: "ui-recovery-uuid",
      operation: "upgrade", targetVersion: "3.0.0" });
    expect(recoveryUuid).toHaveBeenCalledTimes(1);
  });

  it.each([
    ["operation", "rollback"],
    ["targetVersion", "2.0.0"],
  ])("refuses a native pending action with a changed %s", async (field, replacement) => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.host.mockRejectedValueOnce(new Error("preparation reply lost"));
    const source = create();
    await expect(source.actions.upgradeCli?.("codex")).rejects.toThrow("preparation reply lost");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "codex" ? { pendingAction: {
        ...nativePending("ui-recovery-uuid", "upgrade", "3.0.0"), [field]: replacement,
      } } : {}));
    await expect(source.read()).rejects.toThrow("differs from the original");
    await expect(source.actions.reconcileCli?.("codex")).rejects.toThrow("differs from the original");
    expect(mocks.host).toHaveBeenCalledTimes(1);
    expect(mocks.exact).not.toHaveBeenCalled();
  });

  it.each([
    nativePending("bad id", "upgrade", "3.0.0"),
    nativePending("ui-recovery-uuid", "uninstall", "3.0.0"),
    nativePending("ui-recovery-uuid", "upgrade", "3.0.0-preview"),
    { ...nativePending("ui-recovery-uuid", "upgrade", "3.0.0"), other: "untrusted" },
  ])("rejects a malformed native open-action identity without installing a pending intent", async (invalid) => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "codex" ? { pendingAction: invalid } : {}));
    await expect(create().read()).rejects.toThrow("Invalid native CLI pending action");
    expect(mocks.host).not.toHaveBeenCalled();
    expect(mocks.exact).not.toHaveBeenCalled();
  });

  it("blocks other lifecycle writes when native reports an open version action after reload", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "codex" ? { pendingAction: nativePending("original-id", "upgrade", "3.0.0") } : {}));
    const source = create();
    await expect(source.actions.checkCliUpdates?.("codex")).rejects.toThrow("待确认");
    await expect(source.actions.uninstallCli?.("codex")).rejects.toThrow("待确认");
    expect(mocks.host).not.toHaveBeenCalled();
    expect(recoveryUuid).not.toHaveBeenCalled();
  });

  it("clears only a matching native FAILED action on read and shows its original failure text", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.host.mockRejectedValueOnce(new Error("download reply lost"));
    const source = create();
    await expect(source.actions.upgradeCli?.("codex")).rejects.toThrow("download reply lost");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "codex" ? { state: "BLOCKED", raw: "original native download failure",
        pendingAction: null, failedAction: nativeFailed("ui-recovery-uuid", "upgrade", "3.0.0") } : {}));
    const cli = (await source.read()).sections.find((s) => s.vendor === "codex")?.cli;
    expect(cli?.pendingAction).toBeUndefined();
    expect(cli?.raw).toBe("original native download failure");
    expect(mocks.host).toHaveBeenCalledTimes(1);
    expect(mocks.exact).not.toHaveBeenCalled();
    expect(recoveryUuid).toHaveBeenCalledTimes(1);
  });

  it("explicit reconcile observes exact FAILED even when it already retained a final frame", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.host.mockResolvedValue(prepared("rollback", "claude", "ui-recovery-uuid", "1.0.0"));
    mocks.exact.mockRejectedValueOnce(new Error("final reply lost"));
    const source = create();
    await expect(source.actions.rollbackCli?.("claude")).rejects.toThrow("final reply lost");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "claude" ? { state: "BLOCKED", raw: "original native final failure",
        pendingAction: null, failedAction: nativeFailed("ui-recovery-uuid", "rollback", "1.0.0") } : {}));
    await source.actions.reconcileCli?.("claude");
    expect(mocks.exact).toHaveBeenCalledTimes(1);
    expect(mocks.host).toHaveBeenCalledTimes(1);
    expect((await source.read()).sections.find((s) => s.vendor === "claude")?.cli).toMatchObject({
      raw: "original native final failure", pendingAction: undefined,
    });
  });

  it("keeps a known final frame when native reports an earlier preparation phase", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    const response = prepared("upgrade", "grok", "ui-recovery-uuid", "3.0.0");
    mocks.host.mockResolvedValue(response);
    mocks.exact.mockRejectedValueOnce(new Error("final reply lost"))
      .mockResolvedValueOnce(reply("upgrade", "grok", "ui-recovery-uuid", "3.0.0"));
    const source = create();
    await expect(source.actions.upgradeCli?.("grok")).rejects.toThrow("final reply lost");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "grok" ? { pendingAction: nativePending("ui-recovery-uuid", "upgrade", "3.0.0", "STAGED") } : {}));
    await expect(source.actions.reconcileCli?.("grok")).rejects.toThrow("phase differs");
    expect(mocks.exact).toHaveBeenCalledTimes(1);
    expect(mocks.host).toHaveBeenCalledTimes(1);
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "grok" ? { pendingAction: nativePending("ui-recovery-uuid", "upgrade", "3.0.0", "PROBED") } : {}));
    await source.actions.reconcileCli?.("grok");
    expect(mocks.exact).toHaveBeenNthCalledWith(2, response.finalFrame);
  });

  it.each([
    ["missing", undefined],
    ["request ID", nativeFailed("other-id", "upgrade", "3.0.0")],
    ["operation", nativeFailed("ui-recovery-uuid", "rollback", "3.0.0")],
    ["target version", nativeFailed("ui-recovery-uuid", "upgrade", "2.0.0")],
  ])("keeps the local pending intent when native FAILED identity is %s", async (_label, failedAction) => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.host.mockRejectedValueOnce(new Error("preparation reply lost"));
    const source = create();
    await expect(source.actions.upgradeCli?.("codex")).rejects.toThrow("preparation reply lost");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "codex" ? { state: "BLOCKED", pendingAction: null, failedAction } : {}));
    expect((await source.read()).sections.find((s) => s.vendor === "codex")?.cli?.pendingAction).toBe("upgrade");
    await expect(source.actions.upgradeCli?.("codex")).rejects.toThrow("待确认");
    expect(mocks.host).toHaveBeenCalledTimes(1);
    expect(recoveryUuid).toHaveBeenCalledTimes(1);
  });

  it("never treats an open PROBING action as terminal FAILED", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "grok" ? { pendingAction: nativePending("ui-recovery-uuid", "upgrade", "3.0.0", "PROBING"),
        failedAction: nativeFailed("ui-recovery-uuid", "upgrade", "3.0.0") } : {}));
    const source = create();
    expect((await source.read()).sections.find((s) => s.vendor === "grok")?.cli?.pendingAction).toBe("upgrade");
    await expect(source.actions.reconcileCli?.("grok")).rejects.toThrow("仍未确定");
    expect(mocks.host).not.toHaveBeenCalled();
    expect(mocks.exact).not.toHaveBeenCalled();
  });

  it("does not revive a FAILED action after a fresh module load", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "opencode" ? { state: "BLOCKED", raw: "original native failure",
        pendingAction: null, failedAction: nativeFailed("old-id", "upgrade", "3.0.0") } : {}));
    const source = create();
    expect((await source.read()).sections.find((s) => s.vendor === "opencode")?.cli).toMatchObject({
      raw: "original native failure", pendingAction: undefined,
    });
    await expect(source.actions.reconcileCli?.("opencode")).rejects.toThrow("没有可读回");
    expect(mocks.host).not.toHaveBeenCalled();
  });

  it("rejects malformed native pending fields and does not resume an ambiguous probe", async () => {
    const { createManagedCliLifecycleSource: create } = await import("./managedCliLifecycleSource");
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "grok" ? { pendingAction: nativePending("ui-recovery-uuid", "rollback", "1.0.0", "UNKNOWN") } : {}));
    await expect(create().read()).rejects.toThrow("Invalid native CLI pending action");
    expect(mocks.host).not.toHaveBeenCalled();
    mocks.user.mockImplementation(async (frame: { driverId: string }) => status(frame.driverId,
      frame.driverId === "grok" ? { pendingAction: nativePending("ui-recovery-uuid", "rollback", "1.0.0", "PROBING") } : {}));
    const source = create();
    expect((await source.read()).sections.find((s) => s.vendor === "grok")?.cli?.pendingAction).toBe("rollback");
    await expect(source.actions.reconcileCli?.("grok")).rejects.toThrow("仍未确定");
    expect(mocks.host).not.toHaveBeenCalled();
    expect(mocks.exact).not.toHaveBeenCalled();
  });
});
