// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SeatsPanel, type SeatsSource } from "./SeatsPanel";
import type { SeatsPage } from "./seatsPageModel";

const pro = { id: "codexTestM2", name: "Pro 主号", vendor: "codex" as const, models: ["GPT-6.1 Sol", "GPT-6.1 mini"] };
const claude = { id: "claudeTestM2", name: "Claude Pro", vendor: "claude" as const, models: ["Opus 5.5"] };
const all = { tune: true, changeInstance: true, remove: true };

function page(): SeatsPage {
  return {
    running: 2,
    limit: 4,
    instances: [pro, claude],
    efforts: ["高", "中"],
    permissions: ["只读", "可写自己的工作区"],
    templates: ["审计（从模板复制）"],
    range: { instanceIds: ["codexTestM2"], maxPermission: "可写自己的工作区", maxConcurrent: 4 },
    seats: [
      { id: "lead", name: "主控", layer: "direct", isLead: true, term: "long", state: "WORKING", instance: pro,
        allowed: { tune: true, changeInstance: false, remove: false }, instanceLockedReason: "它正在干活，这一轮结束后才能换实例",
        model: "GPT-6.1 Sol", effort: "高", permission: "可写本项目", doing: "在拆实例页的任务" },
      { id: "audit", name: "审计", layer: "direct", term: "long", state: "IDLE", instance: claude, allowed: all,
        model: "Opus 5.5", effort: "高", permission: "只读", lastActivity: "昨天 22:10 复核完" },
      { id: "test", name: "施工 · 测试", layer: "sub", term: "short", state: "STUCK", instance: claude, allowed: all,
        model: "Opus 5.5", effort: "中", permission: "可写自己的工作区", doing: "额度用完，停在当前这一轮", stuckFor: "35 分钟" },
      { id: "old", name: "施工 · 调研", layer: "sub", term: "short", state: "REMOVED", instance: claude,
        allowed: { tune: false, changeInstance: false, remove: false },
        model: "Opus 5.5", effort: "中", permission: "只读", removedNote: "主控回收" },
    ],
  };
}

function source(
  overrides: Partial<SeatsSource["actions"]> = {},
  read: SeatsSource["read"] = async () => page(),
): SeatsSource {
  return { read, actions: { create: vi.fn(async () => {}), tune: vi.fn(async () => {}), remove: vi.fn(async () => {}),
    setRange: vi.fn(async () => {}), ...overrides } };
}

describe("SeatsPanel", () => {
  afterEach(cleanup);

  it("says plainly when the host has no seat data yet", async () => {
    render(<SeatsPanel source={source({}, async () => null)} />);
    expect(await screen.findByText(/席位数据还没接上/)).toBeTruthy();
  });

  it("shows both groups with one plain line each and hides removed seats behind a link", async () => {
    render(<SeatsPanel source={source()} />);
    expect(await screen.findByText("直属席位")).toBeTruthy();
    expect(screen.getByText("下属席位")).toBeTruthy();
    expect(screen.getByText(/2 个在跑，上限 4 个/)).toBeTruthy();
    expect(screen.getByText(/卡住：施工 · 测试/)).toBeTruthy();
    expect(screen.getByText(/额度用完，停在当前这一轮 · 已经 35 分钟/)).toBeTruthy();
    expect(screen.queryByText("施工 · 调研")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /已删除 1 个/ }));
    expect(screen.getByText("施工 · 调研")).toBeTruthy();
  });

  it("opens and closes a card from its header, and never offers deleting the lead", async () => {
    render(<SeatsPanel source={source()} />);
    const head = await screen.findByRole("button", { name: /^主控/ });
    fireEvent.click(head);
    expect(screen.getByRole("button", { name: "调整" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "编排范围" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "删除席位" })).toBeNull();
    fireEvent.click(head);
    expect(screen.queryByRole("button", { name: "调整" })).toBeNull();
  });

  it("does not claim there is no pending question when the host does not say", async () => {
    render(<SeatsPanel source={source()} />);
    fireEvent.click(await screen.findByRole("button", { name: /^审计/ }));
    expect(screen.queryByText("待决问题")).toBeNull();
    expect(screen.queryByText("无")).toBeNull();
  });

  it("deletes a seat only after confirmation", async () => {
    const remove = vi.fn(async () => {});
    render(<SeatsPanel source={source({ remove })} />);
    fireEvent.click(await screen.findByRole("button", { name: /^审计/ }));
    fireEvent.click(screen.getByRole("button", { name: "删除席位" }));
    expect(remove).not.toHaveBeenCalled();
    expect(screen.getByText(/工作记录会保留/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "确认删除" }));
    await waitFor(() => expect(remove).toHaveBeenCalledWith("audit"));
  });

  it("locks the instance only when the host says so, and promises nothing about when changes apply", async () => {
    render(<SeatsPanel source={source()} />);
    fireEvent.click(await screen.findByRole("button", { name: /^主控/ }));
    fireEvent.click(screen.getByRole("button", { name: "调整" }));
    expect((screen.getByLabelText("实例") as HTMLSelectElement).disabled).toBe(true);
    expect(screen.getByText("它正在干活，这一轮结束后才能换实例")).toBeTruthy();
    expect(screen.queryByText(/下一轮/)).toBeNull();
  });

  it("follows the host's flags, not the displayed state", async () => {
    const data = page();
    data.seats[1] = { ...data.seats[1], allowed: { tune: false, changeInstance: false, remove: false } };
    render(<SeatsPanel source={source({}, async () => data)} />);
    fireEvent.click(await screen.findByRole("button", { name: /^审计/ }));
    expect(screen.queryByRole("button", { name: "调整" })).toBeNull();
    expect(screen.queryByRole("button", { name: "删除席位" })).toBeNull();
  });

  it("offers deleting a seat the host lets go even when it cannot be tuned", async () => {
    const data = page();
    data.seats[1] = { ...data.seats[1], allowed: { tune: false, changeInstance: false, remove: true } };
    render(<SeatsPanel source={source({}, async () => data)} />);
    fireEvent.click(await screen.findByRole("button", { name: /^审计/ }));
    expect(screen.queryByRole("button", { name: "调整" })).toBeNull();
    expect(screen.getByRole("button", { name: "删除席位" })).toBeTruthy();
  });

  it("drops the old source's seats when the source changes", async () => {
    const { rerender } = render(<SeatsPanel source={source()} />);
    expect(await screen.findByText("直属席位")).toBeTruthy();
    rerender(<SeatsPanel source={source({}, () => new Promise<SeatsPage>(() => {}))} />);
    expect(await screen.findByText("正在读取席位…")).toBeTruthy();
    expect(screen.queryByText("直属席位")).toBeNull();
  });

  it("lists the models of the chosen instance and hides the choice when the host reports none", async () => {
    const tune = vi.fn(async () => {});
    const data = page();
    data.instances = [pro, { ...claude, models: undefined }];
    data.seats[1] = { ...data.seats[1], instance: data.instances[1] };
    render(<SeatsPanel source={source({ tune }, async () => data)} />);
    fireEvent.click(await screen.findByRole("button", { name: /^审计/ }));
    fireEvent.click(screen.getByRole("button", { name: "调整" }));
    expect(screen.queryByLabelText("模型")).toBeNull();
    fireEvent.change(screen.getByLabelText("实例"), { target: { value: "codexTestM2" } });
    const model = screen.getByLabelText("模型") as HTMLSelectElement;
    expect([...model.options].map((option) => option.value)).toEqual(["GPT-6.1 Sol", "GPT-6.1 mini"]);
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(tune).toHaveBeenCalledWith("audit", expect.objectContaining({ instanceId: "codexTestM2", model: "GPT-6.1 Sol" })),
    );
  });

  it("applies only the latest read, so a poll that started before a write cannot overwrite it", async () => {
    const marked = (text: string) => {
      const data = page();
      data.seats[1] = { ...data.seats[1], lastActivity: text };
      return data;
    };
    let release: (value: SeatsPage) => void = () => {};
    let calls = 0;
    const read = () => {
      calls += 1;
      if (calls === 1) return Promise.resolve(page());
      if (calls === 2) return new Promise<SeatsPage>((resolve) => (release = resolve));
      return Promise.resolve(marked("新的读数"));
    };
    render(<SeatsPanel source={source({}, read)} />);
    fireEvent.click(await screen.findByRole("button", { name: /^审计/ }));
    fireEvent.click(screen.getByRole("button", { name: "删除席位" }));
    await waitFor(() => expect(calls).toBe(2), { timeout: 3000 });
    fireEvent.click(screen.getByRole("button", { name: "确认删除" }));
    expect(await screen.findByText(/新的读数/)).toBeTruthy();
    release(marked("旧的读数"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText(/旧的读数/)).toBeNull();
  }, 8000);

  it("adds only direct seats and rejects a duplicate name", async () => {
    const create = vi.fn(async () => {});
    render(<SeatsPanel source={source({ create })} />);
    fireEvent.click(await screen.findByRole("button", { name: "添加直属席位" }));
    fireEvent.change(screen.getByLabelText("名字"), { target: { value: "审计" } });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    expect(screen.getByRole("alert").textContent).toContain("已经有叫这个名字的席位");
    fireEvent.change(screen.getByLabelText("名字"), { target: { value: "审计 2" } });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    await waitFor(() => expect(create).toHaveBeenCalledWith(expect.objectContaining({ name: "审计 2", instanceId: "codexTestM2" })));
  });

  it("shows no controls the host cannot perform", async () => {
    render(<SeatsPanel source={{ read: async () => page(), actions: {} }} />);
    fireEvent.click(await screen.findByRole("button", { name: /^审计/ }));
    expect(screen.queryByRole("button", { name: "调整" })).toBeNull();
    expect(screen.queryByRole("button", { name: "删除席位" })).toBeNull();
    expect(screen.queryByRole("button", { name: "添加直属席位" })).toBeNull();
  });
});
