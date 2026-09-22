import { describe, expect, it, vi } from "vitest";

const isTauriMock = vi.hoisted(() => vi.fn(() => false));
const getCurrentWindowMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ isTauri: isTauriMock }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: getCurrentWindowMock,
}));

import { subscribeWindowDragDrop } from "./dragDrop";

describe("subscribeWindowDragDrop", () => {
  it("does not request a native window in the browser", () => {
    const unsubscribe = subscribeWindowDragDrop(vi.fn());

    expect(getCurrentWindowMock).not.toHaveBeenCalled();

    unsubscribe();
  });
});
