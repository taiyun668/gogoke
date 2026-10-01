// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DESIGN37_INSTANCES_SCHEMA } from "./design37Instances";
import { Design37InstanceSection } from "./Design37InstanceSection";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

function instance(state: string, login?: Record<string, unknown>) {
  return {
    instanceId: "codexTestM1",
    driverId: "codex",
    version: "1.2.3",
    revision: "7",
    state,
    ...(login === undefined ? {} : { login }),
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

  it("starts one login using only the selected instance ID and shows the host PENDING result", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "gogoke_design37_instances") return snapshot(instance("NOT_LOGGED_IN"));
      if (command === "gogoke_design37_instance_login") {
        return snapshot(instance("NOT_LOGGED_IN", login("PENDING", { deviceCode: "ABCD-EFGH" })));
      }
      return undefined;
    });

    render(<Design37InstanceSection />);
    fireEvent.click(await screen.findByRole("button", { name: "一键登录" }));

    await screen.findByText(/登录：正在登录/);
    expect(invokeMock).toHaveBeenCalledWith("gogoke_design37_instance_login", {
      instanceId: "codexTestM1",
    });
    expect(screen.getByText("ABCD-EFGH")).toBeTruthy();
    expect(screen.queryByText("host-owned-request-id")).toBeNull();
  });

  it("shows the original host login failure reason", async () => {
    invokeMock.mockResolvedValue(snapshot(instance("ERROR", login("ERROR", {
      error: "codex exited 17: invalid_grant",
    }))) as never);

    render(<Design37InstanceSection />);

    expect((await screen.findByRole("alert")).textContent).toContain("codex exited 17: invalid_grant");
    expect(screen.getByText(/状态：出错/)).toBeTruthy();
  });

  it("shows the host-detected successful login state without another user action", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "gogoke_design37_instances") return snapshot(instance("NOT_LOGGED_IN"));
      if (command === "gogoke_design37_instance_login") {
        return snapshot(instance("LOGGED_IN", login("LOGGED_IN")));
      }
      return undefined;
    });

    render(<Design37InstanceSection />);
    fireEvent.click(await screen.findByRole("button", { name: "一键登录" }));

    await screen.findByText(/状态：已登录/);
    expect(screen.getByText("宿主已检测到登录成功。")).toBeTruthy();
  });

  it("cancels the pending host session for only the selected instance", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "gogoke_design37_instances") {
        return snapshot(instance("NOT_LOGGED_IN", login("PENDING")));
      }
      if (command === "gogoke_design37_instance_cancel") {
        return snapshot(instance("NOT_LOGGED_IN", login("CANCELLED")));
      }
      return undefined;
    });

    render(<Design37InstanceSection />);
    fireEvent.click(await screen.findByRole("button", { name: "取消登录" }));

    expect(await screen.findByText("此实例的登录请求已取消。")).toBeTruthy();
    expect(invokeMock).toHaveBeenCalledWith("gogoke_design37_instance_cancel", {
      instanceId: "codexTestM1",
    });
  });

  it("reads the existing host session after remount without starting another login", async () => {
    invokeMock.mockResolvedValue(snapshot(instance("NOT_LOGGED_IN", login("PENDING", {
      deviceCode: "WXYZ-1234",
    }))) as never);

    const first = render(<Design37InstanceSection />);
    expect(await screen.findByText("WXYZ-1234")).toBeTruthy();
    first.unmount();

    render(<Design37InstanceSection />);
    expect(await screen.findByText("WXYZ-1234")).toBeTruthy();
    expect(invokeMock).toHaveBeenCalledWith("gogoke_design37_instances");
    expect(invokeMock).not.toHaveBeenCalledWith("gogoke_design37_instance_login", expect.anything());
  });

  it("copies the host device code and reports success", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    invokeMock.mockResolvedValue(snapshot(instance("NOT_LOGGED_IN", login("PENDING", {
      deviceCode: "COPY-ME",
    }))) as never);

    render(<Design37InstanceSection />);
    fireEvent.click(await screen.findByRole("button", { name: "复制设备码" }));

    await waitFor(() => expect(writeText).toHaveBeenCalledWith("COPY-ME"));
    expect(await screen.findByText("已复制 codexTestM1 的设备码。")).toBeTruthy();
  });

  it("can cancel an unsettled failed original request without allowing another login", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "gogoke_design37_instance_cancel") {
        return snapshot(instance("NOT_LOGGED_IN", login("CANCELLED")));
      }
      return snapshot(instance("ERROR", login("ERROR", { settled: false, error: "original transport failure" })));
    });
    render(<Design37InstanceSection />);
    expect((await screen.findByRole("button", { name: "一键登录" })).hasAttribute("disabled")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "取消登录" }));
    await screen.findByText("此实例的登录请求已取消。");
    expect(invokeMock).toHaveBeenCalledWith("gogoke_design37_instance_cancel", { instanceId: "codexTestM1" });
    expect(invokeMock).not.toHaveBeenCalledWith("gogoke_design37_instance_login", expect.anything());
  });

  it("reports an unexpected host schema as an error", async () => {
    invokeMock.mockResolvedValue({ schema: "other.v1", instances: [] } as never);

    render(<Design37InstanceSection />);

    expect((await screen.findByRole("alert")).textContent).toContain("Instance page returned an unexpected schema.");
  });
});
