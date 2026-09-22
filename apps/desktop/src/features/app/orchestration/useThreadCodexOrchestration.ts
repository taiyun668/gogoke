import { useCallback, useMemo, useRef, useState } from "react";
import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import type { AccessMode, ServiceTier } from "@/types";
import { useThreadCodexParams } from "@threads/hooks/useThreadCodexParams";
import {
  type PendingNewThreadSeed,
  NO_THREAD_SCOPE_SUFFIX,
} from "@threads/utils/threadCodexParamsSeed";

type ThreadCodexOrchestration = {
  accessMode: AccessMode;
  setAccessMode: Dispatch<SetStateAction<AccessMode>>;
  preferredModelId: string | null;
  setPreferredModelId: Dispatch<SetStateAction<string | null>>;
  preferredEffort: string | null;
  setPreferredEffort: Dispatch<SetStateAction<string | null>>;
  preferredServiceTier: ServiceTier | null | undefined;
  setPreferredServiceTier: Dispatch<SetStateAction<ServiceTier | null | undefined>>;
  preferredCollabModeId: string | null;
  setPreferredCollabModeId: Dispatch<SetStateAction<string | null>>;
  preferredCodexArgsOverride: string | null;
  setPreferredCodexArgsOverride: Dispatch<SetStateAction<string | null>>;
  threadCodexSelectionKey: string | null;
  setThreadCodexSelectionKey: Dispatch<SetStateAction<string | null>>;
  threadCodexParamsVersion: number;
  getThreadCodexParams: ReturnType<typeof useThreadCodexParams>["getThreadCodexParams"];
  patchThreadCodexParams: ReturnType<typeof useThreadCodexParams>["patchThreadCodexParams"];
  persistThreadCodexParams: (patch: {
    modelId?: string | null;
    effort?: string | null;
    serviceTier?: ServiceTier | null | undefined;
    accessMode?: AccessMode | null;
    collaborationModeId?: string | null;
    codexArgsOverride?: string | null;
  }) => void;
  activeThreadIdRef: MutableRefObject<string | null>;
  pendingNewThreadSeedRef: MutableRefObject<PendingNewThreadSeed | null>;
};

type UseThreadCodexOrchestrationParams = {
  activeWorkspaceIdForParamsRef: MutableRefObject<string | null>;
};

export function useThreadCodexOrchestration({
  activeWorkspaceIdForParamsRef,
}: UseThreadCodexOrchestrationParams): ThreadCodexOrchestration {
  const {
    version: threadCodexParamsVersion,
    getThreadCodexParams,
    patchThreadCodexParams,
  } = useThreadCodexParams();
  const [accessMode, setAccessMode] = useState<AccessMode>("current");
  const [preferredModelId, setPreferredModelId] = useState<string | null>(null);
  const [preferredEffort, setPreferredEffort] = useState<string | null>(null);
  const [preferredServiceTier, setPreferredServiceTier] = useState<
    ServiceTier | null | undefined
  >(undefined);
  const [preferredCollabModeId, setPreferredCollabModeId] = useState<string | null>(
    null,
  );
  const [preferredCodexArgsOverride, setPreferredCodexArgsOverride] = useState<string | null>(
    null,
  );
  const [threadCodexSelectionKey, setThreadCodexSelectionKey] = useState<string | null>(
    null,
  );
  const activeThreadIdRef = useRef<string | null>(null);
  const pendingNewThreadSeedRef = useRef<PendingNewThreadSeed | null>(null);

  const persistThreadCodexParams = useCallback(
    (patch: {
      modelId?: string | null;
      effort?: string | null;
      serviceTier?: ServiceTier | null | undefined;
      accessMode?: AccessMode | null;
      collaborationModeId?: string | null;
      codexArgsOverride?: string | null;
    }) => {
      const workspaceId = activeWorkspaceIdForParamsRef.current;
      const threadId = activeThreadIdRef.current ?? NO_THREAD_SCOPE_SUFFIX;
      if (!workspaceId) {
        return;
      }
      patchThreadCodexParams(workspaceId, threadId, patch);
      if (
        activeThreadIdRef.current &&
        Object.prototype.hasOwnProperty.call(patch, "serviceTier")
      ) {
        patchThreadCodexParams(workspaceId, NO_THREAD_SCOPE_SUFFIX, {
          serviceTier: patch.serviceTier,
        });
      }
    },
    [activeWorkspaceIdForParamsRef, patchThreadCodexParams],
  );

  return useMemo(
    () => ({
      accessMode,
      setAccessMode,
      preferredModelId,
      setPreferredModelId,
      preferredEffort,
      setPreferredEffort,
      preferredServiceTier,
      setPreferredServiceTier,
      preferredCollabModeId,
      setPreferredCollabModeId,
      preferredCodexArgsOverride,
      setPreferredCodexArgsOverride,
      threadCodexSelectionKey,
      setThreadCodexSelectionKey,
      threadCodexParamsVersion,
      getThreadCodexParams,
      patchThreadCodexParams,
      persistThreadCodexParams,
      activeThreadIdRef,
      pendingNewThreadSeedRef,
    }),
    [
      accessMode,
      preferredCollabModeId,
      preferredCodexArgsOverride,
      preferredEffort,
      preferredModelId,
      preferredServiceTier,
      threadCodexSelectionKey,
      threadCodexParamsVersion,
      setPreferredCodexArgsOverride,
      getThreadCodexParams,
      patchThreadCodexParams,
      persistThreadCodexParams,
    ],
  );
}
