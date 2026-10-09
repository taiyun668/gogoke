// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SideChatPanel, type SideChatSource } from "./SideChatPanel";
import type { SideChat, SideChatPage } from "./sideChatModel";

function chat(overrides: Partial<SideChat> = {}): SideChat {
  return {
    id: "c1",
    title: "实例卡片还能看到报错吗",
    seatId: "audit",
    instanceId: "oc",
    model: "Grok 5",
    effort: "高",
    referenceRound: 18,
    pendingLeadSegments: 0,
    appendsImmediately: false,
    updatedAt: "5 分钟前",
    archived: false,
    answering: false,
    items: [
      { kind: "user", id: "u1", text: "出错时还能看到原始报错吗？" },
      { kind: "answer", id: "a1", text: "能，出错原因还留在卡片上。", basis: "10:41" },
    ],
    ...overrides,
  };
}

function page(chats: SideChat[] = [chat()]): SideChatPage {
  return {
    leadRound: 19,
    efforts: ["高", "中"],
    seats: [
      { id: "audit", name: "审计", defaultInstanceId: "oc", permission: "只读：能看本项目，不能改东西" },
      { id: "sec", name: "秘书长", defaultInstanceId: "cl", permission: "只读：能看本项目，不能改东西" },
    ],
    instances: [
      { id: "oc", name: "SuperGrok 第 2 个", vendor: "opencode", models: ["Grok 5", "Grok 5 mini"] },
      { id: "cl", name: "Claude Pro", vendor: "claude", models: ["Opus 5.5"] },
    ],
    chats,
  };
}

function source(data: SideChatPage | null, actions: SideChatSource["actions"] = {}): SideChatSource {
  return { read: async () => data, actions };
}

describe("SideChatPanel", () => {
  afterEach(() => {
    cleanup();
    window.localStorage.clear();
  });

  it("says plainly when the host has no side chat data yet", async () => {
    render(<SideChatPanel source={source(null)} />);
    expect(await screen.findByText(/旁聊数据还没接上/)).toBeTruthy();
  });

  it("lists kept chats with only the notes that matter and hides archived ones behind a link", async () => {
    render(
      <SideChatPanel
        source={source(page([chat({ pendingLeadSegments: 2 }), chat({ id: "c2", title: "旧问题", archived: true })]))}
      />,
    );
    expect(await screen.findByText("主控有 2 段新进展")).toBeTruthy();
    expect(screen.queryByText("旧问题")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "已归档 1 个" }));
    expect(screen.getByText("旧问题")).toBeTruthy();
  });

  it("shows relays as one plain line each way, with no relay buttons", async () => {
    const relays = chat({
      items: [
        { kind: "user", id: "u1", text: "这一条你告诉主控：版本号挪到右上角。" },
        { kind: "sent-to-lead", id: "s1", text: "版本号挪到右上角。", at: "10:52", result: "steered" },
        { kind: "from-lead", id: "l1", text: "收到，下一版一起改。", at: "10:53", delivery: "steered" },
        { kind: "sent-to-lead", id: "s2", text: "别动配色。", at: "10:55", result: "failed", error: "主控的实例额度用完了" },
      ],
    });
    render(<SideChatPanel source={source(page([relays]))} />);
    fireEvent.click(await screen.findByRole("button", { name: /实例卡片还能看到报错吗/ }));
    expect(screen.getByText("已发给主控 · 插进了主控正在跑的这一轮")).toBeTruthy();
    expect(screen.getByText(/来自主控 · 10:53/)).toBeTruthy();
    expect(screen.getByText("发给主控，没送到")).toBeTruthy();
    for (const name of ["插话", "编辑", "取消", "投递给主控", "发到主控收件箱"]) {
      expect(screen.queryByRole("button", { name })).toBeNull();
    }
    fireEvent.click(screen.getByText("发给主控，没送到"));
    expect(screen.getByText(/主控的实例额度用完了/)).toBeTruthy();
  });

  it("fixes who and which instance after the first question but keeps model choice", async () => {
    const setModel = vi.fn(async () => {});
    render(<SideChatPanel source={source(page(), { ask: vi.fn(async () => {}), setModel })} />);
    fireEvent.click(await screen.findByRole("button", { name: /实例卡片还能看到报错吗/ }));
    const who = screen.getByRole("button", { name: /审计 · SuperGrok 第 2 个/ }) as HTMLButtonElement;
    expect(who.disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: /Grok 5 · 高/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Grok 5 mini" }));
    await waitFor(() => expect(setModel).toHaveBeenCalledWith("c1", "Grok 5 mini", "高"));
  });

  it("asks with Enter but not while an IME is composing, and keeps drafts per chat", async () => {
    const ask = vi.fn(async () => {});
    render(<SideChatPanel source={source(page(), { ask })} />);
    fireEvent.click(await screen.findByRole("button", { name: /实例卡片还能看到报错吗/ }));
    const input = screen.getByLabelText("旁聊输入");
    fireEvent.change(input, { target: { value: "还有别的吗" } });
    fireEvent.compositionStart(input);
    fireEvent.keyDown(input, { key: "Enter" });
    expect(ask).not.toHaveBeenCalled();
    fireEvent.compositionEnd(input);
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(ask).toHaveBeenCalledWith("c1", "还有别的吗"));
  });

  it("chooses who and which instance in one chip before the first question", async () => {
    const create = vi.fn(async () => {});
    render(<SideChatPanel source={source(page(), { create })} />);
    fireEvent.click(await screen.findByRole("button", { name: "新开旁聊" }));
    expect(screen.getByText(/你发第一个问题之前，不会调用模型/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /审计 · SuperGrok 第 2 个/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "秘书长" }));
    expect(screen.getByRole("button", { name: /秘书长 · Claude Pro/ })).toBeTruthy();
    fireEvent.change(screen.getByLabelText("旁聊输入"), { target: { value: "M2 还剩什么" } });
    fireEvent.click(screen.getByRole("button", { name: "发问" }));
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith({ seatId: "sec", instanceId: "cl", model: "Opus 5.5", effort: "高", question: "M2 还剩什么" }),
    );
  });

  it("sends no model from the previous instance when the new one has none verified", async () => {
    const create = vi.fn(async () => {});
    const data = page();
    data.instances[1] = { ...data.instances[1], models: [] };
    render(<SideChatPanel source={source(data, { create })} />);
    fireEvent.click(await screen.findByRole("button", { name: "新开旁聊" }));
    fireEvent.click(screen.getByRole("button", { name: /审计 · SuperGrok 第 2 个/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Claude Pro" }));
    fireEvent.change(screen.getByLabelText("旁聊输入"), { target: { value: "还有什么" } });
    fireEvent.click(screen.getByRole("button", { name: "发问" }));
    await waitFor(() => expect(create).toHaveBeenCalled());
    expect(create).toHaveBeenCalledWith(expect.not.objectContaining({ model: expect.anything() }));
  });

  it("says plainly when delivery between windows was not confirmed", async () => {
    const relays = chat({
      items: [
        { kind: "sent-to-lead", id: "s1", text: "版本号挪到右上角。", at: "10:52", result: "unknown", error: "回执还没回来" },
        { kind: "from-lead", id: "l1", text: "收到。", at: "10:53", delivery: "unknown" },
      ],
    });
    render(<SideChatPanel source={source(page([relays]))} />);
    fireEvent.click(await screen.findByRole("button", { name: /实例卡片还能看到报错吗/ }));
    expect(screen.getByText("发给主控，未能确认送达")).toBeTruthy();
    expect(screen.getByText(/主控发来的，未能确认送达 · 10:53/)).toBeTruthy();
    expect(screen.queryByText(/已发给主控/)).toBeNull();
  });

  it("hides rounds, counts and model when the host does not report them", async () => {
    const bare = chat({ model: undefined, effort: undefined, referenceRound: undefined, pendingLeadSegments: undefined, updatedAt: undefined });
    const data = { ...page([bare]), leadRound: undefined };
    render(<SideChatPanel source={source(data, { ask: vi.fn(async () => {}) })} />);
    fireEvent.click(await screen.findByRole("button", { name: /实例卡片还能看到报错吗/ }));
    fireEvent.click(screen.getByRole("button", { name: "旁聊的更多操作" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "旁聊信息" }));
    expect(screen.queryByText("参考到")).toBeNull();
    expect(screen.queryByText(/undefined/)).toBeNull();
    expect(screen.queryByRole("button", { name: /Grok 5 · 高/ })).toBeNull();
  });

  it("lets the same sentence be sent again while the previous question's delivery is unconfirmed", async () => {
    const ask = vi.fn(async () => {});
    render(<SideChatPanel source={source(page([chat({ askable: false, questionUnconfirmed: true })]), { ask })} />);
    fireEvent.click(await screen.findByRole("button", { name: /实例卡片还能看到报错吗/ }));
    expect(screen.getByText(/现在只能重发同一句/)).toBeTruthy();
    const input = screen.getByLabelText("旁聊输入") as HTMLTextAreaElement;
    expect(input.disabled).toBe(false);
    fireEvent.change(input, { target: { value: "出错时还能看到原始报错吗？" } });
    fireEvent.click(screen.getByRole("button", { name: "发问" }));
    await waitFor(() => expect(ask).toHaveBeenCalledWith("c1", "出错时还能看到原始报错吗？"));
  });

  it("says a seat is not read yet, not removed, when the seats page is unavailable", async () => {
    const data = { ...page([chat({ seatId: "unknown-seat", instanceId: "" })]), seats: [], instances: [], seatsKnown: false };
    render(<SideChatPanel source={source(data, { ask: vi.fn(async () => {}) })} />);
    expect(await screen.findByText(/席位还没读到 · 实例未知/)).toBeTruthy();
    expect(screen.queryByText(/已删除的席位/)).toBeNull();
  });

  it("keeps a chat readable but not askable when its session is unreachable", async () => {
    render(<SideChatPanel source={source(page([chat({ problem: { kind: "unavailable" } })]))} />);
    fireEvent.click(await screen.findByRole("button", { name: /实例卡片还能看到报错吗/ }));
    expect(screen.getByText(/现在连不上这个旁聊的会话/)).toBeTruthy();
    expect(screen.queryByLabelText("旁聊输入")).toBeNull();
  });

  it("explains a removed seat and stops asking, without guessing", async () => {
    render(<SideChatPanel source={source(page([chat({ problem: { kind: "seat-removed", seatName: "审计" } })]))} />);
    fireEvent.click(await screen.findByRole("button", { name: /实例卡片还能看到报错吗/ }));
    expect(screen.getByText(/席位已经删除，这里只能看不能问/)).toBeTruthy();
    expect(screen.queryByLabelText("旁聊输入")).toBeNull();
  });

  it("keeps formal-review and reference details behind the menu", async () => {
    render(<SideChatPanel source={source(page([chat({ pendingLeadSegments: 1 })]))} />);
    fireEvent.click(await screen.findByRole("button", { name: /实例卡片还能看到报错吗/ }));
    expect(screen.queryByText(/旁聊不是正式审查/)).toBeNull();
    expect(screen.getByText("主控有 1 段新进展，下次提问时一起带上")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "旁聊的更多操作" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "旁聊信息" }));
    expect(screen.getByText(/旁聊不是正式审查/)).toBeTruthy();
  });

  it("waits for a slow read instead of discarding it on every poll", async () => {
    vi.useFakeTimers();
    try {
      const read = vi.fn(() => new Promise<SideChatPage>((resolve) => setTimeout(() => resolve(page()), 3500)));
      render(<SideChatPanel source={{ read, actions: {} }} />);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3400);
      });
      expect(read).toHaveBeenCalledTimes(1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(200);
      });
      expect(screen.getAllByText("实例卡片还能看到报错吗").length).toBeGreaterThan(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
