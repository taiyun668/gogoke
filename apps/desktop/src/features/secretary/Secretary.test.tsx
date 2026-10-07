// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SecretaryActionLine, SecretaryEntry, SecretaryPanel, type SecretarySource } from "./Secretary";
import type { SecretaryPage } from "./secretaryModel";

function page(overrides: Partial<SecretaryPage> = {}): SecretaryPage {
  return {
    entry: { kind: "quiet", lastActivity: "10:20 汇总了 3 个项目" },
    routines: [
      { id: "r1", name: "每天早上汇总", schedule: "每天 9:00", nextRun: "明天 9:00", lastRun: { at: "今天 9:00", ok: true }, paused: false },
      { id: "r2", name: "盯云端测试", schedule: "每 30 分钟", nextRun: "10:30", lastRun: { at: "10:00", ok: false, note: "连不上 GitHub" }, paused: false },
    ],
    settings: {
      instanceId: "g1",
      model: "Grok 5",
      effort: "中",
      permission: "只读，能派活和转达",
      instances: [
        { id: "g1", name: "SuperGrok 第 1 个", models: ["Grok 5"] },
        { id: "c1", name: "Claude Pro" },
      ],
      efforts: ["中", "高"],
      permissions: ["只读，能派活和转达"],
      can: ["给各项目的主控派活、追进度"],
      cannot: ["不直接改项目里的文件"],
    },
    ...overrides,
  };
}

function source(data: SecretaryPage | null, actions: SecretarySource["actions"] = {}): SecretarySource {
  return { read: async () => data, actions };
}

describe("SecretaryEntry", () => {
  afterEach(cleanup);

  it("carries its state in one line and never anything louder", () => {
    const { rerender } = render(<SecretaryEntry state={{ kind: "waiting", count: 2 }} onOpen={() => {}} />);
    expect(screen.getByText("2 件等你")).toBeTruthy();
    rerender(<SecretaryEntry state={{ kind: "down", reason: "它的实例额度用完了", recoversAt: "12:40" }} onOpen={() => {}} />);
    expect(screen.getByText("用不了：它的实例额度用完了，12:40 恢复")).toBeTruthy();
    rerender(<SecretaryEntry state={{ kind: "unset" }} onOpen={() => {}} />);
    expect(screen.getByText(/还没设置/)).toBeTruthy();
  });
});

describe("SecretaryActionLine", () => {
  afterEach(cleanup);

  it("shows one line, the verbatim text on demand, and says plainly when delivery is unconfirmed", () => {
    const onOpen = vi.fn();
    render(
      <SecretaryActionLine
        line={{ id: "a1", kind: "sent", target: "gogoke 的主控", text: "把旁聊挂到右侧面板", delivery: "unknown", verbatim: "Owner 要求：把旁聊挂上", canOpen: true }}
        onOpen={onOpen}
      />,
    );
    expect(screen.getByText(/未能确认送达/)).toBeTruthy();
    expect(screen.queryByText(/Owner 要求/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /把旁聊挂到右侧面板/ }));
    expect(screen.getByText(/Owner 要求：把旁聊挂上/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "打开 →" }));
    expect(onOpen).toHaveBeenCalledWith("a1");
  });

  it("points to a project's question instead of carrying it", () => {
    render(<SecretaryActionLine line={{ id: "p1", kind: "pointer", target: "短视频", text: "主控有一个要你定的问题", canOpen: true }} onOpen={() => {}} />);
    expect(screen.getByRole("button", { name: "去回答 →" })).toBeTruthy();
    expect(screen.queryByRole("textbox")).toBeNull();
  });
});

describe("SecretaryPanel", () => {
  afterEach(cleanup);

  it("says plainly when the host has no secretary data", async () => {
    render(<SecretaryPanel source={source(null)} />);
    expect(await screen.findByText("秘书长的数据还没接上。")).toBeTruthy();
  });

  it("lists routines with when they run and how the last run went, and deletes only after confirming", async () => {
    const deleteRoutine = vi.fn(async () => {});
    render(<SecretaryPanel source={source(page(), { deleteRoutine, pauseRoutine: vi.fn(async () => {}) })} />);
    expect(await screen.findByText("每天 9:00 · 下次 明天 9:00")).toBeTruthy();
    expect(screen.getByText("上次 10:00 · 连不上 GitHub")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "删除" })[0]);
    expect(deleteRoutine).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "确认删除" }));
    await waitFor(() => expect(deleteRoutine).toHaveBeenCalledWith("r1"));
  });

  it("offers no new-routine button: routines are made by asking it", async () => {
    render(<SecretaryPanel source={source(page({ routines: [] }))} />);
    expect(await screen.findByText(/跟它说一句就能建/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /新建/ })).toBeNull();
  });

  it("offers resuming everything after routines were paused while the Owner was away", async () => {
    const resumeAll = vi.fn(async () => {});
    render(<SecretaryPanel source={source(page({ pausedWhileAway: { awayFor: "5 天" } }), { resumeAll })} />);
    fireEvent.click(await screen.findByRole("button", { name: "都恢复" }));
    await waitFor(() => expect(resumeAll).toHaveBeenCalled());
  });

  it("offers only the chosen instance's verified models and saves no model for an instance without them", async () => {
    const saveSettings = vi.fn(async () => {});
    render(<SecretaryPanel source={source(page(), { saveSettings })} initialTab="settings" />);
    expect(((await screen.findByLabelText("模型")) as HTMLSelectElement).value).toBe("Grok 5");
    fireEvent.change(screen.getByLabelText("实例"), { target: { value: "c1" } });
    expect(screen.queryByLabelText("模型")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(saveSettings).toHaveBeenCalledWith({ instanceId: "c1", effort: "中", permission: "只读，能派活和转达" }));
    expect(screen.getByText("不直接改项目里的文件")).toBeTruthy();
  });
});
