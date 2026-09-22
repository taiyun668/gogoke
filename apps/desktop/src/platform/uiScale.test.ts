/* @vitest-environment jsdom */
import { beforeEach, describe, expect, it, vi } from "vitest";

const isTauriMock = vi.hoisted(() => vi.fn());
const getCurrentWebviewMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ isTauri: isTauriMock }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: getCurrentWebviewMock,
}));

import { applyUiScale } from "./uiScale";

describe("applyUiScale", () => {
  beforeEach(() => {
    document.documentElement.style.zoom = "";
    isTauriMock.mockReset();
    getCurrentWebviewMock.mockReset();
  });

  it("uses browser zoom without touching the Tauri webview", async () => {
    isTauriMock.mockReturnValue(false);

    await applyUiScale(1.2);

    expect(document.documentElement.style.zoom).toBe("1.2");
    expect(getCurrentWebviewMock).not.toHaveBeenCalled();
  });

  it("keeps native scaling on the Tauri webview", async () => {
    const setZoom = vi.fn(async () => undefined);
    isTauriMock.mockReturnValue(true);
    getCurrentWebviewMock.mockReturnValue({ setZoom });

    await applyUiScale(1.1);

    expect(setZoom).toHaveBeenCalledWith(1.1);
  });
});
