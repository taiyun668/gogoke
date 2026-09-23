import { useCallback, useEffect, useMemo, useState } from "react";
import type { AppSettings } from "../../../types";
import { isMobilePlatform } from "../../../utils/platformPaths";
import type { MobileServerSetupWizardProps } from "../components/MobileServerSetupWizard";

const MOBILE_SETUP_MODE = "preserved_disabled" as const;
const MOBILE_SETUP_SEALED_MESSAGE =
  MOBILE_SETUP_MODE === "preserved_disabled"
    ? "Mobile remote setup is sealed in this build; existing settings and assets remain preserved."
    : "Mobile remote setup is unavailable.";

type UseMobileServerSetupParams = {
  appSettings: AppSettings;
  appSettingsLoading: boolean;
  queueSaveSettings: (next: AppSettings) => Promise<AppSettings>;
  refreshWorkspaces: () => Promise<unknown>;
};

type UseMobileServerSetupResult = {
  isMobileRuntime: boolean;
  showMobileSetupWizard: boolean;
  mobileSetupWizardProps: MobileServerSetupWizardProps;
  handleMobileConnectSuccess: () => Promise<void>;
};

export function useMobileServerSetup({
  appSettings,
  appSettingsLoading,
}: UseMobileServerSetupParams): UseMobileServerSetupResult {
  const isMobileRuntime = useMemo(() => isMobilePlatform(), []);

  const [remoteHostDraft, setRemoteHostDraft] = useState(appSettings.remoteBackendHost);
  const [remoteTokenDraft, setRemoteTokenDraft] = useState(appSettings.remoteBackendToken ?? "");
  const [checking, setChecking] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [statusError, setStatusError] = useState(false);
  const [mobileServerReady, setMobileServerReady] = useState(!isMobileRuntime);
  const [setupWizardDismissed, setSetupWizardDismissed] = useState(false);

  useEffect(() => {
    if (!isMobileRuntime) {
      return;
    }
    setRemoteHostDraft(appSettings.remoteBackendHost);
    setRemoteTokenDraft(appSettings.remoteBackendToken ?? "");
  }, [
    appSettings.remoteBackendHost,
    appSettings.remoteBackendToken,
    isMobileRuntime,
  ]);

  const onConnectTest = useCallback(() => {
    if (!isMobileRuntime) {
      return;
    }
    setMobileServerReady(false);
    setStatusError(true);
    setStatusMessage(MOBILE_SETUP_SEALED_MESSAGE);
  }, [isMobileRuntime]);

  useEffect(() => {
    if (!isMobileRuntime || appSettingsLoading) {
      return;
    }
    setMobileServerReady(false);
    setChecking(false);
    setStatusError(true);
    setStatusMessage(MOBILE_SETUP_SEALED_MESSAGE);
  }, [appSettingsLoading, isMobileRuntime]);

  const handleMobileConnectSuccess = useCallback(async () => {
    if (!isMobileRuntime) {
      return;
    }
    setMobileServerReady(false);
    setChecking(false);
    setStatusError(true);
    setStatusMessage(MOBILE_SETUP_SEALED_MESSAGE);
  }, [isMobileRuntime]);

  return {
    isMobileRuntime,
    showMobileSetupWizard:
      isMobileRuntime && !appSettingsLoading && !mobileServerReady && !setupWizardDismissed,
    mobileSetupWizardProps: {
      remoteHostDraft,
      remoteTokenDraft,
      busy: false,
      checking,
      statusMessage,
      statusError,
      onClose: () => {
        setSetupWizardDismissed(true);
      },
      onRemoteHostChange: setRemoteHostDraft,
      onRemoteTokenChange: setRemoteTokenDraft,
      onConnectTest,
    },
    handleMobileConnectSuccess,
  };
}
