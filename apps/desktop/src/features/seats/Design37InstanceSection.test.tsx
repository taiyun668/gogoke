// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

function profile(extra: Record<string, unknown> = {}) {
  return {
    instanceId: "codexTestM1",
    driverId: "codex",
    name: "Plus 1 号",
    enabled: true,
    profileRevision: "r1",
    cap: 4,
    ...extra,
  };
}

type Host = {
  instances: ReturnType<typeof instance>[];
  profiles: ReturnType<typeof profile>[];
  cli: Array<Record<string, unknown>>;
  operations: Array<Record<string, unknown>>;
  failRead?: boolean;
};

function host(partial: Partial<Host> = {}): Host {
  return { instances: [instance("LOGGED_IN")], profiles: [profile()], cli: [], operations: [], ...partial };
}

function serve(state: Host) {
  invokeMock.mockImplementation(async (command: string, args?: unknown) => {
    if (command === "gogoke_design37_instances") {
      if (state.failRead) throw new Error("宿主没有响应");
      return { schema: DESIGN37_INSTANCES_SCHEMA, instances: state.instances } as never;
    }
    if (command === "gogoke_design37_user_operation") {
      const frame = JSON.parse((args as { frame: string }).frame) as Record<string, unknown>;
      state.operations.push(frame);
      if (frame.command === "instance-management-read") {
        return JSON.stringify({ schema: "gogoke.37.instance-management.v1", profiles: state.profiles, cli: state.cli }) as never;
      }
      return JSON.stringify({ status: "APPLIED" }) as never;
    }
    if (command === "gogoke_design37_install_cli") {
      state.operations.push({ command, ...(args as object) });
      return undefined as never;
    }
    if (command === "gogoke_design37_instance_login" || command === "gogoke_design37_instance_cancel") {
      state.operations.push({ command, ...(args as object) });
      return { schema: DESIGN37_INSTANCES_SCHEMA, instances: state.instances } as never;
    }
    throw new Error(`unexpected ${command}`);
  });
}

describe("Design37InstanceSection", () => {
  beforeEach(() => vi.clearAllMocks());
  afterEach(cleanup);

  it("shows the host's name and plain state, and hides internal identifiers", async () => {
    serve(host());
    render(<Design37InstanceSection />);
    expect(await screen.findByText("Plus 1 号")).toBeTruthy();
    expect(screen.getByText("可以用")).toBeTruthy();
    expect(screen.queryByText(/codexTestM1/)).toBeNull();
    expect(screen.getByText(/暂不支持/)).toBeTruthy();
  });

  it("asks for a name and enabled state when the host has no profile yet, without filling them in", async () => {
    const state = host({ profiles: [profile({ name: undefined, enabled: undefined, profileRevision: undefined, cap: undefined })] });
    serve(state);
    render(<Design37InstanceSection />);
    expect(await screen.findByText(/名字、启用或并发上限还没设好/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "启用" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "设置" }));
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByRole("alert").textContent).toContain("给它起个名字");
    fireEvent.change(screen.getByLabelText("名字"), { target: { value: "Plus 1 号" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(state.operations).toContainEqual(
        expect.objectContaining({ command: "instance-profile", instanceId: "codexTestM1", name: "Plus 1 号", enabled: true }),
      ),
    );
  });

  it("states only that the host asked the browser to open the page", async () => {
    serve(
      host({
        instances: [instance("NOT_LOGGED_IN", login("PENDING", { deviceCode: "ABCD-EFGH", authorizationUrl: "https://example.test/a" }))],
      }),
    );
    render(<Design37InstanceSection />);
    expect(await screen.findByText(/已请浏览器打开授权页/)).toBeTruthy();
    expect(screen.queryByText(/已在浏览器打开/)).toBeNull();
    expect(screen.getByRole("link", { name: "没看到的话，打开授权页" })).toBeTruthy();
    expect(screen.getByText("ABCD-EFGH")).toBeTruthy();
  });

  it("does not promise a check the host cannot run", async () => {
    serve(host({ instances: [instance("NOT_LOGGED_IN", login("UNKNOWN"))] }));
    render(<Design37InstanceSection />);
    expect(await screen.findByText("没能确认登没登上，先不当成登上了")).toBeTruthy();
    expect(screen.queryByText(/检测一下/)).toBeNull();
    expect(screen.queryByRole("button", { name: "检测" })).toBeNull();
  });

  it("continues a staged install and reports an unsettled self-test plainly", async () => {
    const state = host({ cli: [{ driverId: "codex", state: "STAGED" }, { driverId: "claude", state: "PROBE_UNKNOWN", raw: "probe stopped" }] });
    serve(state);
    render(<Design37InstanceSection />);
    const codex = within(await screen.findByLabelText("Codex"));
    expect(codex.getByText("下载好了，还没装完")).toBeTruthy();
    fireEvent.click(codex.getByRole("button", { name: "继续安装" }));
    await waitFor(() => expect(state.operations).toContainEqual(expect.objectContaining({ command: "gogoke_design37_install_cli" })));
    const claude = within(screen.getByLabelText("Claude Code"));
    expect(claude.getByText(/自检停下了，没能确认通没通过/)).toBeTruthy();
    expect(claude.queryByText(/网络/)).toBeNull();
  });

  it("keeps the last rows after a failed read but offers nothing on them", async () => {
    const state = host({ instances: [instance("NOT_LOGGED_IN")] });
    serve(state);
    render(<Design37InstanceSection />);
    expect(await screen.findByRole("button", { name: "登录" })).toBeTruthy();
    state.failRead = true;
    expect(await screen.findByText(/读不到最新状态：宿主没有响应/, {}, { timeout: 2500 })).toBeTruthy();
    expect((screen.getByRole("button", { name: "登录" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.queryByText(/都正常/)).toBeNull();
  });

  it("offers no enable or disable while the enabled state is unknown", async () => {
    serve(host({ profiles: [profile({ enabled: undefined })] }));
    render(<Design37InstanceSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Plus 1 号 的更多操作" }));
    expect(screen.queryByRole("menuitem", { name: "停用" })).toBeNull();
    expect(screen.queryByRole("button", { name: "启用" })).toBeNull();
  });

  it("reports an unexpected host schema", async () => {
    invokeMock.mockResolvedValue({ schema: "other.v1", instances: [] } as never);
    render(<Design37InstanceSection />);
    expect((await screen.findByRole("alert")).textContent).toContain("Instance page returned an unexpected schema.");
  });
});
