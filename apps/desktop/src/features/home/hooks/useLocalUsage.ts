import { useCallback, useEffect, useRef, useState } from "react";
import type { LocalUsageSnapshot } from "../../../types";
import { localUsageSnapshot } from "../../../services/tauri";
import { hasNativeBackendTransport } from "@/platform/runtime";

type LocalUsageState = {
  snapshot: LocalUsageSnapshot | null;
  isLoading: boolean;
  error: string | null;
};

const emptyState: LocalUsageState = {
  snapshot: null,
  isLoading: false,
  error: null,
};

const REFRESH_INTERVAL_MS = 5 * 60 * 1000;

export function useLocalUsage(enabled: boolean, workspacePath: string | null) {
  const runtimeEnabled = enabled && hasNativeBackendTransport();
  const [state, setState] = useState<LocalUsageState>(emptyState);
  const requestIdRef = useRef(0);
  const enabledRef = useRef(runtimeEnabled);
  const workspaceRef = useRef(workspacePath);

  useEffect(() => {
    enabledRef.current = runtimeEnabled;
    if (!runtimeEnabled) {
      setState((prev) => ({ ...prev, isLoading: false }));
    }
  }, [runtimeEnabled]);

  useEffect(() => {
    workspaceRef.current = workspacePath;
  }, [workspacePath]);

  const refresh = useCallback(() => {
    if (!enabledRef.current) {
      return Promise.resolve();
    }
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setState((prev) => ({ ...prev, isLoading: true, error: null }));
    return localUsageSnapshot(30, workspaceRef.current ?? undefined)
      .then((snapshot) => {
        if (requestIdRef.current !== requestId || !enabledRef.current) {
          return;
        }
        setState({ snapshot, isLoading: false, error: null });
      })
      .catch((err) => {
        if (requestIdRef.current !== requestId || !enabledRef.current) {
          return;
        }
        const message = err instanceof Error ? err.message : String(err);
        setState((prev) => ({ ...prev, isLoading: false, error: message }));
      });
  }, []);

  useEffect(() => {
    if (!runtimeEnabled) {
      return;
    }
    refresh()?.catch(() => {});
    const interval = window.setInterval(() => {
      refresh()?.catch(() => {});
    }, REFRESH_INTERVAL_MS);
    return () => {
      window.clearInterval(interval);
    };
  }, [refresh, runtimeEnabled, workspacePath]);

  return { ...state, refresh };
}
