import { beforeEach, describe, expect, it, vi } from "vitest";

const isTauriMock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ isTauri: isTauriMock }));

import { appRuntime, hasNativeBackendTransport } from "./runtime";

describe("runtime identity", () => {
  beforeEach(() => isTauriMock.mockReset());

  it("does not pretend a browser preview has a control host", () => {
    isTauriMock.mockReturnValue(false);
    expect(appRuntime()).toBe("browser-preview");
    expect(hasNativeBackendTransport()).toBe(false);
  });

  it("recognizes the Tauri control-host transport", () => {
    isTauriMock.mockReturnValue(true);
    expect(appRuntime()).toBe("tauri");
    expect(hasNativeBackendTransport()).toBe(true);
  });
});
