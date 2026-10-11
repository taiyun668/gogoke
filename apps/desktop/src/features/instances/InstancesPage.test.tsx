// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { InstancesPage, type InstancePageSource } from "./InstancesPage";
import type { InstancePage } from "./instancePageModel";

function page(): InstancePage {
  return {
    sections: [
      {
        vendor: "codex",
        cli: { state: "READY", version: "1.0.0" },
        instances: [{ id: "i1", name: "Plus 1 号", state: "READY", enabled: true, seats: [] }],
      },
    ],
  };
}

describe("InstancesPage", () => {
  afterEach(cleanup);

  it("keeps the confirmation open and reports no success when the host refuses", async () => {
    const remove = vi.fn(async () => {
      throw new Error("宿主拒绝：实例还在用");
    });
    const source: InstancePageSource = { read: async () => page(), actions: { remove } };
    render(<InstancesPage source={source} />);
    fireEvent.click(await screen.findByRole("button", { name: "Plus 1 号 的更多操作" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "删除实例" }));
    fireEvent.click(screen.getByRole("button", { name: "确认删除" }));
    await waitFor(() => expect(remove).toHaveBeenCalledWith("i1"));
    expect(await screen.findByText("宿主拒绝：实例还在用")).toBeTruthy();
    expect(screen.queryByText(/已删除/)).toBeNull();
    expect(screen.getByRole("button", { name: "确认删除" })).toBeTruthy();
  });

  it("drops the old source's rows and never sends their actions to a new source", async () => {
    const remove = vi.fn(async () => {});
    const { rerender } = render(<InstancesPage source={{ read: async () => page(), actions: { remove } }} />);
    expect(await screen.findByText("Plus 1 号")).toBeTruthy();
    rerender(<InstancesPage source={{ read: () => new Promise<InstancePage>(() => {}), actions: { remove } }} />);
    expect(await screen.findByText("正在读取实例…")).toBeTruthy();
    expect(screen.queryByText("Plus 1 号")).toBeNull();
  });

  it("does not let an unfinished operation on the old source lock the new one", async () => {
    const stuck = vi.fn(() => new Promise<void>(() => {}));
    const { rerender } = render(<InstancesPage source={{ read: async () => page(), actions: { remove: stuck } }} />);
    fireEvent.click(await screen.findByRole("button", { name: "Plus 1 号 的更多操作" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "删除实例" }));
    fireEvent.click(screen.getByRole("button", { name: "确认删除" }));
    await waitFor(() => expect(stuck).toHaveBeenCalled());
    const remove = vi.fn(async () => {});
    rerender(<InstancesPage source={{ read: async () => page(), actions: { remove } }} />);
    fireEvent.click(await screen.findByRole("button", { name: "Plus 1 号 的更多操作" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "删除实例" }));
    const confirm = screen.getByRole("button", { name: "确认删除" }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);
    await waitFor(() => expect(remove).toHaveBeenCalledWith("i1"));
  });

  it("applies only the latest read, so a poll that started before a write cannot overwrite it", async () => {
    const named = (name: string) => {
      const data = page();
      data.sections[0].instances[0].name = name;
      return data;
    };
    let release: (value: InstancePage) => void = () => {};
    let calls = 0;
    const read = () => {
      calls += 1;
      if (calls === 1) return Promise.resolve(page());
      if (calls === 2) return new Promise<InstancePage>((resolve) => (release = resolve));
      return Promise.resolve(named("新名字"));
    };
    const check = vi.fn(async () => {});
    render(<InstancesPage source={{ read, actions: { check } }} />);
    fireEvent.click(await screen.findByRole("button", { name: "Plus 1 号 的更多操作" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "详情" }));
    await waitFor(() => expect(calls).toBe(2), { timeout: 2500 });
    fireEvent.click(screen.getByRole("button", { name: "检测" }));
    expect(await screen.findByText("新名字")).toBeTruthy();
    release(named("旧名字"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText("旧名字")).toBeNull();
  }, 8000);

  it("waits for a slow read instead of discarding it on every poll", async () => {
    vi.useFakeTimers();
    try {
      const read = vi.fn(() => new Promise<InstancePage>((resolve) => setTimeout(() => resolve(page()), 2500)));
      render(<InstancesPage source={{ read, actions: {} }} />);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2400);
      });
      expect(read).toHaveBeenCalledTimes(1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(200);
      });
      expect(screen.getAllByText("Plus 1 号").length).toBeGreaterThan(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("refuses every entry once only an earlier read is on screen, including Enter in an open form", async () => {
    let failing = false;
    const read = async () => {
      if (failing) throw new Error("宿主没有响应");
      return page();
    };
    const create = vi.fn(async () => {});
    render(<InstancesPage source={{ read, actions: { create } }} />);
    fireEvent.click(await screen.findByRole("button", { name: "新建 Codex 实例" }));
    const name = screen.getByLabelText("名字");
    fireEvent.change(name, { target: { value: "Plus 2 号" } });
    failing = true;
    expect(await screen.findByText(/读不到最新状态/, undefined, { timeout: 2500 })).toBeTruthy();
    fireEvent.keyDown(name, { key: "Enter" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(create).not.toHaveBeenCalled();
  }, 8000);
});
