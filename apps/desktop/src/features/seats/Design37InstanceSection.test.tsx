// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DESIGN37_INSTANCES_SCHEMA } from "./design37Instances";
import { Design37InstanceSection } from "./Design37InstanceSection";
import { createPreviewHost } from "./preview/host";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

function instance(state: string, login?: Record<string, unknown>, extra: Record<string, unknown> = {}) {
  return {
    instanceId: "codexTestM1",
    driverId: "codex",
    version: "1.2.3",
    revision: "7",
    state,
    ...(login === undefined ? {} : { login }),
    ...extra,
  };
}

function snapshot(...instances: ReturnType<typeof instance>[]) {
  return { schema: DESIGN37_INSTANCES_SCHEMA, instances };
}

function login(state: string, extra: Record<string, unknown> = {}) {
  return {
    requestId: "host-owned-request-id",
    expectedRevision: 7,
    state,
    output: "",
    browserState: "OPENED",
    startedAt: 100,
    settled: state !== "PENDING",
    ...extra,
  };
}

describe("Design37InstanceSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockResolvedValue(snapshot() as never);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("groups by vendor, shows plain state and hides internal identifiers", async () => {
    invokeMock.mockResolvedValue(snapshot(instance("LOGGED_IN")) as never);
    render(<Design37InstanceSection />);

    expect(await screen.findByText(/可以用 · 空闲/)).toBeTruthy();
    expect(screen.getByText("Codex CLI")).toBeTruthy();
    expect(screen.getByText("1 个可以用", { exact: false })).toBeTruthy();
    expect(screen.queryByText(/修订/)).toBeNull();
    expect(screen.queryByText("1.2.3")).toBeNull();
    // Antigravity is listed as unsupported with no controls.
    expect(screen.getByText(/暂不支持/)).toBeTruthy();
  });

  it("starts one login for the selected instance and shows the device code", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "gogoke_design37_instance_login") {
        return snapshot(instance("NOT_LOGGED_IN", login("PENDING", { deviceCode: "ABCD-EFGH" })));
      }
      const calls = invokeMock.mock.calls.filter(([name]) => name === "gogoke_design37_instance_login");
      return calls.length
        ? snapshot(instance("NOT_LOGGED_IN", login("PENDING", { deviceCode: "ABCD-EFGH" })))
        : snapshot(instance("NOT_LOGGED_IN"));
    });

    render(<Design37InstanceSection />);
    fireEvent.click(await screen.findByRole("button", { name: "登录" }));

    expect(await screen.findByText("ABCD-EFGH")).toBeTruthy();
    expect(invokeMock).toHaveBeenCalledWith("gogoke_design37_instance_login", { instanceId: "codexTestM1" });
    expect(screen.getByRole("button", { name: "取消登录" })).toBeTruthy();
    expect(screen.queryByText("host-owned-request-id")).toBeNull();
  });

  it("keeps the CLI's original failure text behind 查看原话", async () => {
    invokeMock.mockResolvedValue(
      snapshot(instance("ERROR", login("ERROR", { error: "codex exited 17: invalid_grant" }))) as never,
    );
    render(<Design37InstanceSection />);

    expect(await screen.findByText(/这次没登上/)).toBeTruthy();
    expect(screen.queryByText("codex exited 17: invalid_grant")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "查看原话" }));
    expect(screen.getByText("codex exited 17: invalid_grant")).toBeTruthy();
    expect(screen.getByRole("button", { name: "重新登录" })).toBeTruthy();
  });

  it("cancels the pending host session for only the selected instance", async () => {
    let cancelled = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "gogoke_design37_instance_cancel") cancelled = true;
      return cancelled
        ? snapshot(instance("NOT_LOGGED_IN", login("CANCELLED")))
        : snapshot(instance("NOT_LOGGED_IN", login("PENDING")));
    });

    render(<Design37InstanceSection />);
    fireEvent.click(await screen.findByRole("button", { name: "取消登录" }));

    expect(await screen.findByText(/上次登录已取消/)).toBeTruthy();
    expect(invokeMock).toHaveBeenCalledWith("gogoke_design37_instance_cancel", { instanceId: "codexTestM1" });
  });

  it("reads the existing host session after remount without starting another login", async () => {
    invokeMock.mockResolvedValue(
      snapshot(instance("NOT_LOGGED_IN", login("PENDING", { deviceCode: "WXYZ-1234" }))) as never,
    );

    const first = render(<Design37InstanceSection />);
    expect(await screen.findByText("WXYZ-1234")).toBeTruthy();
    first.unmount();

    render(<Design37InstanceSection />);
    expect(await screen.findByText("WXYZ-1234")).toBeTruthy();
    expect(invokeMock).not.toHaveBeenCalledWith("gogoke_design37_instance_login", expect.anything());
  });

  it("copies the device code", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    invokeMock.mockResolvedValue(
      snapshot(instance("NOT_LOGGED_IN", login("PENDING", { deviceCode: "COPY-ME" }))) as never,
    );

    render(<Design37InstanceSection />);
    fireEvent.click(await screen.findByRole("button", { name: "复制代码" }));

    await waitFor(() => expect(writeText).toHaveBeenCalledWith("COPY-ME"));
    expect(await screen.findByRole("button", { name: "已复制" })).toBeTruthy();
  });

  it("offers only cancel while a failed original request is still unsettled", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "gogoke_design37_instance_cancel") {
        return snapshot(instance("NOT_LOGGED_IN", login("CANCELLED")));
      }
      return snapshot(instance("ERROR", login("ERROR", { settled: false, error: "original transport failure" })));
    });
    render(<Design37InstanceSection />);

    fireEvent.click(await screen.findByRole("button", { name: "取消登录" }));
    expect(screen.queryByRole("button", { name: "重新登录" })).toBeNull();
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("gogoke_design37_instance_cancel", { instanceId: "codexTestM1" }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith("gogoke_design37_instance_login", expect.anything());
  });

  it("does not offer actions the host cannot perform", async () => {
    invokeMock.mockResolvedValue(snapshot(instance("LOGGED_IN")) as never);
    render(<Design37InstanceSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Codex 实例 的更多操作" }));

    expect(screen.getByRole("menuitem", { name: "详情" })).toBeTruthy();
    expect(screen.queryByRole("menuitem", { name: "删除实例" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "停用" })).toBeNull();
    expect(screen.queryByRole("button", { name: /新建 Claude Code 实例/ })).toBeNull();
    // The host's register enrolls a fixed test instance; it is not offered as creating one.
    expect(screen.queryByRole("button", { name: /新建 Codex 实例/ })).toBeNull();
  });

  it("names the instance readably and never shows its internal id", async () => {
    invokeMock.mockResolvedValue(snapshot(instance("LOGGED_IN")) as never);
    render(<Design37InstanceSection />);
    expect(await screen.findByText("Codex 实例")).toBeTruthy();
    expect(screen.queryByText(/codexTestM1/)).toBeNull();
  });

  it("does not offer login while the CLI the instance needs is missing", async () => {
    invokeMock.mockResolvedValue(snapshot(instance("NOT_INSTALLED")) as never);
    render(<Design37InstanceSection />);
    expect(await screen.findByText(/CLI 还没装，装好才能登录/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "登录" })).toBeNull();
  });

  it("keeps a logged-in instance usable and lists seat session problems apart", async () => {
    invokeMock.mockResolvedValue(
      snapshot(
        instance("LOGGED_IN", undefined, {
          runtimeIssues: [
            { seatId: "audit", sessionId: "s1", generation: "g1", reason: "turn aborted: 429", sourceEpoch: "e1", sourceCursor: "c1" },
          ],
        }),
      ) as never,
    );
    render(<Design37InstanceSection />);
    expect(await screen.findByText(/可以用 · 空闲/)).toBeTruthy();
    expect(screen.getByText(/1 个席位的会话出了问题/)).toBeTruthy();
    expect(screen.queryByText(/turn aborted: 429/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Codex 实例 的更多操作" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "详情" }));
    expect(screen.getByText("turn aborted: 429")).toBeTruthy();
    expect(screen.queryByText(/audit/)).toBeNull();
  });

  it.each([
    ["OPENED", /已在浏览器打开授权页/],
    ["FAILED", /浏览器没能自动打开/],
    ["NOT_REQUESTED", /还没打开授权页/],
  ])("says only what the host reports about the browser (%s)", async (browserState, text) => {
    invokeMock.mockResolvedValue(
      snapshot(instance("NOT_LOGGED_IN", login("PENDING", { browserState, authorizationUrl: "https://example.test/auth" }))) as never,
    );
    render(<Design37InstanceSection />);
    expect(await screen.findByText(text)).toBeTruthy();
    expect(Boolean(screen.queryByRole("link", { name: "打开授权页" }))).toBe(browserState !== "OPENED");
  });

  it("shows the CLI copy as unreported and hides unknown account and models", async () => {
    invokeMock.mockResolvedValue(snapshot(instance("LOGGED_IN", undefined, { newVersion: "9.9.9" })) as never);
    render(<Design37InstanceSection />);
    expect(await screen.findByText(/宿主还没报告这份 CLI 的情况/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "升级" })).toBeNull();
    fireEvent.click(screen.getAllByRole("button", { name: "详情" })[0]);
    expect(screen.queryByText("9.9.9", { exact: false })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Codex 实例 的更多操作" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "详情" }));
    expect(screen.queryByText("账号")).toBeNull();
    expect(screen.queryByText("能用的模型")).toBeNull();
    expect(screen.queryByText(/登录后才知道/)).toBeNull();
  });

  it("reports an unexpected host schema", async () => {
    invokeMock.mockResolvedValue({ schema: "other.v1", instances: [] } as never);
    render(<Design37InstanceSection />);
    expect((await screen.findByRole("alert")).textContent).toContain("Instance page returned an unexpected schema.");
  });

  it.each([true, false])(
    "reads the same host login result after the page closes before completion (success=%s)",
    async (success) => {
      const host = createPreviewHost();
      invokeMock.mockImplementation((command, args) => host.invoke(command, args as Record<string, unknown>));
      const first = render(<Design37InstanceSection />);
      fireEvent.click(await screen.findByRole("button", { name: "登录" }));
      await screen.findByText(/正在登录/);
      const pending = (await host.invoke("gogoke_design37_instances")) as {
        instances: { login: { requestId: string } }[];
      };
      first.unmount();
      host.settle(success);
      const settled = (await host.invoke("gogoke_design37_instances")) as {
        instances: { login: { requestId: string } }[];
      };
      expect(settled.instances[0].login.requestId).toBe(pending.instances[0].login.requestId);
      render(<Design37InstanceSection />);
      if (success) {
        await screen.findByText(/可以用 · 空闲/);
      } else {
        await screen.findByText(/这次没登上/);
        fireEvent.click(screen.getByRole("button", { name: "查看原话" }));
        expect(screen.getByText(/PREVIEW_CLI_FAILED: synthetic failure/)).toBeTruthy();
      }
      expect(invokeMock.mock.calls.filter(([command]) => command === "gogoke_design37_instance_login")).toHaveLength(1);
    },
  );
});
