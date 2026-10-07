// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NowBlock, NowPin, StopMenu } from "./NowBlock";
import { batchWord, type NowBatch, type SeatLine } from "./nowModel";

function seat(overrides: Partial<SeatLine> = {}): SeatLine {
  return {
    id: "b",
    name: "施工 · 实例页",
    vendor: "codex",
    who: "Plus 1 号 · GPT-6.1 Sol · 高",
    state: "running",
    lastAction: { verb: "改", target: "instancePageModel.ts" },
    ask: "按复核意见改实例页 8 条；配色和布局不动。",
    bounds: "只在它自己的工作树里改",
    steps: [{ verb: "读", target: "InstancesPage.tsx" }, { verb: "改", target: "instancePageModel.ts" }],
    startedAt: Date.now() - 48_000,
    ...overrides,
  };
}

function batch(overrides: Partial<NowBatch> = {}): NowBatch {
  return {
    id: "batch-1",
    title: "实例页按复核意见改",
    closed: false,
    seats: [
      seat(),
      seat({ id: "t", name: "施工 · 测试", vendor: "opencode", lastAction: { verb: "读", target: "Design37InstanceSection.test.tsx" } }),
      seat({ id: "a", name: "审计", vendor: "claude", lastAction: { verb: "读", target: "PR #71 复核意见" } }),
    ],
    ...overrides,
  };
}

describe("NowBlock", () => {
  afterEach(cleanup);

  it("shows each running seat's latest action and no status words on the lines", () => {
    render(<NowBlock batch={batch()} />);
    expect(screen.getByText("正在进行")).toBeTruthy();
    expect(screen.getByText("3 处理中")).toBeTruthy();
    expect(screen.getByText("instancePageModel.ts")).toBeTruthy();
    expect(screen.queryByText("运行中")).toBeNull();
  });

  it("says a just-dispatched seat is waiting for its first action", () => {
    render(<NowBlock batch={batch({ seats: [seat({ state: "starting", lastAction: undefined, steps: [] })] })} />);
    expect(screen.getByText("已派出去，等它第一个动作")).toBeTruthy();
  });

  it("replaces a returned line with the first sentence it handed back, and a failure with what went wrong", () => {
    render(
      <NowBlock
        batch={batch({
          seats: [
            seat({ state: "returned", say: "实例页 8 条都改了，配色和布局一行没动。", diff: { added: 52, removed: 18 } }),
            seat({ id: "t", name: "施工 · 测试", state: "failed", say: "测试跑不起来：依赖还没装好。", fix: "已交给主控" }),
          ],
        })}
      />,
    );
    expect(screen.getByText("实例页 8 条都改了，配色和布局一行没动。")).toBeTruthy();
    expect(screen.getByText("测试跑不起来：依赖还没装好。")).toBeTruthy();
    expect(screen.getByText("+52")).toBeTruthy();
    expect(screen.getByText("1 已交回 · 1 失败")).toBeTruthy();
  });

  it("opens one seat at a time with the four handover facts and commit messages", () => {
    const openInGit = vi.fn();
    const data = batch({
      seats: [
        seat({ state: "returned", say: "改好了。", commits: ["实例页：按复核意见改 8 条"], report: ["改好了。"] }),
        seat({ id: "a", name: "审计", vendor: "claude", checks: "施工交回的 2 个提交" }),
      ],
    });
    render(<NowBlock batch={data} actions={{ openInGit }} />);
    fireEvent.click(screen.getByRole("button", { name: /施工 · 实例页/ }));
    for (const label of ["谁接的", "派的活", "边界"]) expect(screen.getByText(label)).toBeTruthy();
    expect(screen.getByText("实例页：按复核意见改 8 条")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "在 Git 里看这些提交" }));
    expect(openInGit).toHaveBeenCalledWith("b");
    fireEvent.click(screen.getByRole("button", { name: /审计/ }));
    expect(screen.getByText("核对的版本")).toBeTruthy();
    expect(screen.queryByText("实例页：按复核意见改 8 条")).toBeNull();
  });

  it("folds a closed batch to one line with one word, and offers merging only when the host can", () => {
    const merge = vi.fn();
    const closed = batch({
      closed: true,
      landing: { commits: 2, merged: false },
      seats: [seat({ state: "returned", say: "改好了。", diff: { added: 52, removed: 18 } })],
    });
    const { rerender } = render(<NowBlock batch={closed} />);
    expect(screen.getByText("改动就绪")).toBeTruthy();
    expect(screen.getByText(/2 个提交 · 还在席位的工作树里，没合并/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "并进来" })).toBeNull();
    rerender(<NowBlock batch={closed} actions={{ merge }} />);
    fireEvent.click(screen.getByRole("button", { name: "并进来" }));
    expect(merge).toHaveBeenCalledWith("batch-1");
    rerender(<NowBlock batch={{ ...closed, landing: { commits: 2, merged: true } }} actions={{ merge }} />);
    expect(screen.getByText(/已合并进主树/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "并进来" })).toBeNull();
  });

  it("puts live and problem lines first and folds the rest behind one line when many seats run", () => {
    const many = Array.from({ length: 8 }, (_, index) =>
      seat({ id: `s${index}`, name: `席位 ${index}`, state: index < 5 ? "returned" : "running", say: `结论 ${index}` }),
    );
    render(<NowBlock batch={batch({ seats: many })} />);
    expect(screen.getByRole("button", { name: /席位 7/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /席位 4/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "另外 3 个已交回，点开看" }));
    expect(screen.getByRole("button", { name: /席位 4/ })).toBeTruthy();
  });

  it("marks an audit verdict that no longer applies", () => {
    render(<NowBlock batch={batch({ seats: [seat({ id: "a", name: "审计", state: "stale", say: "核对完之后施工又改了 1 个文件，这个结论要重新核" })] })} />);
    expect(screen.getByText("1 结论要重新核")).toBeTruthy();
    expect(screen.getByText(/这个结论要重新核/)).toBeTruthy();
  });

  it("shows the last known state with its time when the host cannot be read", () => {
    const { container } = render(<NowBlock batch={batch()} frozenAt="10:42" />);
    expect(screen.getByText(/10:42 最后一次读到的样子/)).toBeTruthy();
    expect(container.querySelector(".now.is-frozen")).toBeTruthy();
    expect(container.querySelector(".now-flow")).toBeNull();
  });

  it("reports the same word as the conversation row; 待处理 only for a real CLI question", () => {
    expect(batchWord(batch(), true)).toBe("待处理");
    expect(batchWord(batch(), false)).toBe("在运行");
    expect(batchWord(batch({ closed: true, landing: { commits: 2, merged: false }, seats: [seat({ state: "returned" })] }), false)).toBe("改动就绪");
    expect(batchWord(batch({ closed: true, seats: [seat({ state: "stopped" })] }), false)).toBeNull();
    expect(batchWord(null, false)).toBeNull();
  });
});

describe("elapsed", () => {
  it("reads minutes and seconds, and hours once past an hour", async () => {
    const { elapsed } = await import("./nowModel");
    expect(elapsed(48_000)).toBe("0:48");
    expect(elapsed(3_725_000)).toBe("1:02:05");
  });
});

describe("NowPin and StopMenu", () => {
  afterEach(cleanup);

  it("jumps back to the block from the pinned line", () => {
    const onJump = vi.fn();
    render(<NowPin batch={batch()} onJump={onJump} />);
    fireEvent.click(screen.getByRole("button", { name: /正在进行/ }));
    expect(onJump).toHaveBeenCalled();
  });

  it("offers only the kinds of stop the host can do", () => {
    const onStopWork = vi.fn();
    const { rerender } = render(<StopMenu />);
    expect(screen.queryByRole("button", { name: "停止" })).toBeNull();
    rerender(<StopMenu onStopWork={onStopWork} />);
    fireEvent.click(screen.getByRole("button", { name: "停止" }));
    expect(screen.queryByRole("menuitem", { name: /停这一轮/ })).toBeNull();
    fireEvent.click(screen.getByRole("menuitem", { name: /停这件事/ }));
    expect(onStopWork).toHaveBeenCalled();
  });
});
