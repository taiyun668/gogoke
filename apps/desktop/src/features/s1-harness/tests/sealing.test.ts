// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import type { AppSettings } from "@/types";
import { connectWorkspace, listWorkspaces } from "@/services/tauri";
import { describe, expect, it, vi } from "vitest";
import dictationController from "../../app/hooks/useDictationController.ts?raw";
import dictationHook from "../../dictation/hooks/useDictation.ts?raw";
import dictationModel from "../../dictation/hooks/useDictationModel.ts?raw";
import { useMobileServerSetup } from "../../mobile/hooks/useMobileServerSetup";
import serverHook from "../../settings/hooks/useSettingsServerSection.ts?raw";
import serverView from "../../settings/components/sections/SettingsServerSection.tsx?raw";

vi.mock("@/services/tauri", () => ({
  connectWorkspace: vi.fn(),
  listWorkspaces: vi.fn(),
}));

vi.mock("../../../utils/platformPaths", () => ({
  isMobilePlatform: vi.fn(() => true),
}));

describe("S1 sealed UI surfaces", () => {
  it("does not retain microphone or dictation mutation calls", () => {
    const combined = `${dictationController}\n${dictationHook}\n${dictationModel}`;
    for (const call of ["requestDictationPermission(", "startDictation(", "stopDictation(", "dictationDownloadModel(", "dictationRemoveModel("]) {
      expect(combined).not.toContain(call);
    }
    expect(combined).toContain("getDictationModelStatus");
  });

  it("does not auto-probe remote daemon or Tailscale", () => {
    expect(serverHook).not.toContain("listWorkspaces(");
    expect(serverHook).toContain("remoteAccessSealed = true");
    expect(serverHook).not.toContain("handleRefreshTailscaleCommandPreview();");
    expect(serverHook).not.toContain("handleTcpDaemonStatus();");
  });

  it("keeps remote controls visibly disabled", () => {
    expect(serverView).toContain("remoteAccessSealed");
    expect(serverView).toContain("disabled={remoteAccessSealed");
  });

  it("keeps mobile onboarding sealed without probing, connecting, or saving", async () => {
    const listWorkspacesMock = vi.mocked(listWorkspaces);
    const connectWorkspaceMock = vi.mocked(connectWorkspace);
    listWorkspacesMock.mockClear();
    connectWorkspaceMock.mockClear();
    const queueSaveSettings = vi.fn();
    const refreshWorkspaces = vi.fn();
    const appSettings = {
      remoteBackendHost: "desktop.example:4732",
      remoteBackendToken: "synthetic-token",
      remoteBackends: [],
      activeRemoteBackendId: null,
    } as unknown as AppSettings;

    const { result } = renderHook(() =>
      useMobileServerSetup({
        appSettings,
        appSettingsLoading: false,
        queueSaveSettings,
        refreshWorkspaces,
      }),
    );

    await waitFor(() => {
      expect(result.current.mobileSetupWizardProps.statusMessage).toMatch(/sealed/i);
    });
    expect(listWorkspacesMock).toHaveBeenCalledTimes(0);
    expect(connectWorkspaceMock).toHaveBeenCalledTimes(0);
    expect(refreshWorkspaces).toHaveBeenCalledTimes(0);

    await act(async () => {
      result.current.mobileSetupWizardProps.onConnectTest();
      await result.current.handleMobileConnectSuccess();
    });
    expect(queueSaveSettings).toHaveBeenCalledTimes(0);
    expect(listWorkspacesMock).toHaveBeenCalledTimes(0);
    expect(connectWorkspaceMock).toHaveBeenCalledTimes(0);
    expect(refreshWorkspaces).toHaveBeenCalledTimes(0);
  });
});
