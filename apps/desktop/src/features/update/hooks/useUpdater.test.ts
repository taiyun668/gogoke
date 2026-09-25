// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DebugEntry } from "../../../types";
import { useUpdater } from "./useUpdater";
import { STORAGE_KEY_PENDING_POST_UPDATE_VERSION } from "../utils/postUpdateRelease";
import { checkGogokeUpdate, installGogokeUpdate } from "../../../services/tauri";

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: vi.fn(() => true),
}));

vi.mock("../../../services/tauri", () => ({
  checkGogokeUpdate: vi.fn(),
  installGogokeUpdate: vi.fn(),
  takeGogokeUpdateFailure: vi.fn(),
}));

const checkMock = vi.mocked(checkGogokeUpdate);
const installMock = vi.mocked(installGogokeUpdate);
const fetchMock = vi.fn();

describe("useUpdater", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    fetchMock.mockReset();
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("sets error state when update check fails", async () => {
    checkMock.mockRejectedValue(new Error("nope"));
    const onDebug = vi.fn();
    const { result } = renderHook(() => useUpdater({ onDebug }));

    await act(async () => {
      await result.current.startUpdate();
    });

    expect(result.current.state.stage).toBe("error");
    expect(result.current.state.error).toBe("nope");
    expect(onDebug).toHaveBeenCalledWith(
      expect.objectContaining({
        id: expect.any(String),
        timestamp: expect.any(Number),
        label: "updater/error",
        source: "error",
        payload: "nope",
      } satisfies Partial<DebugEntry>),
    );
  });

  it("returns to idle when no update is available", async () => {
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater({}));

    await act(async () => {
      await result.current.startUpdate();
    });

    expect(result.current.state.stage).toBe("idle");
  });

  it("announces when no update is available for manual checks", async () => {
    vi.useFakeTimers();
    checkMock.mockResolvedValue(null);
    const { result } = renderHook(() => useUpdater({}));

    await act(async () => {
      await result.current.checkForUpdates({ announceNoUpdate: true });
    });

    expect(result.current.state.stage).toBe("latest");

    await act(async () => {
      vi.advanceTimersByTime(2000);
    });

    expect(result.current.state.stage).toBe("idle");
  });

  it("installs a prepared gogoke update and enters restart state", async () => {
    checkMock.mockResolvedValue({
      version: "1.2.3",
      releaseType: "full",
      asset: "gogoke-1.2.3-windows-x64-unsigned-setup.exe",
      sha256: "a".repeat(64),
      publishedAt: "2026-09-15T00:00:00Z",
      notesUrl: "https://github.com/taiyun668/gogoke/releases/tag/v1.2.3",
      notes: "",
    });
    installMock.mockResolvedValue();

    const { result } = renderHook(() => useUpdater({}));

    await act(async () => {
      await result.current.startUpdate();
    });

    expect(result.current.state.stage).toBe("available");
    expect(result.current.state.version).toBe("1.2.3");

    await act(async () => {
      await result.current.startUpdate();
    });

    await waitFor(() => expect(result.current.state.stage).toBe("restarting"));
    expect(installMock).toHaveBeenCalledWith("1.2.3");
    expect(
      window.localStorage.getItem(STORAGE_KEY_PENDING_POST_UPDATE_VERSION),
    ).toBe("1.2.3");
  });

  it("resets to idle on dismiss", async () => {
    checkMock.mockResolvedValue({
      version: "1.0.0",
      releaseType: "full",
      asset: "gogoke-1.0.0-windows-x64-unsigned-setup.exe",
      sha256: "b".repeat(64),
      publishedAt: "2026-09-15T00:00:00Z",
      notesUrl: "https://github.com/taiyun668/gogoke/releases/tag/v1.0.0",
      notes: "",
    });
    const { result } = renderHook(() => useUpdater({}));

    await act(async () => {
      await result.current.startUpdate();
    });

    await act(async () => {
      await result.current.dismiss();
    });

    expect(result.current.state.stage).toBe("idle");
  });

  it("surfaces verified installer launch errors", async () => {
    checkMock.mockResolvedValue({
      version: "2.0.0",
      releaseType: "full",
      asset: "gogoke-2.0.0-windows-x64-unsigned-setup.exe",
      sha256: "c".repeat(64),
      publishedAt: "2026-09-15T00:00:00Z",
      notesUrl: "https://github.com/taiyun668/gogoke/releases/tag/v2.0.0",
      notes: "",
    });
    installMock.mockRejectedValue(new Error("installer launch failed"));
    const onDebug = vi.fn();
    const { result } = renderHook(() => useUpdater({ onDebug }));

    await act(async () => {
      await result.current.startUpdate();
    });

    await act(async () => {
      await result.current.startUpdate();
    });

    await waitFor(() => expect(result.current.state.stage).toBe("error"));
    expect(result.current.state.error).toBe("installer launch failed");
    expect(onDebug).toHaveBeenCalledWith(
      expect.objectContaining({
        id: expect.any(String),
        timestamp: expect.any(Number),
        label: "updater/error",
        source: "error",
        payload: "installer launch failed",
      } satisfies Partial<DebugEntry>),
    );
  });

  it("does not run updater workflow when disabled", async () => {
    checkMock.mockResolvedValue({
      version: "9.9.9",
      releaseType: "full",
      asset: "gogoke-9.9.9-windows-x64-unsigned-setup.exe",
      sha256: "d".repeat(64),
      publishedAt: "2026-09-15T00:00:00Z",
      notesUrl: "https://github.com/taiyun668/gogoke/releases/tag/v9.9.9",
      notes: "",
    });
    const { result } = renderHook(() => useUpdater({ enabled: false }));

    await act(async () => {
      await result.current.checkForUpdates({ announceNoUpdate: true });
      await result.current.startUpdate();
    });

    expect(checkMock).not.toHaveBeenCalled();
    expect(result.current.state.stage).toBe("idle");
  });

  it("skips automatic startup checks when auto-check is disabled but still allows manual checks", async () => {
    checkMock.mockResolvedValue(null);

    const { result } = renderHook(() =>
      useUpdater({ autoCheckOnMount: false }),
    );

    expect(checkMock).not.toHaveBeenCalled();

    await act(async () => {
      await result.current.checkForUpdates({ announceNoUpdate: true });
    });

    expect(checkMock).toHaveBeenCalledTimes(1);
    expect(result.current.state.stage).toBe("latest");
  });

  it("loads post-update release notes after restart when marker matches current version", async () => {
    window.localStorage.setItem(
      STORAGE_KEY_PENDING_POST_UPDATE_VERSION,
      __APP_VERSION__,
    );
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({
        tag_name: `v${__APP_VERSION__}`,
        html_url: `https://github.com/taiyun668/gogoke/releases/tag/v${__APP_VERSION__}`,
        body: "## New\n- Added updater notes",
      }),
    } as Response);

    const { result } = renderHook(() => useUpdater({}));

    await waitFor(() =>
      expect(result.current.postUpdateNotice?.stage).toBe("ready"),
    );

    expect(result.current.postUpdateNotice).toMatchObject({
      stage: "ready",
      version: __APP_VERSION__,
      htmlUrl: `https://github.com/taiyun668/gogoke/releases/tag/v${__APP_VERSION__}`,
      body: "## New\n- Added updater notes",
    });

    await act(async () => {
      result.current.dismissPostUpdateNotice();
    });
    expect(result.current.postUpdateNotice).toBeNull();
    expect(
      window.localStorage.getItem(STORAGE_KEY_PENDING_POST_UPDATE_VERSION),
    ).toBeNull();
  });

  it("shows post-update fallback when release notes fetch fails", async () => {
    window.localStorage.setItem(
      STORAGE_KEY_PENDING_POST_UPDATE_VERSION,
      __APP_VERSION__,
    );
    fetchMock.mockRejectedValue(new Error("offline"));
    const onDebug = vi.fn();
    const { result } = renderHook(() => useUpdater({ onDebug }));

    await waitFor(() =>
      expect(result.current.postUpdateNotice?.stage).toBe("fallback"),
    );

    expect(result.current.postUpdateNotice).toMatchObject({
      stage: "fallback",
      version: __APP_VERSION__,
      htmlUrl: `https://github.com/taiyun668/gogoke/releases/tag/v${__APP_VERSION__}`,
    });
    expect(onDebug).toHaveBeenCalledWith(
      expect.objectContaining({
        label: "updater/release-notes-error",
        source: "error",
      }),
    );
  });

  it("does not reopen post-update toast after dismissing during loading", async () => {
    window.localStorage.setItem(
      STORAGE_KEY_PENDING_POST_UPDATE_VERSION,
      __APP_VERSION__,
    );

    let resolveFetch: ((value: Response) => void) | null = null;
    fetchMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveFetch = resolve as (value: Response) => void;
        }),
    );

    const { result } = renderHook(() => useUpdater({}));

    await waitFor(() =>
      expect(result.current.postUpdateNotice?.stage).toBe("loading"),
    );

    await act(async () => {
      result.current.dismissPostUpdateNotice();
    });

    expect(result.current.postUpdateNotice).toBeNull();
    expect(
      window.localStorage.getItem(STORAGE_KEY_PENDING_POST_UPDATE_VERSION),
    ).toBeNull();

    await act(async () => {
      resolveFetch?.({
        ok: true,
        status: 200,
        json: async () => ({
          tag_name: `v${__APP_VERSION__}`,
          html_url: `https://github.com/taiyun668/gogoke/releases/tag/v${__APP_VERSION__}`,
          body: "## Notes",
        }),
      } as Response);
      await Promise.resolve();
    });

    expect(result.current.postUpdateNotice).toBeNull();
  });

  it("clears stale post-update marker when version does not match current app", async () => {
    window.localStorage.setItem(
      STORAGE_KEY_PENDING_POST_UPDATE_VERSION,
      "0.0.1",
    );

    const { result } = renderHook(() => useUpdater({}));

    await waitFor(() =>
      expect(
        window.localStorage.getItem(STORAGE_KEY_PENDING_POST_UPDATE_VERSION),
      ).toBeNull(),
    );
    expect(result.current.postUpdateNotice).toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
