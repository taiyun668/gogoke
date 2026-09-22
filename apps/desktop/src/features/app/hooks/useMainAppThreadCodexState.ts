import { useCallback, useMemo } from "react";
import { setWorkspaceRuntimeCodexArgs } from "@services/tauri";
import { buildCodexArgsOptions } from "@threads/utils/codexArgsProfiles";
import {
  resolveWorkspaceRuntimeCodexArgsBadgeLabel,
  resolveWorkspaceRuntimeCodexArgsOverride,
} from "@threads/utils/threadCodexParamsSeed";
import type { ThreadCodexParams } from "@threads/utils/threadStorage";

type ThreadCodexParamsPatch = Partial<
  Pick<
    ThreadCodexParams,
    | "modelId"
    | "effort"
    | "serviceTier"
    | "accessMode"
    | "collaborationModeId"
    | "codexArgsOverride"
  >
>;

type ThreadCodexMetadata = {
  modelId: string | null;
  effort: string | null;
};

type UseMainAppThreadCodexStateArgs = {
  appCodexArgs: string | null | undefined;
  selectedCodexArgsOverride: string | null;
  getThreadCodexParams: (
    workspaceId: string,
    threadId: string,
  ) => ThreadCodexParams | null;
  patchThreadCodexParams: (
    workspaceId: string,
    threadId: string,
    patch: ThreadCodexParamsPatch,
  ) => void;
};

export function useMainAppThreadCodexState({
  appCodexArgs,
  selectedCodexArgsOverride,
  getThreadCodexParams,
  patchThreadCodexParams,
}: UseMainAppThreadCodexStateArgs) {
  const handleThreadCodexMetadataDetected = useCallback(
    (workspaceId: string, threadId: string, metadata: ThreadCodexMetadata) => {
      if (!workspaceId || !threadId) {
        return;
      }

      const modelId =
        typeof metadata.modelId === "string" && metadata.modelId.trim().length > 0
          ? metadata.modelId.trim()
          : null;
      const effort =
        typeof metadata.effort === "string" && metadata.effort.trim().length > 0
          ? metadata.effort.trim().toLowerCase()
          : null;
      if (!modelId && !effort) {
        return;
      }

      const current = getThreadCodexParams(workspaceId, threadId);
      const patch: ThreadCodexParamsPatch = {};
      if (modelId && !current?.modelId) {
        patch.modelId = modelId;
      }
      if (effort && !current?.effort) {
        patch.effort = effort;
      }
      if (Object.keys(patch).length === 0) {
        return;
      }
      patchThreadCodexParams(workspaceId, threadId, patch);
    },
    [getThreadCodexParams, patchThreadCodexParams],
  );

  const codexArgsOptions = useMemo(
    () =>
      buildCodexArgsOptions({
        appCodexArgs: appCodexArgs ?? null,
        additionalCodexArgs: [selectedCodexArgsOverride],
      }),
    [appCodexArgs, selectedCodexArgsOverride],
  );

  const ensureWorkspaceRuntimeCodexArgs = useCallback(
    async (workspaceId: string, threadId: string | null) => {
      const sanitizedCodexArgsOverride = resolveWorkspaceRuntimeCodexArgsOverride({
        workspaceId,
        threadId,
        getThreadCodexParams,
      });
      await setWorkspaceRuntimeCodexArgs(workspaceId, sanitizedCodexArgsOverride);
    },
    [getThreadCodexParams],
  );

  const getThreadArgsBadge = useCallback(
    (workspaceId: string, threadId: string) =>
      resolveWorkspaceRuntimeCodexArgsBadgeLabel({
        workspaceId,
        threadId,
        getThreadCodexParams,
      }),
    [getThreadCodexParams],
  );

  return {
    handleThreadCodexMetadataDetected,
    codexArgsOptions,
    ensureWorkspaceRuntimeCodexArgs,
    getThreadArgsBadge,
  };
}
