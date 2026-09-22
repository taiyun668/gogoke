/** @vitest-environment jsdom */
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceInfo } from "../../../types";
import { useWorkspaceActions } from "./useWorkspaceActions";

describe("useWorkspaceActions", () => {
  const workspace: WorkspaceInfo = {
    id: "ws-1",
    name: "Workspace",
    path: "/tmp/workspace",
    connected: true,
    settings: {
      sidebarCollapsed: false,
    },
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("opens one new-agent draft in the selected workspace", async () => {
    const setActiveThreadId = vi.fn();
    const startNewAgentDraft = vi.fn();

    const { result } = renderHook(() =>
      useWorkspaceActions({
        isCompact: false,
        addWorkspace: vi.fn(async () => null),
        addWorkspaceFromPath: vi.fn(async () => null),
        addWorkspaceFromGitUrl: vi.fn(async () => null),
        addWorkspacesFromPaths: vi.fn(async () => null),
        setActiveThreadId,
        setActiveTab: vi.fn(),
        exitDiffView: vi.fn(),
        selectWorkspace: vi.fn(),
        onStartNewAgentDraft: startNewAgentDraft,
        openWorktreePrompt: vi.fn(),
        openClonePrompt: vi.fn(),
        composerInputRef: { current: null },
        onDebug: vi.fn(),
      }),
    );

    await act(async () => {
      await result.current.handleAddAgent(workspace);
    });

    expect(setActiveThreadId).toHaveBeenCalledWith(null, "ws-1");
    expect(startNewAgentDraft).toHaveBeenCalledWith("ws-1");
  });
});
