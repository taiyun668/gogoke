// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
});
