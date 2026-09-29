import { useCallback, useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import type { DebugEntry } from "../../../types";
import {
  checkGogokeUpdate,
  installGogokeUpdate,
  takeGogokeUpdateFailure,
  type GogokeUpdateOffer,
} from "../../../services/tauri";
import {
  buildReleaseTagUrl,
  clearPendingPostUpdateVersion,
  fetchReleaseNotesForVersion,
  loadPendingPostUpdateVersion,
  normalizeReleaseVersion,
  savePendingPostUpdateVersion,
} from "../utils/postUpdateRelease";

type UpdateStage =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "restarting"
  | "latest"
  | "cleanup_pending"
  | "error";

type UpdateProgress = {
  totalBytes?: number;
  downloadedBytes: number;
};

export type UpdateState = {
  stage: UpdateStage;
  version?: string;
  progress?: UpdateProgress;
  message?: string;
  error?: string;
};

type PostUpdateNotice =
  | {
      stage: "loading";
      version: string;
      htmlUrl: string;
    }
  | {
      stage: "ready";
      version: string;
      body: string;
      htmlUrl: string;
    }
  | {
      stage: "fallback";
      version: string;
      htmlUrl: string;
    };

export type PostUpdateNoticeState = PostUpdateNotice | null;

type UseUpdaterOptions = {
  enabled?: boolean;
  autoCheckOnMount?: boolean;
  onDebug?: (entry: DebugEntry) => void;
};

export function useUpdater({
  enabled = true,
  autoCheckOnMount = true,
  onDebug,
}: UseUpdaterOptions) {
  const [state, setState] = useState<UpdateState>({ stage: "idle" });
  const [postUpdateNotice, setPostUpdateNotice] = useState<PostUpdateNoticeState>(
    null,
  );
  const updateRef = useRef<GogokeUpdateOffer | null>(null);
  const hasAttemptedAutoCheckRef = useRef(false);
  const postUpdateFetchGenerationRef = useRef(0);
  const latestTimeoutRef = useRef<number | null>(null);
  const latestToastDurationMs = 2000;

  const clearLatestTimeout = useCallback(() => {
    if (latestTimeoutRef.current !== null) {
      window.clearTimeout(latestTimeoutRef.current);
      latestTimeoutRef.current = null;
    }
  }, []);

  const resetToIdle = useCallback(async () => {
    clearLatestTimeout();
    const update = updateRef.current;
    updateRef.current = null;
    setState({ stage: "idle" });
    void update;
  }, [clearLatestTimeout]);

  const checkForUpdates = useCallback(async (options?: { announceNoUpdate?: boolean }) => {
    if (!enabled) {
      return;
    }
    let update: GogokeUpdateOffer | null = null;
    try {
      clearLatestTimeout();
      setState({ stage: "checking" });
      update = await checkGogokeUpdate();
      if (!update) {
        if (options?.announceNoUpdate) {
          setState({ stage: "latest" });
          latestTimeoutRef.current = window.setTimeout(() => {
            latestTimeoutRef.current = null;
            setState({ stage: "idle" });
          }, latestToastDurationMs);
        } else {
          setState({ stage: "idle" });
        }
        return;
      }

      updateRef.current = update;
      setState({
        stage: "available",
        version: update.version,
      });
    } catch (error) {
      const message =
        error instanceof Error ? error.message : JSON.stringify(error);
      onDebug?.({
        id: `${Date.now()}-client-updater-error`,
        timestamp: Date.now(),
        source: "error",
        label: "updater/error",
        payload: message,
      });
      setState({ stage: "error", error: message });
    } finally {
      void update;
    }
  }, [clearLatestTimeout, enabled, onDebug]);

  const startUpdate = useCallback(async () => {
    if (!enabled) {
      return;
    }
    const update = updateRef.current;
    if (!update) {
      await checkForUpdates();
      return;
    }

    setState((prev) => ({
      ...prev,
      stage: "installing",
      progress: undefined,
      error: undefined,
    }));

    try {
      savePendingPostUpdateVersion(update.version);
      await installGogokeUpdate(update.version);
      setState((prev) => ({
        ...prev,
        stage: "restarting",
      }));
    } catch (error) {
      const message =
        error instanceof Error ? error.message : JSON.stringify(error);
      onDebug?.({
        id: `${Date.now()}-client-updater-error`,
        timestamp: Date.now(),
        source: "error",
        label: "updater/error",
        payload: message,
      });
      setState((prev) => ({
        ...prev,
        stage: "error",
        error: message,
      }));
    }
  }, [checkForUpdates, enabled, onDebug]);

  useEffect(() => {
    if (!enabled || import.meta.env.DEV || !isTauri()) {
      return;
    }
    if (hasAttemptedAutoCheckRef.current) {
      return;
    }
    hasAttemptedAutoCheckRef.current = true;
    void (async () => {
      try {
        const notice = await takeGogokeUpdateFailure();
        if (notice?.kind === "cleanup_pending") {
          setState({ stage: "cleanup_pending", message: notice.message });
          return;
        }
        if (notice?.kind === "failure") {
          setState({
            stage: "error",
            error: notice.message,
          });
          return;
        }
        if (autoCheckOnMount) {
          await checkForUpdates();
        }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setState({ stage: "error", error: message });
      }
    })();
  }, [autoCheckOnMount, checkForUpdates, enabled]);

  useEffect(() => {
    if (!enabled || !isTauri()) {
      return;
    }
    const pendingVersion = loadPendingPostUpdateVersion();
    if (!pendingVersion) {
      return;
    }

    const normalizedPendingVersion = normalizeReleaseVersion(pendingVersion);
    const normalizedCurrentVersion = normalizeReleaseVersion(__APP_VERSION__);
    if (
      !normalizedPendingVersion ||
      normalizedPendingVersion !== normalizedCurrentVersion
    ) {
      clearPendingPostUpdateVersion();
      return;
    }

    const fallbackUrl = buildReleaseTagUrl(normalizedPendingVersion);
    const generation = postUpdateFetchGenerationRef.current + 1;
    postUpdateFetchGenerationRef.current = generation;
    let cancelled = false;
    setPostUpdateNotice({
      stage: "loading",
      version: normalizedPendingVersion,
      htmlUrl: fallbackUrl,
    });

    void fetchReleaseNotesForVersion(normalizedPendingVersion)
      .then((releaseInfo) => {
        if (
          cancelled ||
          postUpdateFetchGenerationRef.current !== generation
        ) {
          return;
        }
        if (releaseInfo.body) {
          setPostUpdateNotice({
            stage: "ready",
            version: normalizedPendingVersion,
            body: releaseInfo.body,
            htmlUrl: releaseInfo.htmlUrl,
          });
          return;
        }
        setPostUpdateNotice({
          stage: "fallback",
          version: normalizedPendingVersion,
          htmlUrl: releaseInfo.htmlUrl,
        });
      })
      .catch((error) => {
        if (
          cancelled ||
          postUpdateFetchGenerationRef.current !== generation
        ) {
          return;
        }
        const message =
          error instanceof Error ? error.message : JSON.stringify(error);
        onDebug?.({
          id: `${Date.now()}-client-updater-release-notes-error`,
          timestamp: Date.now(),
          source: "error",
          label: "updater/release-notes-error",
          payload: message,
        });
        setPostUpdateNotice({
          stage: "fallback",
          version: normalizedPendingVersion,
          htmlUrl: fallbackUrl,
        });
      });

    return () => {
      cancelled = true;
    };
  }, [enabled, onDebug]);

  useEffect(() => {
    return () => {
      clearLatestTimeout();
    };
  }, [clearLatestTimeout]);

  const dismissPostUpdateNotice = useCallback(() => {
    postUpdateFetchGenerationRef.current += 1;
    clearPendingPostUpdateVersion();
    setPostUpdateNotice(null);
  }, []);

  return {
    state,
    startUpdate,
    checkForUpdates,
    dismiss: resetToIdle,
    postUpdateNotice,
    dismissPostUpdateNotice,
  };
}
