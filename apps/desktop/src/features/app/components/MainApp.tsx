import { lazy, useCallback, useEffect, useMemo, useRef, useState, type ComponentProps } from "react";
import type { Messages } from "@/features/messages/components/Messages";
import type { Composer } from "@/features/composer/components/Composer";
import { ComposerInput } from "@/features/composer/components/ComposerInput";
import successSoundUrl from "@/assets/success-notification.mp3";
import errorSoundUrl from "@/assets/error-notification.mp3";
import { MainAppShell } from "@app/components/MainAppShell";
import { useThreads } from "@threads/hooks/useThreads";
import { usePullRequestComposer } from "@/features/git/hooks/usePullRequestComposer";
import { useAutoExitEmptyDiff } from "@/features/git/hooks/useAutoExitEmptyDiff";
import { isMissingRepo } from "@/features/git/utils/repoErrors";
import { useModels } from "@/features/models/hooks/useModels";
import { useCollaborationModes } from "@/features/collaboration/hooks/useCollaborationModes";
import { useCollaborationModeSelection } from "@/features/collaboration/hooks/useCollaborationModeSelection";
import { useSkills } from "@/features/skills/hooks/useSkills";
import { useApps } from "@/features/apps/hooks/useApps";
import { useCustomPrompts } from "@/features/prompts/hooks/useCustomPrompts";
import { useBranchSwitcherShortcut } from "@/features/git/hooks/useBranchSwitcherShortcut";
import { useRenameWorktreePrompt } from "@/features/workspaces/hooks/useRenameWorktreePrompt";
import { useLayoutController } from "@app/hooks/useLayoutController";
import { useUpdaterController } from "@app/hooks/useUpdaterController";
import { useResponseRequiredNotificationsController } from "@app/hooks/useResponseRequiredNotificationsController";
import { useErrorToasts } from "@/features/notifications/hooks/useErrorToasts";
import { useComposerShortcuts } from "@/features/composer/hooks/useComposerShortcuts";
import { useComposerMenuActions } from "@/features/composer/hooks/useComposerMenuActions";
import { useComposerEditorState } from "@/features/composer/hooks/useComposerEditorState";
import { useMainAppComposerWorkspaceState } from "@app/hooks/useMainAppComposerWorkspaceState";
import { useMainAppGitState } from "@app/hooks/useMainAppGitState";
import { useMainAppLayoutSurfaces } from "@app/hooks/useMainAppLayoutSurfaces";
import { useMainAppLayoutNodes } from "@app/hooks/useMainAppLayoutNodes";
import { useWorkspaceFromUrlPrompt } from "@/features/workspaces/hooks/useWorkspaceFromUrlPrompt";
import { useWorkspaceController } from "@app/hooks/useWorkspaceController";
import { useWorkspaceSelection } from "@/features/workspaces/hooks/useWorkspaceSelection";
import { usePlanReadyActions } from "@app/hooks/usePlanReadyActions";
import { useThreadRows } from "@app/hooks/useThreadRows";
import { useInterruptShortcut } from "@app/hooks/useInterruptShortcut";
import { useArchiveShortcut } from "@app/hooks/useArchiveShortcut";
import { useCopyThread } from "@threads/hooks/useCopyThread";
import { useTerminalController } from "@/features/terminal/hooks/useTerminalController";
import { useWorkspaceLaunchScript } from "@app/hooks/useWorkspaceLaunchScript";
import { useWorkspaceLaunchScripts } from "@app/hooks/useWorkspaceLaunchScripts";
import { useWorktreeSetupScript } from "@app/hooks/useWorktreeSetupScript";
import { effectiveCommitMessageModelId } from "@/features/git/utils/commitMessageModelSelection";
import { useMobileServerSetup } from "@/features/mobile/hooks/useMobileServerSetup";
import { useMainAppModals } from "@app/hooks/useMainAppModals";
import { useMainAppDisplayNodes } from "@app/hooks/useMainAppDisplayNodes";
import { useMainAppPromptActions } from "@app/hooks/useMainAppPromptActions";
import { useMainAppShellProps } from "@app/hooks/useMainAppShellProps";
import { useMainAppSidebarMenuOrchestration } from "@app/hooks/useMainAppSidebarMenuOrchestration";
import { useMainAppSettingsActions } from "@app/hooks/useMainAppSettingsActions";
import { useMainAppThreadCodexState } from "@app/hooks/useMainAppThreadCodexState";
import { useMainAppWorktreeState } from "@app/hooks/useMainAppWorktreeState";
import { useMainAppWorkspaceActions } from "@app/hooks/useMainAppWorkspaceActions";
import { useMainAppWorkspaceLifecycle } from "@app/hooks/useMainAppWorkspaceLifecycle";
import { useMainAppMobileThreadRefresh } from "@app/hooks/useMainAppMobileThreadRefresh";
import { useHomeAccount } from "@app/hooks/useHomeAccount";
import type {
  ComposerEditorSettings,
  ServiceTier,
  WorkspaceInfo,
} from "@/types";
import { useOpenAppIcons } from "@app/hooks/useOpenAppIcons";
import { useAccountSwitching } from "@app/hooks/useAccountSwitching";
import { useNewAgentDraft } from "@app/hooks/useNewAgentDraft";
import { useSystemNotificationThreadLinks } from "@app/hooks/useSystemNotificationThreadLinks";
import { useThreadListSortKey } from "@app/hooks/useThreadListSortKey";
import { useThreadListActions } from "@app/hooks/useThreadListActions";
import { useRemoteThreadLiveConnection } from "@app/hooks/useRemoteThreadLiveConnection";
import { useTrayRecentThreads } from "@app/hooks/useTrayRecentThreads";
import { useTraySessionUsage } from "@app/hooks/useTraySessionUsage";
import { useTauriEvent } from "@app/hooks/useTauriEvent";
import { useAppBootstrapOrchestration } from "@app/bootstrap/useAppBootstrapOrchestration";
import {
  useThreadCodexBootstrapOrchestration,
  useThreadCodexSyncOrchestration,
  useThreadSelectionHandlersOrchestration,
  useThreadUiOrchestration,
} from "@app/orchestration/useThreadOrchestration";
import {
  useWorkspaceInsightsOrchestration,
  useWorkspaceOrderingOrchestration,
} from "@app/orchestration/useWorkspaceOrchestration";
import { useAppShellOrchestration } from "@app/orchestration/useLayoutOrchestration";
import { normalizeCodexArgsInput } from "@/utils/codexArgsInput";
import { subscribeTrayOpenThread } from "@services/events";
import { I18nProvider } from "@/i18n";
import { hasNativeBackendTransport } from "@/platform/runtime";
import { createDesign37SecretarySource, sameSecretaryWriter, signalGogokeUpdateReady,
  pendingNativeVisibleIntent, recoverPendingNativeVisibleIntent,
  type SecretaryBinding, type SecretaryOriginalInput, type SecretaryVendorUserFact,
  type SecretaryWriteFact } from "@/services/tauri";
import { NowProvider, NowPinSlot, type NowSource } from "@/features/now/NowContext";
import { SecretaryEntry, SecretaryPanel, SecretaryActionLine, type SecretarySource } from "@/features/secretary/Secretary";
import { entryLine, type EntryState, type ActionLine } from "@/features/secretary/secretaryModel";

/** Complete host read, with explicit native identity and separate UI/output anchors. */
export type SecretaryView = {
  association: {
    workspaceId: string | null;
    threadId: string;
    domainId: string;
    seatId: string;
    sessionId: string;
  } | null;
  entry: EntryState;
  source: SecretarySource;
  readState: "known" | "frozen";
  readAt: string;
  readError?: string;
  actionLines: Array<{ turnId: string; itemId: string; line: ActionLine }>;
  /** Native host data/actions for the ordinary renderers; never inherited from the active project. */
  conversation: {
    messages: Omit<ComponentProps<typeof Messages>, "afterItem">;
    composer: ComponentProps<typeof Composer> | null;
  } | null;
  nativeInput?: {
    instanceId: string; model: string; effort: string; permissionTier: string;
    canSend: boolean; canStop: boolean;
    binding: SecretaryBinding;
    send: (binding: SecretaryBinding, body: string) => Promise<SecretaryWriteFact>;
    stop: (binding: SecretaryBinding) => Promise<SecretaryWriteFact>;
  } | null;
  outbound?: SecretaryWriteFact[];
  historyGap?: string | null;
  statuses?: string[];
  originalInputs?: SecretaryOriginalInput[];
  vendorUserFacts?: SecretaryVendorUserFact[];
  inputRowsEnded?: boolean;
  verifiedSend?: { requestId: string; body: string; hGeneration: string } | null;
  retryStop?: (requestId: string) => Promise<SecretaryWriteFact>;
  retrySend?: (requestId: string) => Promise<SecretaryWriteFact>;
  openConversation?: () => Promise<void>;
  openAction?: (id: string) => void;
};

const secretaryAssociationKey = (view: SecretaryView | null) => view?.association
  ? JSON.stringify([view.association.workspaceId, view.association.threadId,
      view.association.domainId, view.association.seatId, view.association.sessionId]) : null;

const SettingsView = lazy(() =>
  import("@settings/components/SettingsView").then((module) => ({
    default: module.SettingsView,
  })),
);

/** The native singleton is independent of the selected project and its hooks. */
function useNativeSecretaryView(enabled: boolean): SecretaryView | null {
  const producer = useMemo(() => enabled ? createDesign37SecretarySource() : null, [enabled]);
  const [view, setView] = useState<SecretaryView | null>(null);
  useEffect(() => {
    setView(null);
    if (!producer) return;
    let disposed = false;
    let pending: Promise<void> | null = null;
    const refresh = (): Promise<void> => {
      if (pending) return pending;
      pending = (async () => {
        try {
          const snapshot = await producer.readSnapshot();
          const conversation = await producer.readConversation(snapshot.configuration);
          if (disposed) return;
          setView({
            association: conversation ? { workspaceId: null, threadId: conversation.threadId,
              domainId: "global", seatId: conversation.seatId,
              sessionId: conversation.sessionId } : null,
            entry: snapshot.page.entry,
            source: producer.source,
            readState: "known",
            readAt: new Date().toLocaleString(),
            actionLines: [],
            conversation: conversation ? {
              messages: { items: conversation.messages, threadId: conversation.threadId,
                workspaceId: null, workspacePath: null,
                isThinking: conversation.turnState === "RUNNING",
                openTargets: [], selectedOpenAppId: "" },
              composer: null,
            } : null,
            nativeInput: conversation?.writer ? { ...conversation.writer,
              send: producer.send, stop: producer.stop } : null,
            outbound: producer.writeFacts(),
            historyGap: conversation?.historyGap ?? null,
            statuses: conversation?.statuses ?? [],
            originalInputs: conversation?.inputs ?? [],
            vendorUserFacts: conversation?.vendorUserFacts ?? [],
            inputRowsEnded: conversation?.inputRowsEnded ?? false,
            verifiedSend: conversation?.verifiedSend ?? null,
            retryStop: producer.retryStop,
            retrySend: producer.retrySend,
            openConversation: refresh,
          });
        } catch (cause) {
          if (disposed) return;
          const readError = cause instanceof Error ? cause.message : String(cause);
          producer.invalidateTranscript();
          setView((previous) => previous
            ? { ...previous, readState: "frozen", readError,
                outbound: producer.writeFacts(), nativeInput: null }
            : { association: null, conversation: null,
                entry: { kind: "down", reason: readError }, source: producer.source,
                readState: "frozen", readAt: "尚未读到", readError, actionLines: [] });
        } finally {
          pending = null;
        }
      })();
      return pending;
    };
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 2_000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [producer]);
  return view;
}

export default function MainApp({ nowSource = null, secretaryView }: {
  nowSource?: NowSource | null;
  secretaryView?: SecretaryView | null;
} = {}) {
  const bootstrap = useAppBootstrapOrchestration();
  const nativeSecretary = useNativeSecretaryView(secretaryView === undefined && hasNativeBackendTransport());
  const selectedSecretaryView = secretaryView === undefined ? nativeSecretary : secretaryView;
  useEffect(() => {
    if (hasNativeBackendTransport()) {
      void signalGogokeUpdateReady().catch((error) => {
        console.warn("Failed to signal gogoke update readiness", error);
      });
    }
  }, []);
  return (
    <I18nProvider language={bootstrap.appSettings.appLanguage}>
      <MainAppContent bootstrap={bootstrap} nowSource={nowSource} secretaryView={selectedSecretaryView} />
    </I18nProvider>
  );
}

function MainAppContent({
  bootstrap,
  nowSource,
  secretaryView,
}: {
  bootstrap: ReturnType<typeof useAppBootstrapOrchestration>;
  nowSource: NowSource | null;
  secretaryView: SecretaryView | null;
}) {
  const secretaryKey = secretaryAssociationKey(secretaryView);
  const secretaryToken = useMemo(() => ({}), [secretaryView?.source, secretaryKey]);
  const [secretaryDrafts, setSecretaryDrafts] = useState<Record<string, {
    text: string; revision: number }>>({});
  const secretaryDraftState = secretaryKey ? secretaryDrafts[secretaryKey] : undefined;
  const secretaryDraft = secretaryDraftState?.text ?? "";
  const secretarySubmittedDrafts = useRef(new Map<string, {
    text: string; revision: number; binding: SecretaryBinding;
    requestId: string | null; earlierRequestIds: Set<string>;
  }>());
  useEffect(() => {
    const verified = secretaryView?.verifiedSend;
    if (!verified) return;
    const submitted = secretaryKey ? secretarySubmittedDrafts.current.get(secretaryKey) : undefined;
    const original = secretaryView?.outbound?.find((fact) => fact.operation === "send" &&
      fact.requestId === verified.requestId && fact.hGeneration === verified.hGeneration &&
      fact.inputVerified && fact.status === "ACCEPTED");
    if (!submitted || !original || submitted.text !== verified.body ||
        original.body !== verified.body || !sameSecretaryWriter(submitted.binding, original.binding) ||
        (submitted.requestId !== null ? submitted.requestId !== verified.requestId
          : submitted.earlierRequestIds.has(verified.requestId))) return;
    setSecretaryDrafts((previous) => {
      if (!secretaryKey) return previous;
      const draft = previous[secretaryKey];
      return draft?.revision === submitted.revision && draft.text === verified.body
        ? { ...previous, [secretaryKey]: { ...draft, text: "" } } : previous;
    });
    if (secretaryKey) secretarySubmittedDrafts.current.delete(secretaryKey);
  }, [secretaryKey, secretaryView?.verifiedSend?.requestId,
    secretaryView?.verifiedSend?.body, secretaryView?.verifiedSend?.hGeneration]);
  const [secretaryWritePending, setSecretaryWritePending] = useState(false);
  const secretaryWritePendingRef = useRef(false);
  const [secretaryWriteFailure, setSecretaryWriteFailure] = useState<{
    token: object; operation: "send" | "stop"; sessionId: string | null;
    body: string | null; text: string } | null>(null);
  const relevantSecretarySends = secretaryView?.outbound?.filter((fact) =>
    fact.operation === "send" && secretaryView.nativeInput &&
    sameSecretaryWriter(fact.binding, secretaryView.nativeInput.binding)) ?? [];
  const secretarySendBlocked = relevantSecretarySends.some((fact) => fact.status === "UNKNOWN") ||
    relevantSecretarySends.some((fact) => fact.status === "ACCEPTED" &&
      !fact.inputVerified && fact.body === secretaryDraft);
  const secretaryTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const currentSecretary = useRef<{ view: SecretaryView | null; token: object | null }>({ view: secretaryView, token: secretaryToken });
  currentSecretary.current = { view: secretaryView, token: secretaryToken };
  const [secretaryOpening, setSecretaryOpening] = useState<object | null>(null);
  const secretaryOpeningRef = useRef<object | null>(null);
  const [selectedSecretary, setSelectedSecretary] = useState<object | null>(null);
  const secretaryNavigation = useRef(0);
  const secretaryMounted = useRef(true);
  const leaveSecretary = useCallback(() => { secretaryNavigation.current += 1; setSelectedSecretary(null); }, []);
  const [secretaryOpenError, setSecretaryOpenError] = useState<{ token: object; text: string } | null>(null);
  const secretaryPanelSource = useMemo<SecretarySource | null>(() => {
    const source = secretaryView?.source;
    if (!source) return null;
    return {
      read: source.read,
      get actions() {
        const current = currentSecretary.current.view;
        return current?.source === source && current.readState === "known" ? source.actions : {};
      },
    };
  }, [secretaryView?.source]);
  useEffect(() => {
    secretaryMounted.current = true;
    return () => { secretaryMounted.current = false; };
  }, []);
  const {
    appSettings,
    setAppSettings,
    doctor,
    codexUpdate,
    appSettingsLoading,
    reduceTransparency,
    setReduceTransparency,
    scaleShortcutTitle,
    scaleShortcutText,
    queueSaveSettings,
    dictationModel,
    dictationState,
    dictationLevel,
    dictationTranscript,
    dictationError,
    dictationHint,
    dictationReady,
    handleToggleDictation,
    cancelDictation,
    clearDictationTranscript,
    clearDictationError,
    clearDictationHint,
    debugOpen,
    setDebugOpen,
    debugEntries,
    showDebugButton,
    addDebugEntry,
    handleCopyDebug,
    clearDebugEntries,
    shouldReduceTransparency,
  } = bootstrap;
  const {
    threadListSortKey,
    setThreadListSortKey,
    threadListOrganizeMode,
    setThreadListOrganizeMode,
  } = useThreadListSortKey();
  const [activeTab, setActiveTab] = useState<
    "home" | "projects" | "codex" | "git" | "log"
  >("codex");
  const tabletTab =
    activeTab === "projects" || activeTab === "home" ? "codex" : activeTab;
  const {
    workspaces,
    workspaceGroups,
    groupedWorkspaces,
    getWorkspaceGroupName,
    ungroupedLabel,
    activeWorkspace,
    activeWorkspaceId,
    setActiveWorkspaceId,
    addWorkspace,
    addWorkspaceFromPath,
    addWorkspaceFromGitUrl,
    addWorkspacesFromPaths,
    mobileRemoteWorkspacePathPrompt,
    updateMobileRemoteWorkspacePathInput,
    appendMobileRemoteWorkspacePathFromRecent,
    cancelMobileRemoteWorkspacePathPrompt,
    submitMobileRemoteWorkspacePathPrompt,
    addCloneAgent,
    addWorktreeAgent,
    connectWorkspace,
    markWorkspaceConnected,
    updateWorkspaceSettings,
    createWorkspaceGroup,
    renameWorkspaceGroup,
    moveWorkspaceGroup,
    deleteWorkspaceGroup,
    assignWorkspaceGroup,
    removeWorkspace,
    removeWorktree,
    renameWorktree,
    renameWorktreeUpstream,
    deletingWorktreeIds,
    hasLoaded,
    refreshWorkspaces,
  } = useWorkspaceController({
    appSettings,
    addDebugEntry,
    queueSaveSettings,
  });
  const {
    isMobileRuntime,
    showMobileSetupWizard,
    mobileSetupWizardProps,
    handleMobileConnectSuccess,
  } = useMobileServerSetup({
    appSettings,
    appSettingsLoading,
    queueSaveSettings,
    refreshWorkspaces,
  });
  const updaterEnabled = !isMobileRuntime;

  const workspacesById = useMemo(
    () => new Map(workspaces.map((workspace) => [workspace.id, workspace])),
    [workspaces],
  );
  const {
    threadCodexParamsVersion,
    getThreadCodexParams,
    patchThreadCodexParams,
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
    activeThreadIdRef,
    pendingNewThreadSeedRef,
    persistThreadCodexParams,
  } = useThreadCodexBootstrapOrchestration({
    activeWorkspaceId,
  });
  const {
    appRef,
    isResizing,
    sidebarWidth,
    chatDiffSplitPositionPercent,
    rightPanelWidth,
    onSidebarResizeStart,
    onChatDiffSplitPositionResizeStart,
    onRightPanelResizeStart,
    planPanelHeight,
    onPlanPanelResizeStart,
    terminalPanelHeight,
    onTerminalPanelResizeStart,
    debugPanelHeight,
    onDebugPanelResizeStart,
    isCompact,
    isTablet,
    isPhone,
    sidebarCollapsed,
    rightPanelCollapsed,
    collapseSidebar,
    expandSidebar,
    collapseRightPanel,
    expandRightPanel,
    terminalOpen,
    handleDebugClick,
    handleToggleTerminal,
    openTerminal,
    closeTerminal: closeTerminalPanel,
  } = useLayoutController({
    activeWorkspaceId,
    setActiveTab,
    setDebugOpen,
    toggleDebugPanelShortcut: appSettings.toggleDebugPanelShortcut,
    toggleTerminalShortcut: appSettings.toggleTerminalShortcut,
  });
  const sidebarToggleProps = {
    isCompact,
    sidebarCollapsed,
    rightPanelCollapsed,
    onCollapseSidebar: collapseSidebar,
    onExpandSidebar: expandSidebar,
    onCollapseRightPanel: collapseRightPanel,
    onExpandRightPanel: expandRightPanel,
  };
  const composerInputRef = useRef<HTMLTextAreaElement | null>(null);
  const workspaceHomeTextareaRef = useRef<HTMLTextAreaElement | null>(null);

  const getWorkspaceName = useCallback(
    (workspaceId: string) => workspacesById.get(workspaceId)?.name,
    [workspacesById],
  );

  const recordPendingThreadLinkRef = useRef<
    (workspaceId: string, threadId: string) => void
  >(() => {});

  const { errorToasts, dismissErrorToast } = useErrorToasts();
  const queueGitStatusRefreshRef = useRef<() => void>(() => {});
  const handleThreadMessageActivity = useCallback(() => {
    queueGitStatusRefreshRef.current();
  }, []);

  // Access mode is thread-scoped (best-effort persisted) and falls back to the app default.

  const {
    models,
    selectedModel,
    selectedModelId,
    setSelectedModelId,
    reasoningSupported,
    reasoningOptions,
    selectedEffort,
    setSelectedEffort
  } = useModels({
    activeWorkspace,
    onDebug: addDebugEntry,
    preferredModelId,
    preferredEffort,
    selectionKey: threadCodexSelectionKey,
  });

  const {
    collaborationModes,
    selectedCollaborationMode,
    selectedCollaborationModeId,
    setSelectedCollaborationModeId,
  } = useCollaborationModes({
    activeWorkspace,
    enabled: appSettings.collaborationModesEnabled,
    preferredModeId: preferredCollabModeId,
    selectionKey: threadCodexSelectionKey,
    onDebug: addDebugEntry,
  });

  const [selectedCodexArgsOverride, setSelectedCodexArgsOverride] = useState<string | null>(
    null,
  );
  const [selectedServiceTier, setSelectedServiceTier] = useState<
    ServiceTier | null | undefined
  >(undefined);
  useEffect(() => {
    setSelectedCodexArgsOverride(normalizeCodexArgsInput(preferredCodexArgsOverride));
  }, [preferredCodexArgsOverride, threadCodexSelectionKey]);
  useEffect(() => {
    setSelectedServiceTier(preferredServiceTier);
  }, [preferredServiceTier, threadCodexSelectionKey]);

  const {
    handleSelectModel,
    handleSelectEffort,
    handleSelectServiceTier,
    handleSelectCollaborationMode,
    handleSelectAccessMode,
    handleSelectCodexArgsOverride,
  } = useThreadSelectionHandlersOrchestration({
    appSettingsLoading,
    setAppSettings,
    queueSaveSettings,
    activeThreadIdRef,
    setSelectedModelId,
    setSelectedEffort,
    setSelectedServiceTier,
    setSelectedCollaborationModeId,
    setAccessMode,
    setSelectedCodexArgsOverride,
    persistThreadCodexParams,
  });
  const commitMessageModelId = useMemo(
    () => effectiveCommitMessageModelId(models, appSettings.commitMessageModelId),
    [models, appSettings.commitMessageModelId],
  );

  const composerShortcuts = {
    modelShortcut: appSettings.composerModelShortcut,
    accessShortcut: appSettings.composerAccessShortcut,
    reasoningShortcut: appSettings.composerReasoningShortcut,
    collaborationShortcut: appSettings.collaborationModesEnabled
      ? appSettings.composerCollaborationShortcut
      : null,
    models,
    collaborationModes,
    selectedModelId,
    onSelectModel: handleSelectModel,
    selectedCollaborationModeId,
    onSelectCollaborationMode: handleSelectCollaborationMode,
    accessMode,
    onSelectAccessMode: handleSelectAccessMode,
    reasoningOptions,
    selectedEffort,
    onSelectEffort: handleSelectEffort,
    selectedServiceTier: selectedServiceTier ?? null,
    reasoningSupported,
  };

  useComposerShortcuts({
    textareaRef: composerInputRef,
    ...composerShortcuts,
  });

  useComposerShortcuts({
    textareaRef: workspaceHomeTextareaRef,
    ...composerShortcuts,
  });

  useComposerMenuActions({
    models,
    selectedModelId,
    onSelectModel: handleSelectModel,
    collaborationModes,
    selectedCollaborationModeId,
    onSelectCollaborationMode: handleSelectCollaborationMode,
    accessMode,
    onSelectAccessMode: handleSelectAccessMode,
    reasoningOptions,
    selectedEffort,
    onSelectEffort: handleSelectEffort,
    reasoningSupported,
    onFocusComposer: () => composerInputRef.current?.focus(),
  });
  const { skills } = useSkills({ activeWorkspace, onDebug: addDebugEntry });
  const {
    prompts,
    createPrompt,
    updatePrompt,
    deletePrompt,
    movePrompt,
    getWorkspacePromptsDir,
    getGlobalPromptsDir,
  } = useCustomPrompts({ activeWorkspace, onDebug: addDebugEntry });
  const resolvedModel = selectedModel?.model ?? null;
  const resolvedEffort = reasoningSupported ? selectedEffort : null;

  const {
    handleThreadCodexMetadataDetected,
    codexArgsOptions,
    ensureWorkspaceRuntimeCodexArgs,
    getThreadArgsBadge,
  } = useMainAppThreadCodexState({
    appCodexArgs: appSettings.codexArgs,
    selectedCodexArgsOverride,
    getThreadCodexParams,
    patchThreadCodexParams,
  });

  const { collaborationModePayload } = useCollaborationModeSelection({
    selectedCollaborationMode,
    selectedCollaborationModeId,
    selectedEffort: resolvedEffort,
    resolvedModel,
  });

  const {
    setActiveThreadId,
    hasLocalThreadSnapshot,
    activeThreadId,
    activeItems,
    approvals,
    userInputRequests,
    threadsByWorkspace,
    threadParentById,
    isSubagentThread,
    threadStatusById,
    threadResumeLoadingById,
    threadListLoadingByWorkspace,
    threadListPagingByWorkspace,
    threadListCursorByWorkspace,
    activeTurnIdByThread,
    tokenUsageByThread,
    rateLimitsByWorkspace,
    accountByWorkspace,
    planByThread,
    lastAgentMessageByThread,
    pinnedThreadsVersion,
    interruptTurn,
    removeThread,
    pinThread,
    unpinThread,
    isThreadPinned,
    getPinTimestamp,
    renameThread,
    startThreadForWorkspace,
    listThreadsForWorkspaces,
    listThreadsForWorkspace,
    loadOlderThreadsForWorkspace,
    resetWorkspaceThreads,
    refreshThread,
    sendUserMessage,
    sendUserMessageToThread,
    startFork,
    startReview,
    startUncommittedReview,
    startResume,
    startCompact,
    startApps,
    startMcp,
    startFast,
    startStatus,
    reviewPrompt,
    closeReviewPrompt,
    showPresetStep,
    choosePreset,
    highlightedPresetIndex,
    setHighlightedPresetIndex,
    highlightedBranchIndex,
    setHighlightedBranchIndex,
    highlightedCommitIndex,
    setHighlightedCommitIndex,
    handleReviewPromptKeyDown,
    confirmBranch,
    selectBranch,
    selectBranchAtIndex,
    selectCommit,
    selectCommitAtIndex,
    confirmCommit,
    updateCustomInstructions,
    confirmCustom,
    handleApprovalDecision,
    handleApprovalRemember,
    handleUserInputSubmit,
    refreshAccountInfo,
    refreshAccountRateLimits,
  } = useThreads({
    activeWorkspace,
    onWorkspaceConnected: markWorkspaceConnected,
    onDebug: addDebugEntry,
    model: resolvedModel,
    effort: resolvedEffort,
    serviceTier: selectedServiceTier,
    collaborationMode: collaborationModePayload,
    onSelectServiceTier: handleSelectServiceTier,
    accessMode,
    ensureWorkspaceRuntimeCodexArgs,
    reviewDeliveryMode: appSettings.reviewDeliveryMode,
    steerEnabled: appSettings.steerEnabled,
    threadTitleAutogenerationEnabled: appSettings.threadTitleAutogenerationEnabled,
    chatHistoryScrollbackItems: appSettingsLoading
      ? null
      : appSettings.chatHistoryScrollbackItems,
    customPrompts: prompts,
    onMessageActivity: handleThreadMessageActivity,
    threadSortKey: threadListSortKey,
    onThreadCodexMetadataDetected: handleThreadCodexMetadataDetected,
  });
  const { connectionState: remoteThreadConnectionState, reconnectLive } =
    useRemoteThreadLiveConnection({
      backendMode: appSettings.backendMode,
      activeWorkspace,
      activeThreadId,
      activeThreadHasLocalSnapshot: hasLocalThreadSnapshot(activeThreadId),
      activeThreadIsProcessing: Boolean(
        activeThreadId && threadStatusById[activeThreadId]?.isProcessing,
      ),
      refreshThread,
      reconnectWorkspace: connectWorkspace,
    });

  // Reopening or reconnecting reads the retained original request. It never
  // repeats a write or creates a replacement intent after an unknown result.
  useEffect(() => {
    if (!activeWorkspaceId || !hasNativeBackendTransport()) return;
    let active = true;
    let reading = false;
    const recoverOriginal = async () => {
      if (!active || reading) return;
      reading = true;
      try {
        const pending = pendingNativeVisibleIntent(activeWorkspaceId);
        if (!pending) return;
        await recoverPendingNativeVisibleIntent(activeWorkspaceId);
      } catch (cause) {
        if (active) addDebugEntry({ id: `${Date.now()}-native-original-recovery`,
          timestamp: Date.now(), source: "error",
          label: "Original native conversation request recovery",
          payload: cause instanceof Error ? cause.message : String(cause) });
      } finally { reading = false; }
    };
    void recoverOriginal();
    window.addEventListener("focus", recoverOriginal);
    window.addEventListener("online", recoverOriginal);
    return () => {
      active = false;
      window.removeEventListener("focus", recoverOriginal);
      window.removeEventListener("online", recoverOriginal);
    };
  }, [activeWorkspaceId, activeWorkspace?.connected,
    remoteThreadConnectionState, addDebugEntry]);

  const { mobileThreadRefreshLoading, handleMobileThreadRefresh } =
    useMainAppMobileThreadRefresh({
      activeWorkspace,
      activeThreadId,
      startThreadForWorkspace,
      refreshThread,
      reconnectLive,
    });
  const {
    updaterState,
    startUpdate,
    dismissUpdate,
    postUpdateNotice,
    dismissPostUpdateNotice,
    handleTestNotificationSound,
    handleTestSystemNotification,
  } = useUpdaterController({
    enabled: updaterEnabled,
    autoCheckOnMount:
      !appSettingsLoading && appSettings.automaticAppUpdateChecksEnabled,
    notificationSoundsEnabled: appSettings.notificationSoundsEnabled,
    systemNotificationsEnabled: appSettings.systemNotificationsEnabled,
    subagentSystemNotificationsEnabled:
      appSettings.subagentSystemNotificationsEnabled,
    isSubagentThread,
    getWorkspaceName,
    onThreadNotificationSent: (workspaceId, threadId) =>
      recordPendingThreadLinkRef.current(workspaceId, threadId),
    onDebug: addDebugEntry,
    successSoundUrl,
    errorSoundUrl,
  });
  const gitState = useMainAppGitState({
    activeWorkspace,
    activeWorkspaceId,
    activeItems,
    activeThreadId,
    activeTab,
    tabletTab,
    isCompact,
    isTablet,
    setActiveTab,
    appSettings: {
      preloadGitDiffs: appSettings.preloadGitDiffs,
      gitDiffIgnoreWhitespaceChanges: appSettings.gitDiffIgnoreWhitespaceChanges,
      splitChatDiffView: appSettings.splitChatDiffView,
      reviewDeliveryMode: appSettings.reviewDeliveryMode,
    },
    addDebugEntry,
    updateWorkspaceSettings,
    commitMessageModelId,
    connectWorkspace,
    startThreadForWorkspace,
    sendUserMessageToThread,
  });
  const {
    activeWorkspaceRef,
    activeWorkspaceIdRef,
    queueGitStatusRefresh,
    alertError,
    centerMode,
    setCenterMode,
    selectedDiffPath,
    setSelectedDiffPath,
    gitPanelMode,
    setGitPanelMode,
    gitDiffViewStyle,
    setGitDiffViewStyle,
    filePanelMode,
    selectedPullRequest,
    setSelectedPullRequest,
    selectedCommitSha,
    diffSource,
    setDiffSource,
    gitStatus,
    gitLogEntries,
    gitLogAheadEntries,
    gitLogBehindEntries,
    shouldLoadDiffs,
    activeDiffs,
    activeDiffLoading,
    activeDiffError,
    shouldLoadGitHubPanelData,
    handleGitIssuesChange,
    handleGitPullRequestsChange,
    handleGitPullRequestDiffsChange,
    handleGitPullRequestCommentsChange,
    refreshGitRemote,
    branches,
    currentBranch,
    isBranchSwitcherEnabled,
    handleCheckoutBranch,
    handleCreateGitHubRepo,
    createGitHubRepoLoading,
    handleInitGitRepo,
    initGitRepoLoading,
    isLaunchingPullRequestReview,
    pullRequestReviewActions,
    runPullRequestReview,
  } = gitState;
  queueGitStatusRefreshRef.current = queueGitStatusRefresh;
  const { isExpanded: composerEditorExpanded, toggleExpanded: toggleComposerEditorExpanded } =
    useComposerEditorState();

  const composerEditorSettings = useMemo<ComposerEditorSettings>(
    () => ({
      preset: appSettings.composerEditorPreset,
      expandFenceOnSpace: appSettings.composerFenceExpandOnSpace,
      expandFenceOnEnter: appSettings.composerFenceExpandOnEnter,
      fenceLanguageTags: appSettings.composerFenceLanguageTags,
      fenceWrapSelection: appSettings.composerFenceWrapSelection,
      autoWrapPasteMultiline: appSettings.composerFenceAutoWrapPasteMultiline,
      autoWrapPasteCodeLike: appSettings.composerFenceAutoWrapPasteCodeLike,
      continueListOnShiftEnter: appSettings.composerListContinuation,
    }),
    [
      appSettings.composerEditorPreset,
      appSettings.composerFenceExpandOnSpace,
      appSettings.composerFenceExpandOnEnter,
      appSettings.composerFenceLanguageTags,
      appSettings.composerFenceWrapSelection,
      appSettings.composerFenceAutoWrapPasteMultiline,
      appSettings.composerFenceAutoWrapPasteCodeLike,
      appSettings.composerListContinuation,
    ],
  );

  const { apps } = useApps({
    activeWorkspace,
    activeThreadId,
    enabled: appSettings.experimentalAppsEnabled,
    onDebug: addDebugEntry,
  });

  useThreadCodexSyncOrchestration({
    activeWorkspaceId,
    activeThreadId,
    appSettings: {
      defaultAccessMode: appSettings.defaultAccessMode,
      lastComposerModelId: appSettings.lastComposerModelId,
      lastComposerReasoningEffort: appSettings.lastComposerReasoningEffort,
    },
    threadCodexParamsVersion,
    getThreadCodexParams,
    patchThreadCodexParams,
    setThreadCodexSelectionKey,
    setAccessMode,
    setPreferredModelId,
    setPreferredEffort,
    setPreferredServiceTier,
    setPreferredCollabModeId,
    setPreferredCodexArgsOverride,
    activeThreadIdRef,
    pendingNewThreadSeedRef,
    selectedModelId,
    resolvedEffort,
    selectedServiceTier,
    accessMode,
    selectedCollaborationModeId,
    selectedCodexArgsOverride,
  });

  const { handleSetThreadListSortKey, handleRefreshAllWorkspaceThreads } =
    useThreadListActions({
      threadListSortKey,
      setThreadListSortKey,
      workspaces,
      refreshWorkspaces,
      listThreadsForWorkspaces,
      resetWorkspaceThreads,
    });

  useResponseRequiredNotificationsController({
    systemNotificationsEnabled: appSettings.systemNotificationsEnabled,
    subagentSystemNotificationsEnabled:
      appSettings.subagentSystemNotificationsEnabled,
    isSubagentThread,
    approvals,
    userInputRequests,
    getWorkspaceName,
    onDebug: addDebugEntry,
  });

  const {
    activeAccount,
    accountSwitching,
    handleSwitchAccount,
    handleCancelSwitchAccount,
  } = useAccountSwitching({
    activeWorkspaceId,
    accountByWorkspace,
    refreshAccountInfo,
    refreshAccountRateLimits,
    alertError,
  });
  const {
    newAgentDraftWorkspaceId,
    startingDraftThreadWorkspaceId,
    isDraftModeForActiveWorkspace: isNewAgentDraftMode,
    startNewAgentDraft,
    clearDraftState,
    clearDraftStateIfDifferentWorkspace,
    runWithDraftStart,
  } = useNewAgentDraft({
    activeWorkspace,
    activeWorkspaceId,
    activeThreadId,
  });
  const { getThreadRows } = useThreadRows(threadParentById);

  useTrayRecentThreads({
    workspaces,
    threadsByWorkspace,
    isSubagentThread,
  });

  useAutoExitEmptyDiff({
    centerMode,
    autoExitEnabled: diffSource === "local",
    activeDiffCount: activeDiffs.length,
    activeDiffLoading,
    activeDiffError,
    activeThreadId,
    isCompact,
    setCenterMode,
    setSelectedDiffPath,
    setActiveTab,
  });

  const { handleCopyThread } = useCopyThread({
    activeItems,
    onDebug: addDebugEntry,
  });

  const {
    renamePrompt: renameWorktreePrompt,
    notice: renameWorktreeNotice,
    upstreamPrompt: renameWorktreeUpstreamPrompt,
    confirmUpstream: confirmRenameWorktreeUpstream,
    openRenamePrompt: openRenameWorktreePrompt,
    handleRenameChange: handleRenameWorktreeChange,
    handleRenameCancel: handleRenameWorktreeCancel,
    handleRenameConfirm: handleRenameWorktreeConfirm,
  } = useRenameWorktreePrompt({
    workspaces,
    activeWorkspaceId,
    renameWorktree,
    renameWorktreeUpstream,
    onRenameSuccess: (workspace) => {
      resetWorkspaceThreads(workspace.id);
      void listThreadsForWorkspace(workspace);
      if (activeThreadId && activeWorkspaceId === workspace.id) {
        void refreshThread(workspace.id, activeThreadId);
      }
    },
  });

  const handleOpenRenameWorktree = useCallback(() => {
    if (activeWorkspace) {
      openRenameWorktreePrompt(activeWorkspace.id);
    }
  }, [activeWorkspace, openRenameWorktreePrompt]);

  const {
    terminalTabs,
    activeTerminalId,
    onSelectTerminal,
    onNewTerminal,
    onCloseTerminal,
    terminalState,
    ensureTerminalWithTitle,
    restartTerminalSession,
    requestTerminalFocus,
  } = useTerminalController({
    activeWorkspaceId,
    activeWorkspace,
    terminalOpen,
    onCloseTerminalPanel: closeTerminalPanel,
    onDebug: addDebugEntry,
  });

  const ensureLaunchTerminal = useCallback(
    (workspaceId: string) => ensureTerminalWithTitle(workspaceId, "launch", "Launch"),
    [ensureTerminalWithTitle],
  );

  const openTerminalWithFocus = useCallback(() => {
    if (!activeWorkspaceId) {
      return;
    }
    requestTerminalFocus();
    openTerminal();
  }, [activeWorkspaceId, openTerminal, requestTerminalFocus]);

  const handleToggleTerminalWithFocus = useCallback(() => {
    if (!activeWorkspaceId) {
      return;
    }
    if (!terminalOpen) {
      requestTerminalFocus();
    }
    handleToggleTerminal();
  }, [
    activeWorkspaceId,
    handleToggleTerminal,
    requestTerminalFocus,
    terminalOpen,
  ]);

  const launchScriptState = useWorkspaceLaunchScript({
    activeWorkspace,
    updateWorkspaceSettings,
    openTerminal: openTerminalWithFocus,
    ensureLaunchTerminal,
    restartLaunchSession: restartTerminalSession,
    terminalState,
    activeTerminalId,
  });

  const launchScriptsState = useWorkspaceLaunchScripts({
    activeWorkspace,
    updateWorkspaceSettings,
    openTerminal: openTerminalWithFocus,
    ensureLaunchTerminal: (workspaceId, entry, title) => {
      const label = entry.label?.trim() || entry.icon;
      return ensureTerminalWithTitle(
        workspaceId,
        `launch:${entry.id}`,
        title || `Launch ${label}`,
      );
    },
    restartLaunchSession: restartTerminalSession,
    terminalState,
    activeTerminalId,
  });

  const worktreeSetupScriptState = useWorktreeSetupScript({
    ensureTerminalWithTitle,
    restartTerminalSession,
    openTerminal,
    onDebug: addDebugEntry,
  });

  const handleWorktreeCreated = useCallback(
    async (worktree: WorkspaceInfo, _parentWorkspace?: WorkspaceInfo) => {
      await worktreeSetupScriptState.maybeRunWorktreeSetupScript(worktree);
    },
    [worktreeSetupScriptState],
  );

  const { exitDiffView, selectWorkspace, selectHome } = useWorkspaceSelection({
    workspaces,
    isCompact,
    setActiveTab,
    setActiveWorkspaceId,
    updateWorkspaceSettings,
    setCenterMode,
    setSelectedDiffPath,
  });

  const resolveCloneProjectContext = useCallback(
    (workspace: WorkspaceInfo) => {
      const groupId = workspace.settings.groupId ?? null;
      const group = groupId
        ? appSettings.workspaceGroups.find((entry) => entry.id === groupId)
        : null;
      return {
        groupId,
        copiesFolder: group?.copiesFolder ?? null,
      };
    },
    [appSettings.workspaceGroups],
  );

  const { handleMoveWorkspace } = useWorkspaceOrderingOrchestration({
    workspaces,
    workspacesById,
    updateWorkspaceSettings,
  });

  const {
    handleSelectOpenAppId,
    handleToggleAutomaticAppUpdateChecks,
    persistProjectCopiesFolder,
  } = useMainAppSettingsActions({
    appSettings,
    setAppSettings,
    queueSaveSettings,
  });

  const openAppIconById = useOpenAppIcons(appSettings.openAppTargets);

  const {
    workspaceFromUrlPrompt,
    openWorkspaceFromUrlPrompt,
    closeWorkspaceFromUrlPrompt,
    chooseWorkspaceFromUrlDestinationPath,
    submitWorkspaceFromUrlPrompt,
    updateWorkspaceFromUrlUrl,
    updateWorkspaceFromUrlTargetFolderName,
    clearWorkspaceFromUrlDestinationPath,
    canSubmitWorkspaceFromUrlPrompt,
  } = useWorkspaceFromUrlPrompt({
    onSubmit: async (url, destinationPath, targetFolderName) => {
      await handleAddWorkspaceFromGitUrl(url, destinationPath, targetFolderName);
    },
  });

  const { appModalsProps, modalActions } = useMainAppModals({
    settingsViewComponent: SettingsView,
    workspaces,
    workspaceGroups,
    groupedWorkspaces,
    ungroupedLabel,
    activeWorkspace,
    setActiveWorkspaceId,
    branches,
    currentBranch,
    threadRename: {
      threadsByWorkspace,
      renameThread,
    },
    git: {
      checkoutBranch: handleCheckoutBranch,
      initGitRepo: handleInitGitRepo,
      createGitHubRepo: handleCreateGitHubRepo,
      refreshGitRemote,
      initGitRepoLoading,
      createGitHubRepoLoading,
    },
    workspacePrompts: {
      addWorktreeAgent,
      addCloneAgent,
      connectWorkspace,
      updateWorkspaceSettings,
      selectWorkspace,
      handleWorktreeCreated,
      resolveCloneProjectContext,
      persistProjectCopiesFolder,
      onCompactActivate: isCompact ? () => setActiveTab("codex") : undefined,
      onWorkspacePromptError: (message, kind) => {
        addDebugEntry({
          id: `${Date.now()}-client-add-${kind}-error`,
          timestamp: Date.now(),
          source: "error",
          label: `${kind}/add error`,
          payload: message,
        });
      },
      mobileRemoteWorkspacePathPrompt,
      updateMobileRemoteWorkspacePathInput,
      appendMobileRemoteWorkspacePathFromRecent,
      cancelMobileRemoteWorkspacePathPrompt,
      submitMobileRemoteWorkspacePathPrompt,
      openWorkspaceFromUrlPrompt,
      workspaceFromUrl: {
        workspaceFromUrlPrompt,
        workspaceFromUrlCanSubmit: canSubmitWorkspaceFromUrlPrompt,
        onWorkspaceFromUrlPromptUrlChange: updateWorkspaceFromUrlUrl,
        onWorkspaceFromUrlPromptTargetFolderNameChange:
          updateWorkspaceFromUrlTargetFolderName,
        onWorkspaceFromUrlPromptChooseDestinationPath:
          chooseWorkspaceFromUrlDestinationPath,
        onWorkspaceFromUrlPromptClearDestinationPath:
          clearWorkspaceFromUrlDestinationPath,
        onWorkspaceFromUrlPromptCancel: closeWorkspaceFromUrlPrompt,
        onWorkspaceFromUrlPromptConfirm: submitWorkspaceFromUrlPrompt,
      },
    },
    settings: {
      handleMoveWorkspace,
      removeWorkspace,
      createWorkspaceGroup,
      renameWorkspaceGroup,
      moveWorkspaceGroup,
      deleteWorkspaceGroup,
      assignWorkspaceGroup,
      reduceTransparency,
      setReduceTransparency,
      appSettings,
      openAppIconById,
      queueSaveSettings,
      handleToggleAutomaticAppUpdateChecks,
      doctor,
      codexUpdate,
      updateWorkspaceSettings,
      scaleShortcutTitle,
      scaleShortcutText,
      handleTestNotificationSound,
      handleTestSystemNotification,
      handleMobileConnectSuccess,
      dictationModel,
    },
  });

  useBranchSwitcherShortcut({
    shortcut: appSettings.branchSwitcherShortcut,
    isEnabled: isBranchSwitcherEnabled,
    onTrigger: modalActions.openBranchSwitcher,
  });

  const handleRenameThread = useCallback(
    (workspaceId: string, threadId: string) => {
      modalActions.openRenamePrompt(workspaceId, threadId);
    },
    [modalActions],
  );

  const showHome = !activeWorkspace;
  const {
    latestAgentRuns,
    isLoadingLatestAgents,
    usageMetric,
    setUsageMetric,
    usageWorkspaceId,
    setUsageWorkspaceId,
    usageWorkspaceOptions,
    localUsageSnapshot,
    isLoadingLocalUsage,
    localUsageError,
    refreshLocalUsage,
  } = useWorkspaceInsightsOrchestration({
    workspaces,
    workspacesById,
    hasLoaded,
    showHome,
    threadsByWorkspace,
    lastAgentMessageByThread,
    threadStatusById,
    threadListLoadingByWorkspace,
    getWorkspaceGroupName,
  });

  const activeRateLimits = activeWorkspaceId
    ? rateLimitsByWorkspace[activeWorkspaceId] ?? null
    : null;
  const {
    homeAccount,
    homeRateLimits,
  } = useHomeAccount({
    showHome,
    usageWorkspaceId,
    workspaces,
    threadsByWorkspace,
    threadListLoadingByWorkspace,
    rateLimitsByWorkspace,
    accountByWorkspace,
    refreshAccountInfo,
    refreshAccountRateLimits,
  });
  const activeTokenUsage = activeThreadId
    ? tokenUsageByThread[activeThreadId] ?? null
    : null;
  useTraySessionUsage({
    accountRateLimits: activeRateLimits,
    showRemaining: appSettings.usageShowRemaining,
  });
  const activePlan = activeThreadId
    ? planByThread[activeThreadId] ?? null
    : null;
  const hasActivePlan = Boolean(
    activePlan && (activePlan.steps.length > 0 || activePlan.explanation)
  );
  const composerWorkspaceState = useMainAppComposerWorkspaceState({
    view: {
      activeTab,
      tabletTab,
      centerMode,
      isCompact,
      isTablet,
      rightPanelCollapsed,
      filePanelMode,
    },
    workspace: {
      activeWorkspace,
      activeWorkspaceId,
      isNewAgentDraftMode,
      startingDraftThreadWorkspaceId,
      threadsByWorkspace,
    },
    thread: {
      activeThreadId,
      activeItems,
      threadStatusById,
      activeTurnIdByThread,
      userInputRequests,
    },
    settings: {
      steerEnabled: appSettings.steerEnabled,
      followUpMessageBehavior: appSettings.followUpMessageBehavior,
      experimentalAppsEnabled: appSettings.experimentalAppsEnabled,
      pauseQueuedMessagesWhenResponseRequired:
        appSettings.pauseQueuedMessagesWhenResponseRequired,
    },
    models: {
      models,
      selectedModelId,
      resolvedEffort,
      selectedServiceTier,
      collaborationModePayload,
    },
    refs: {
      composerInputRef,
      workspaceHomeTextareaRef,
    },
    actions: {
      connectWorkspace,
      startThreadForWorkspace,
      sendUserMessage,
      sendUserMessageToThread,
      seedThreadCodexParams: patchThreadCodexParams,
      startFork,
      startReview,
      startResume,
      startCompact,
      startApps,
      startMcp,
      startFast,
      startStatus,
      addWorktreeAgent,
      handleWorktreeCreated,
      addDebugEntry,
    },
  });
  const {
    files,
    setFileAutocompleteActive,
    showWorkspaceHome,
    showComposer,
    canInterrupt,
    recentThreadInstances,
    recentThreadsUpdatedAt,
    clearActiveImages,
    removeImagesForThread,
    handleSend,
    setPrefillDraft,
    clearDraftForThread,
    workspaceHomeState,
    agentMdState,
  } = composerWorkspaceState;
  const {
    runs: workspaceRuns,
    draft: workspacePrompt,
    runMode: workspaceRunMode,
    modelSelections: workspaceModelSelections,
    error: workspaceRunError,
    isSubmitting: workspaceRunSubmitting,
    setDraft: setWorkspacePrompt,
    setRunMode: setWorkspaceRunMode,
    toggleModelSelection: toggleWorkspaceModelSelection,
    setModelCount: setWorkspaceModelCount,
    startRun: startWorkspaceRun,
  } = workspaceHomeState;
  const {
    content: agentMdContent,
    exists: agentMdExists,
    truncated: agentMdTruncated,
    isLoading: agentMdLoading,
    isSaving: agentMdSaving,
    error: agentMdError,
    isDirty: agentMdDirty,
    setContent: setAgentMdContent,
    refresh: refreshAgentMd,
    save: saveAgentMd,
  } = agentMdState;
  const promptActions = useMainAppPromptActions({
    activeWorkspace,
    connectWorkspace,
    startThreadForWorkspace,
    sendUserMessageToThread,
    alertError,
    createPrompt,
    updatePrompt,
    deletePrompt,
    movePrompt,
    getWorkspacePromptsDir,
    getGlobalPromptsDir,
  });
  const worktreeState = useMainAppWorktreeState({
    activeWorkspace,
    workspacesById,
    renameWorktreePrompt,
    renameWorktreeNotice,
    renameWorktreeUpstreamPrompt,
    confirmRenameWorktreeUpstream,
    handleOpenRenameWorktree,
    handleRenameWorktreeChange,
    handleRenameWorktreeCancel,
    handleRenameWorktreeConfirm,
  });
  const { baseWorkspaceRef } = worktreeState;

  useMainAppWorkspaceLifecycle({
    activeTab,
    isTablet,
    setActiveTab,
    workspaces,
    hasLoaded,
    connectWorkspace,
    listThreadsForWorkspaces,
    refreshWorkspaces,
    backendMode: appSettings.backendMode,
    activeWorkspace,
    activeThreadId,
    threadStatusById,
    remoteThreadConnectionState,
    refreshThread,
  });

  const {
    handleAddWorkspace,
    handleAddWorkspaceFromGitUrl,
    handleAddAgent,
    handleAddWorktreeAgent,
    handleAddCloneAgent,
    dropTargetRef: workspaceDropTargetRef,
    isDragOver: isWorkspaceDropActive,
    handleDragOver: handleWorkspaceDragOver,
    handleDragEnter: handleWorkspaceDragEnter,
    handleDragLeave: handleWorkspaceDragLeave,
    handleDrop: handleWorkspaceDrop,
  } = useMainAppWorkspaceActions({
    workspaceActions: {
      isCompact,
      addWorkspace,
      addWorkspaceFromPath,
      addWorkspaceFromGitUrl,
      addWorkspacesFromPaths,
      setActiveThreadId,
      setActiveTab,
      exitDiffView,
      selectWorkspace,
      onStartNewAgentDraft: startNewAgentDraft,
      openWorktreePrompt: modalActions.openWorktreePrompt,
      openClonePrompt: modalActions.openClonePrompt,
      composerInputRef,
      onDebug: addDebugEntry,
    },
  });

  useInterruptShortcut({
    isEnabled: canInterrupt,
    shortcut: appSettings.interruptShortcut,
    onTrigger: () => {
      void interruptTurn();
    },
  });

  const selectedCommitEntry = useMemo(() => {
    if (!selectedCommitSha) {
      return null;
    }
    return (
      [...gitLogAheadEntries, ...gitLogBehindEntries, ...gitLogEntries].find(
        (entry) => entry.sha === selectedCommitSha,
      ) ?? null
    );
  }, [gitLogAheadEntries, gitLogBehindEntries, gitLogEntries, selectedCommitSha]);

  const {
    handleSelectPullRequest,
    resetPullRequestSelection,
    composerContextActions,
    composerSendLabel,
    handleComposerSend,
  } = usePullRequestComposer({
    activeWorkspace,
    selectedPullRequest,
    selectedCommit: selectedCommitEntry,
    filePanelMode: filePanelMode === "seats" || filePanelMode === "sidechat" ? "git" : filePanelMode,
    gitPanelMode,
    centerMode,
    isCompact,
    setSelectedPullRequest,
    setDiffSource,
    setSelectedDiffPath,
    setCenterMode,
    setGitPanelMode,
    setPrefillDraft,
    setActiveTab,
    pullRequestReviewActions,
    pullRequestReviewLaunching: isLaunchingPullRequestReview,
    runPullRequestReview,
    startReview,
    clearActiveImages,
    handleSend,
  });

  const {
    handleComposerSendWithDraftStart,
    handleSelectWorkspaceInstance,
    handleOpenThreadLink,
    handleArchiveActiveThread,
  } = useThreadUiOrchestration({
    activeWorkspaceId,
    activeThreadId,
    accessMode,
    selectedServiceTier,
    selectedCollaborationModeId,
    selectedCodexArgsOverride,
    pendingNewThreadSeedRef,
    runWithDraftStart,
    handleComposerSend,
    clearDraftState,
    exitDiffView,
    resetPullRequestSelection,
    selectWorkspace,
    setActiveThreadId,
    setActiveTab,
    isCompact,
    removeThread,
    clearDraftForThread,
    removeImagesForThread,
  });

  const handleOpenThreadLinkFromExternal = useCallback(
    (workspaceId: string, threadId: string) => {
      leaveSecretary();
      setActiveTab("codex");
      handleOpenThreadLink(threadId, workspaceId);
    },
    [handleOpenThreadLink, setActiveTab, leaveSecretary],
  );

  const { recordPendingThreadLink, openThreadLinkOrQueue } =
    useSystemNotificationThreadLinks({
      hasLoadedWorkspaces: hasLoaded,
      workspacesById,
      refreshWorkspaces,
      connectWorkspace,
      openThreadLink: handleOpenThreadLinkFromExternal,
    });

  useTauriEvent(
    subscribeTrayOpenThread,
    ({ workspaceId, threadId }: { workspaceId: string; threadId: string }) => {
      openThreadLinkOrQueue(workspaceId, threadId);
    },
  );

  useEffect(() => {
    recordPendingThreadLinkRef.current = recordPendingThreadLink;
    return () => {
      recordPendingThreadLinkRef.current = () => {};
    };
  }, [recordPendingThreadLink]);

  const { handlePlanAccept, handlePlanSubmitChanges } = usePlanReadyActions({
    activeWorkspace,
    activeThreadId,
    collaborationModes,
    resolvedModel,
    resolvedEffort,
    connectWorkspace,
    sendUserMessageToThread,
    setSelectedCollaborationModeId,
    persistThreadCodexParams,
  });

  const {
    showGitDetail,
    isThreadOpen,
    dropOverlayActive,
    dropOverlayText,
    appClassName,
    appStyle,
  } = useAppShellOrchestration({
    isCompact,
    isPhone,
    isTablet,
    sidebarCollapsed,
    rightPanelCollapsed,
    shouldReduceTransparency,
    isWorkspaceDropActive,
    centerMode,
    selectedDiffPath,
    showComposer,
    activeThreadId,
    sidebarWidth,
    chatDiffSplitPositionPercent,
    rightPanelWidth,
    planPanelHeight,
    terminalPanelHeight,
    debugPanelHeight,
    appSettings,
  });

  const sidebarMenuOrchestration = useMainAppSidebarMenuOrchestration({
    sidebarActions: {
      openSettings: modalActions.openSettings,
      resetPullRequestSelection,
      clearDraftState,
      clearDraftStateIfDifferentWorkspace,
      selectHome,
      exitDiffView,
      selectWorkspace,
      setActiveThreadId,
      connectWorkspace,
      isCompact,
      setActiveTab,
      workspacesById,
      updateWorkspaceSettings,
      removeThread,
      clearDraftForThread,
      removeImagesForThread,
      refreshThread,
      handleRenameThread,
      removeWorkspace,
      removeWorktree,
      loadOlderThreadsForWorkspace,
      listThreadsForWorkspace,
    },
    workspaceCycling: {
      workspaces,
      groupedWorkspaces,
      threadsByWorkspace,
      getThreadRows,
      getPinTimestamp,
      pinnedThreadsVersion,
      activeWorkspaceIdRef,
      activeThreadIdRef,
      exitDiffView,
      resetPullRequestSelection,
      selectWorkspace,
      setActiveThreadId,
    },
    appMenu: {
      activeWorkspaceRef,
      baseWorkspaceRef,
      onAddWorkspace: handleAddWorkspace,
      onAddWorkspaceFromUrl: openWorkspaceFromUrlPrompt,
      onAddAgent: handleAddAgent,
      onAddWorktreeAgent: handleAddWorktreeAgent,
      onAddCloneAgent: handleAddCloneAgent,
      onToggleDebug: handleDebugClick,
      onToggleTerminal: handleToggleTerminalWithFocus,
      sidebarCollapsed,
      rightPanelCollapsed,
      onExpandSidebar: expandSidebar,
      onCollapseSidebar: collapseSidebar,
      onExpandRightPanel: expandRightPanel,
      onCollapseRightPanel: collapseRightPanel,
    },
    appSettings,
    onDebug: addDebugEntry,
  });
  useArchiveShortcut({
    isEnabled: isThreadOpen,
    shortcut: appSettings.archiveThreadShortcut,
    onTrigger: handleArchiveActiveThread,
  });
  const showCompactCodexThreadActions =
    Boolean(activeWorkspace) &&
    isCompact &&
    ((isPhone && activeTab === "codex") || (isTablet && tabletTab === "codex"));
  const showMobilePollingFetchStatus =
    showCompactCodexThreadActions &&
    Boolean(activeWorkspace?.connected) &&
    appSettings.backendMode === "remote" &&
    remoteThreadConnectionState === "polling";
  const gitRootOverride = activeWorkspace?.settings.gitRoot;
  const hasGitRootOverride =
    typeof gitRootOverride === "string" && gitRootOverride.trim().length > 0;
  const showGitInitBanner =
    Boolean(activeWorkspace) && !hasGitRootOverride && isMissingRepo(gitStatus.error);
  const displayNodes = useMainAppDisplayNodes({
    showCompactCodexThreadActions,
    handleMobileThreadRefresh,
    mobileThreadRefreshLoading,
    centerMode,
    gitDiffViewStyle,
    setGitDiffViewStyle,
    isCompact,
    rightPanelCollapsed,
    sidebarToggleProps,
    workspaceHomeProps: activeWorkspace
      ? {
          workspace: activeWorkspace,
          showGitInitBanner,
          initGitRepoLoading,
          onInitGitRepo: modalActions.openInitGitRepoPrompt,
          runs: workspaceRuns,
          recentThreadInstances,
          recentThreadsUpdatedAt,
          prompt: workspacePrompt,
          onPromptChange: setWorkspacePrompt,
          onStartRun: startWorkspaceRun,
          runMode: workspaceRunMode,
          onRunModeChange: setWorkspaceRunMode,
          models,
          selectedModelId,
          onSelectModel: setSelectedModelId,
          modelSelections: workspaceModelSelections,
          onToggleModel: toggleWorkspaceModelSelection,
          onModelCountChange: setWorkspaceModelCount,
          collaborationModes,
          selectedCollaborationModeId,
          onSelectCollaborationMode: setSelectedCollaborationModeId,
          reasoningOptions,
          selectedEffort,
          onSelectEffort: setSelectedEffort,
          reasoningSupported,
          error: workspaceRunError,
          isSubmitting: workspaceRunSubmitting,
          activeWorkspaceId,
          activeThreadId,
          threadStatusById,
          onSelectInstance: handleSelectWorkspaceInstance,
          skills,
          appsEnabled: appSettings.experimentalAppsEnabled,
          apps,
          prompts,
          files,
          onFileAutocompleteActiveChange: setFileAutocompleteActive,
          dictationEnabled: appSettings.dictationEnabled && dictationReady,
          dictationState,
          dictationLevel,
          onToggleDictation: handleToggleDictation,
          onCancelDictation: cancelDictation,
          onOpenDictationSettings: () => modalActions.openSettings("dictation"),
          dictationError,
          onDismissDictationError: clearDictationError,
          dictationHint,
          onDismissDictationHint: clearDictationHint,
          dictationTranscript,
          onDictationTranscriptHandled: clearDictationTranscript,
          textareaRef: workspaceHomeTextareaRef,
          agentMdContent,
          agentMdExists,
          agentMdTruncated,
          agentMdLoading,
          agentMdSaving,
          agentMdError,
          agentMdDirty,
          onAgentMdChange: setAgentMdContent,
          onAgentMdRefresh: () => {
            void refreshAgentMd();
          },
          onAgentMdSave: () => {
            void saveAgentMd();
          },
        }
      : null,
  });
  const { workspaceHomeNode } = displayNodes;
  const layoutSurfaces = useMainAppLayoutSurfaces({
    appSettings: {
      usageShowRemaining: appSettings.usageShowRemaining,
      composerCodeBlockCopyUseModifier:
        appSettings.composerCodeBlockCopyUseModifier,
      showMessageFilePath: appSettings.showMessageFilePath,
      openAppTargets: appSettings.openAppTargets,
      selectedOpenAppId: appSettings.selectedOpenAppId,
      experimentalAppsEnabled: appSettings.experimentalAppsEnabled,
      followUpMessageBehavior: appSettings.followUpMessageBehavior,
      composerFollowUpHintEnabled: appSettings.composerFollowUpHintEnabled,
      dictationEnabled: appSettings.dictationEnabled,
      splitChatDiffView: appSettings.splitChatDiffView,
      gitDiffIgnoreWhitespaceChanges:
        appSettings.gitDiffIgnoreWhitespaceChanges,
    },
    workspaces,
    groupedWorkspaces,
    workspaceGroupsCount: workspaceGroups.length,
    deletingWorktreeIds,
    newAgentDraftWorkspaceId,
    startingDraftThreadWorkspaceId,
    threadsByWorkspace,
    threadParentById,
    threadStatusById,
    threadResumeLoadingById,
    threadListLoadingByWorkspace,
    threadListPagingByWorkspace,
    threadListCursorByWorkspace,
    pinnedThreadsVersion,
    threadListSortKey,
    onSetThreadListSortKey: handleSetThreadListSortKey,
    threadListOrganizeMode,
    onSetThreadListOrganizeMode: setThreadListOrganizeMode,
    onRefreshAllThreads: handleRefreshAllWorkspaceThreads,
    activeWorkspace,
    activeWorkspaceId,
    activeThreadId,
    activeItems,
    userInputRequests,
    approvals,
    activeRateLimits,
    activeAccount,
    homeRateLimits,
    homeAccount,
    accountSwitching,
    onSwitchAccount: handleSwitchAccount,
    onCancelSwitchAccount: handleCancelSwitchAccount,
    onDecision: handleApprovalDecision,
    onRemember: handleApprovalRemember,
    onUserInputSubmit: handleUserInputSubmit,
    onPlanAccept: handlePlanAccept,
    onPlanSubmitChanges: handlePlanSubmitChanges,
    activePlan,
    activeTokenUsage,
    latestAgentRuns,
    isLoadingLatestAgents,
    localUsageSnapshot,
    isLoadingLocalUsage,
    localUsageError,
    onRefreshLocalUsage: () => {
      refreshLocalUsage()?.catch(() => {});
    },
    usageMetric,
    onUsageMetricChange: setUsageMetric,
    usageWorkspaceId,
    usageWorkspaceOptions,
    onUsageWorkspaceChange: setUsageWorkspaceId,
    gitState,
    selectedServiceTier: selectedServiceTier ?? null,
    composerWorkspaceState,
    promptActions,
    worktreeState,
    sidebarHandlers: sidebarMenuOrchestration,
    displayNodes,
    threadPinning: {
      pinThread,
      unpinThread,
      isThreadPinned,
      getPinTimestamp,
      getThreadArgsBadge,
    },
    workspaceDrop: {
      workspaceDropTargetRef,
      isWorkspaceDropActive: dropOverlayActive,
      workspaceDropText: dropOverlayText,
      onWorkspaceDragOver: handleWorkspaceDragOver,
      onWorkspaceDragEnter: handleWorkspaceDragEnter,
      onWorkspaceDragLeave: handleWorkspaceDragLeave,
      onWorkspaceDrop: handleWorkspaceDrop,
    },
    threadNavigation: {
      exitDiffView,
      clearDraftState,
      selectWorkspace,
      setActiveThreadId,
      resetPullRequestSelection,
      selectHome,
    },
    pullRequestComposer: {
      composerSendLabel,
      handleSelectPullRequest,
    },
    dictationUi: {
      onOpenDictationSettings: () => modalActions.openSettings('dictation'),
      dictationTranscript,
      dictationError,
      dictationHint,
    },
    openAppIconById,
    openInitGitRepoPrompt: modalActions.openInitGitRepoPrompt,
    startUncommittedReview,
    handleAddWorkspace,
    openWorkspaceFromUrlPrompt,
    handleAddAgent,
    handleAddWorktreeAgent,
    handleAddCloneAgent,
    handleOpenThreadLink,
    handleSelectOpenAppId,
    handleCopyThread,
    handleToggleTerminalWithFocus,
    launchScriptState,
    launchScriptsState,
    models,
    selectedModelId,
    onSelectModel: handleSelectModel,
    collaborationModes,
    selectedCollaborationModeId,
    onSelectCollaborationMode: handleSelectCollaborationMode,
    reasoningOptions,
    selectedEffort,
    onSelectEffort: handleSelectEffort,
    reasoningSupported,
    codexArgsOptions,
    selectedCodexArgsOverride,
    onSelectCodexArgsOverride: handleSelectCodexArgsOverride,
    accessMode,
    onSelectAccessMode: handleSelectAccessMode,
    skills,
    apps,
    prompts,
    composerInputRef,
    composerEditorSettings,
    composerEditorExpanded,
    onToggleComposerEditorExpanded: toggleComposerEditorExpanded,
    dictationReady,
    dictationState,
    dictationLevel,
    onToggleDictation: handleToggleDictation,
    onCancelDictation: cancelDictation,
    clearDictationTranscript,
    clearDictationError,
    clearDictationHint,
    composerContextActions,
    reviewPrompt,
    closeReviewPrompt,
    showPresetStep,
    choosePreset,
    highlightedPresetIndex,
    setHighlightedPresetIndex,
    highlightedBranchIndex,
    setHighlightedBranchIndex,
    highlightedCommitIndex,
    setHighlightedCommitIndex,
    handleReviewPromptKeyDown,
    selectBranch,
    selectBranchAtIndex,
    confirmBranch,
    selectCommit,
    selectCommitAtIndex,
    confirmCommit,
    updateCustomInstructions,
    confirmCustom,
    handleComposerSendWithDraftStart,
    interruptTurn,
    terminalOpen,
    debugOpen,
    debugEntries,
    terminalTabs,
    activeTerminalId,
    onSelectTerminal,
    onNewTerminal,
    onCloseTerminal,
    terminalState,
    onClearDebug: clearDebugEntries,
    onCopyDebug: handleCopyDebug,
    onResizeDebug: onDebugPanelResizeStart,
    onResizeTerminal: onTerminalPanelResizeStart,
    isCompact,
    isPhone,
    activeTab,
    setActiveTab,
    tabletTab,
    showMobilePollingFetchStatus,
    appModalsAboutOpen:
      appModalsProps.settingsOpen && appModalsProps.settingsSection === 'about',
    updaterState,
    startUpdate,
    dismissUpdate,
    postUpdateNotice,
    dismissPostUpdateNotice,
    errorToasts,
    dismissErrorToast,
    showDebugButton,
    handleDebugClick,
  });

  const projectNavigation = useRef({ workspaceId: activeWorkspaceId, threadId: activeThreadId });
  projectNavigation.current = { workspaceId: activeWorkspaceId, threadId: activeThreadId };
  useEffect(() => { leaveSecretary(); }, [activeWorkspaceId, activeThreadId, leaveSecretary]);
  const secretaryHasAssociation = Boolean(secretaryView?.association && secretaryView.conversation)
    && [secretaryView!.association!.threadId, secretaryView!.association!.domainId,
      secretaryView!.association!.seatId, secretaryView!.association!.sessionId]
      .every((value) => typeof value === "string" && value.length > 0)
    && (secretaryView!.association!.workspaceId === null ||
      (typeof secretaryView!.association!.workspaceId === "string" &&
        secretaryView!.association!.workspaceId.length > 0))
    && secretaryView!.conversation!.messages.workspaceId === secretaryView!.association!.workspaceId
    && secretaryView!.conversation!.messages.threadId === secretaryView!.association!.threadId;
  const secretarySettingsOnly = secretaryView !== null && secretaryView.association === null
    && secretaryView.conversation === null;
  const secretaryActive = (secretaryHasAssociation || secretarySettingsOnly)
    && selectedSecretary === secretaryToken;
  const secretaryCanOpen = (secretaryHasAssociation || secretarySettingsOnly) && secretaryView!.readState === "known"
    && Boolean(secretaryView!.openConversation) && secretaryOpening !== secretaryToken;
  const openSecretary = async () => {
    if (!secretaryCanOpen || !secretaryView?.openConversation
      || secretaryOpeningRef.current === secretaryToken) return;
    const view = secretaryView;
    const token = secretaryToken;
    const navigation = secretaryNavigation.current;
    const project = projectNavigation.current;
    secretaryOpeningRef.current = token;
    setSecretaryOpening(token);
    setSecretaryOpenError(null);
    try {
      await view.openConversation!();
      if (!secretaryMounted.current || currentSecretary.current.token !== token
        || currentSecretary.current.view?.readState !== "known" || secretaryNavigation.current !== navigation
        || projectNavigation.current.workspaceId !== project.workspaceId
        || projectNavigation.current.threadId !== project.threadId) return;
      // Reuse the ordinary renderers with host-owned data; never call the legacy thread-resume hook.
      setSelectedSecretary(token);
      setActiveTab("codex");
      expandRightPanel();
    } catch (cause) {
      if (secretaryMounted.current && currentSecretary.current.token === token) {
        setSecretaryOpenError({ token, text: cause instanceof Error ? cause.message : String(cause) });
      }
    } finally {
      if (secretaryOpeningRef.current === token) secretaryOpeningRef.current = null;
      if (secretaryMounted.current && currentSecretary.current.token === token) setSecretaryOpening(null);
    }
  };
  const integratedSurfaces = { ...layoutSurfaces, primary: {
    ...layoutSurfaces.primary,
    sidebarProps: { ...layoutSurfaces.primary.sidebarProps },
    messagesProps: { ...layoutSurfaces.primary.messagesProps },
  } };
  const ordinarySidebar = layoutSurfaces.primary.sidebarProps;
  integratedSurfaces.primary.sidebarProps.onSelectHome = () => { leaveSecretary(); ordinarySidebar.onSelectHome(); };
  integratedSurfaces.primary.sidebarProps.onSelectWorkspace = (id) => { leaveSecretary(); ordinarySidebar.onSelectWorkspace(id); };
  integratedSurfaces.primary.sidebarProps.onSelectThread = (workspaceId, threadId) => {
    leaveSecretary(); ordinarySidebar.onSelectThread(workspaceId, threadId);
  };
  if (secretaryActive) {
    integratedSurfaces.primary.sidebarProps.activeWorkspaceId = null;
    integratedSurfaces.primary.sidebarProps.activeThreadId = null;
    if (secretaryView!.conversation) {
      integratedSurfaces.primary.messagesProps = { ...secretaryView!.conversation.messages };
    }
    if (secretaryView!.readState === "frozen") {
      integratedSurfaces.primary.messagesProps.onUserInputSubmit = undefined;
      integratedSurfaces.primary.messagesProps.onPlanAccept = undefined;
      integratedSurfaces.primary.messagesProps.onPlanSubmitChanges = undefined;
    }
    integratedSurfaces.primary.composerProps = secretaryView!.conversation?.composer
      ? { ...secretaryView!.conversation.composer, disabled: secretaryView!.readState === "frozen"
        || secretaryView!.conversation.composer.disabled,
        canStop: secretaryView!.readState === "known" && secretaryView!.conversation.composer.canStop } : null;
  }
  // Fixed entry stays outside the conversation scroll region. Missing host data is not "unset".
  const secretaryReadTime = secretaryView ? new Date(secretaryView.readAt) : null;
  const secretaryReadTimeLabel = secretaryReadTime && !Number.isNaN(secretaryReadTime.getTime())
    ? secretaryReadTime.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false }) : "时间未知";
  integratedSurfaces.primary.sidebarProps.secretaryEntry = secretaryView ? <>
    {secretaryCanOpen ? (
      <SecretaryEntry state={secretaryView!.entry} active={secretaryActive} onOpen={() => void openSecretary()} />
    ) : (
      <button type="button" className="sec-entry" disabled>
        <span className="sec-avatar" aria-hidden>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
            <path d="M4 7h16M4 12h10M4 17h7" />
            <circle cx="18" cy="16" r="3" />
          </svg>
        </span>
        <span className="sec-entry-main">
          <span className="sec-entry-name">秘书长</span>
          <span className="sec-entry-sub">{secretaryOpening === secretaryToken ? "正在打开…"
            : secretaryView ? entryLine(secretaryView.entry).text : "数据还没接上"}</span>
        </span>
      </button>
    )}
    {secretaryView.readState === "frozen" ? <div className="sec-help" role="status">
      {secretaryReadTimeLabel} 最后读到；{secretaryView.readError ?? "最新状态未读到"}
    </div> : null}
    {secretaryOpenError?.token === secretaryToken ? <div className="sec-help" role="alert">{secretaryOpenError.text}</div> : null}
    {!secretaryActive && secretaryWriteFailure ? <div className="sec-help" role="alert">
      {secretaryWriteFailure.text}
      {secretaryWriteFailure.body ? <pre>{secretaryWriteFailure.body}</pre> : null}
    </div> : null}
  </> : null;
  integratedSurfaces.primary.messagesProps.afterItem = secretaryActive ? (itemId) => <>
    {secretaryView!.actionLines.filter((action) => action.itemId === itemId && action.turnId.length > 0)
      .map((action) => <SecretaryActionLine key={action.line.id} line={action.line}
        onOpen={secretaryView!.readState === "known" ? secretaryView!.openAction : undefined} />)}
  </> : undefined;

  const {
    sidebarNode,
    messagesNode,
    composerNode,
    approvalToastsNode,
    updateToastNode,
    errorToastsNode,
    homeNode,
    mainHeaderNode,
    desktopTopbarLeftNode,
    tabletNavNode,
    tabBarNode,
    gitDiffPanelNode,
    gitDiffViewerNode,
    planPanelNode,
    debugPanelNode,
    debugPanelFullNode,
    terminalDockNode,
    compactEmptyCodexNode,
    compactEmptyGitNode,
    compactGitBackNode,
  } = useMainAppLayoutNodes(integratedSurfaces);

  const mainMessagesNode = secretaryActive && secretarySettingsOnly
    ? <div role="status">{entryLine(secretaryView!.entry).text}。右侧显示宿主读回的设置和定时任务。</div>
    : !secretaryActive && showWorkspaceHome ? workspaceHomeNode : messagesNode;
  const nativeInput = secretaryActive && secretaryHasAssociation && secretaryView?.readState === "known"
    ? secretaryView.nativeInput : null;
  const sendSecretary = async () => {
    if (!secretaryKey || !nativeInput?.canSend || secretarySendBlocked || secretaryWritePendingRef.current ||
        !secretaryDraft.trim()) return;
    const token = secretaryToken;
    const submitted = { text: secretaryDraft, revision: secretaryDraftState?.revision ?? 0,
      binding: nativeInput.binding, requestId: null as string | null,
      earlierRequestIds: new Set(secretaryView?.outbound?.map((fact) => fact.requestId)) };
    secretarySubmittedDrafts.current.set(secretaryKey, submitted);
    secretaryWritePendingRef.current = true;
    setSecretaryWritePending(true);
    try {
      const fact = await nativeInput.send(nativeInput.binding, secretaryDraft);
      if (secretarySubmittedDrafts.current.get(secretaryKey) === submitted) {
        submitted.requestId = fact.requestId;
      }
      if (currentSecretary.current.token === token) {
        setSecretaryWriteFailure(null);
        void currentSecretary.current.view?.openConversation?.();
      }
    } catch (cause) {
      if (currentSecretary.current.token === token) {
        setSecretaryWriteFailure({ token, operation: "send",
          sessionId: nativeInput.binding.sessionId, body: secretaryDraft,
          text: cause instanceof Error ? cause.message : String(cause) });
        void currentSecretary.current.view?.openConversation?.();
      }
    } finally {
      secretaryWritePendingRef.current = false;
      setSecretaryWritePending(false);
    }
  };
  const stopSecretary = async () => {
    if (!nativeInput?.canStop || secretaryWritePendingRef.current) return;
    const token = secretaryToken;
    secretaryWritePendingRef.current = true;
    setSecretaryWritePending(true);
    try {
      await nativeInput.stop(nativeInput.binding);
      if (currentSecretary.current.token === token) {
        setSecretaryWriteFailure(null);
        void currentSecretary.current.view?.openConversation?.();
      }
    } catch (cause) {
      if (currentSecretary.current.token === token) {
        setSecretaryWriteFailure({ token, operation: "stop",
          sessionId: nativeInput.binding.sessionId, body: null,
          text: cause instanceof Error ? cause.message : String(cause) });
        void currentSecretary.current.view?.openConversation?.();
      }
    } finally {
      secretaryWritePendingRef.current = false;
      setSecretaryWritePending(false);
    }
  };
  const retrySecretaryStop = async (requestId: string) => {
    if (!secretaryView?.retryStop || secretaryWritePendingRef.current) return;
    const original = secretaryView.outbound?.find((fact) => fact.operation === "stop" &&
      fact.requestId === requestId);
    if (!original) return;
    const token = secretaryToken;
    secretaryWritePendingRef.current = true;
    setSecretaryWritePending(true);
    try {
      await secretaryView.retryStop(requestId);
      if (currentSecretary.current.token === token) {
        setSecretaryWriteFailure(null);
        void currentSecretary.current.view?.openConversation?.();
      }
    } catch (cause) {
      if (currentSecretary.current.token === token) {
        setSecretaryWriteFailure({ token, operation: "stop",
          sessionId: original.sessionId,
          body: null,
          text: cause instanceof Error ? cause.message : String(cause) });
        void currentSecretary.current.view?.openConversation?.();
      }
    } finally {
      secretaryWritePendingRef.current = false;
      setSecretaryWritePending(false);
    }
  };
  const retrySecretarySend = async (requestId: string) => {
    if (!secretaryView?.retrySend || secretaryWritePendingRef.current) return;
    const original = secretaryView.outbound?.find((fact) => fact.operation === "send" &&
      fact.requestId === requestId);
    if (!original) return;
    const token = secretaryToken;
    secretaryWritePendingRef.current = true;
    setSecretaryWritePending(true);
    try {
      await secretaryView.retrySend(requestId);
      if (currentSecretary.current.token === token) {
        setSecretaryWriteFailure(null);
        void currentSecretary.current.view?.openConversation?.();
      }
    } catch (cause) {
      if (currentSecretary.current.token === token) {
        setSecretaryWriteFailure({ token, operation: "send",
          sessionId: original.sessionId, body: original.body,
          text: cause instanceof Error ? cause.message : String(cause) });
        void currentSecretary.current.view?.openConversation?.();
      }
    } finally {
      secretaryWritePendingRef.current = false;
      setSecretaryWritePending(false);
    }
  };
  const secretaryEvidenceNode = secretaryActive ? <>
    {secretaryView?.historyGap ? <div role="status">对话读取未完整：{secretaryView.historyGap}</div> : null}
    {!nativeInput && secretaryDraft ? <div role="status">此秘书长会话的草稿已保留：<pre>{secretaryDraft}</pre></div> : null}
    {secretaryView?.statuses?.length ? <details>
      <summary>宿主原始轮次状态</summary>
      <pre>{secretaryView.statuses.join("\n")}</pre>
    </details> : null}
    {secretaryView?.inputRowsEnded ? <details>
      <summary>H 原始用户输入记录（{secretaryView.originalInputs?.length ?? 0}）</summary>
      <div>本次已读完 H 存储行；这不表示轮次或回复已完成。</div>
      <ol>{secretaryView.originalInputs?.map((input) => <li key={`${input.generation}:${input.requestId}`}>
        <div>请求 {input.requestId} · H 代 {input.generation} · {input.operation}</div>
        <div>原文 {input.bodyState} · H 阶段 {input.phase} · 回执 {input.receiptStatus ?? "未取得"}
          {input.occurredAtMs ? ` · 原始用户时间 ${input.occurredAtMs} ms` : ""}</div>
        {input.bodyState === "VERIFIED" && input.body !== null ? <pre>{input.body}</pre> : null}
        {input.receiptState ? <div>回执正文 {input.receiptState}</div> : null}
        {input.turnId ? <div>原始轮次 {input.turnId}</div> : null}
      </li>)}</ol>
    </details> : null}
    {secretaryView?.vendorUserFacts?.length ? <details>
      <summary>A 供应商 USER 回显（非 USER 授权事实，{secretaryView.vendorUserFacts.length} 项）</summary>
      <ol>{secretaryView.vendorUserFacts.map((fact) => <li key={fact.sourceEventId}>
        <div>来源 {fact.sourceEventId} · 轮次 {fact.turnId ?? "未提供"} ·
          {fact.matchedOriginal ? "与已验证 H 原文一致" : "未与已验证 H 原文对应"}</div>
        <pre>{fact.text}</pre>
      </li>)}</ol>
    </details> : null}
    {secretaryView?.outbound?.map((fact) => <div role="status" key={fact.requestId}>
      <div>原始会话 {fact.sessionId} · 席位 {fact.seatId} · H 代 {fact.hGeneration}</div>
      {fact.status === "REJECTED" ? "宿主已明确拒绝此原始请求；原文与回执已保留。" : fact.operation === "send"
        ? fact.status === "ACCEPTED"
          ? fact.inputVerified
            ? "原始 H 用户正文与发送回执已核实。"
            : "原始 H 发送回执已接受；用户正文尚未从持久来源核实，草稿仍保留。"
          : "原始 H 发送结果未确认；未创建第二个发送请求。"
        : fact.status === "ACCEPTED"
          ? "原始 H 停止事实已确认。"
          : "原始 H 停止结果未确认。"}
      {fact.body ? <pre>{fact.body}</pre> : null}
      {fact.reason ? <div>{fact.reason}</div> : null}
      {fact.receipt ? <details><summary>原始回执</summary>
        <pre>{JSON.stringify(fact.receipt)}</pre></details> : null}
      {fact.operation === "send" && fact.status === "UNKNOWN" && secretaryView.retrySend
        ? <button type="button" disabled={secretaryWritePending}
          onClick={() => void retrySecretarySend(fact.requestId)}>核对原始发送请求</button> : null}
      {fact.operation === "stop" && fact.status === "UNKNOWN"
        && secretaryView.retryStop ? <button type="button" disabled={secretaryWritePending}
          onClick={() => void retrySecretaryStop(fact.requestId)}>核对原始停止请求</button> : null}
    </div>)}
    {secretaryWriteFailure ? <div role="alert">
      {secretaryWriteFailure.sessionId ? `会话 ${secretaryWriteFailure.sessionId}：` : ""}{secretaryWriteFailure.text}
      {secretaryWriteFailure.body && secretaryWriteFailure.token !== secretaryToken
        ? <pre>{secretaryWriteFailure.body}</pre> : null}
    </div> : null}
  </> : null;
  const secretaryComposerNode = secretaryActive && secretaryHasAssociation
    ? nativeInput ? <footer className="composer">
      <ComposerInput text={secretaryDraft} disabled={secretaryWritePending}
        placeholder="向秘书长发送消息…" disabledPlaceholder="秘书长操作正在确认…"
        sendLabel="发送给秘书长" canStop={nativeInput.canStop && !secretaryWritePending}
        canSend={nativeInput.canSend && !secretarySendBlocked && !secretaryWritePending && Boolean(secretaryDraft.trim())}
        isProcessing={nativeInput.canStop} onStop={() => void stopSecretary()}
        onSend={() => void sendSecretary()}
        onTextChange={(text) => {
          if (!secretaryKey) return;
          setSecretaryDrafts((previous) => ({ ...previous,
            [secretaryKey]: { text, revision: (previous[secretaryKey]?.revision ?? 0) + 1 },
          }));
        }}
        onSelectionChange={() => {}}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            void sendSecretary();
          }
        }}
        textareaRef={secretaryTextareaRef} suggestionsOpen={false} suggestions={[]}
        highlightIndex={0} onHighlightIndex={() => {}} onSelectSuggestion={() => {}} />
      <div className="composer-meta" role="status">
        实例 {nativeInput.instanceId} · 模型 {nativeInput.model} · 强度 {nativeInput.effort} · 权限档位 {nativeInput.permissionTier}
      </div>
      {secretaryEvidenceNode}
    </footer> : secretaryView?.conversation?.composer ? composerNode
      : <div role="status">{secretaryView?.readState === "frozen"
      ? secretaryView.readError ?? "秘书长最新状态未读到"
      : entryLine(secretaryView!.entry).text}。当前会话只可查看。{secretaryEvidenceNode}</div>
    : secretaryActive ? secretaryEvidenceNode : composerNode;
  const compactThreadConnectionState: "live" | "polling" | "disconnected" =
    !activeWorkspace?.connected
      ? "disconnected"
      : remoteThreadConnectionState;
  const mainAppShellProps = useMainAppShellProps({
    shell: {
      appClassName,
      isResizing,
      appStyle,
      appRef,
      sidebarToggleProps,
      shouldLoadGitHubPanelData,
      appModalsProps,
      showMobileSetupWizard,
      mobileSetupWizardProps,
    },
    gitHubPanelDataProps: {
      activeWorkspace,
      gitPanelMode,
      shouldLoadDiffs,
      diffSource,
      selectedPullRequestNumber: selectedPullRequest?.number ?? null,
      onIssuesChange: handleGitIssuesChange,
      onPullRequestsChange: handleGitPullRequestsChange,
      onPullRequestDiffsChange: handleGitPullRequestDiffsChange,
      onPullRequestCommentsChange: handleGitPullRequestCommentsChange,
    },
    appLayout: {
      isPhone,
      isTablet,
      showHome: !secretaryActive && showHome,
      showGitDetail: !secretaryActive && showGitDetail,
      activeTab,
      tabletTab,
      centerMode: secretaryActive ? "chat" : centerMode,
      preloadGitDiffs: !secretaryActive && appSettings.preloadGitDiffs,
      splitChatDiffView: !secretaryActive && appSettings.splitChatDiffView,
      hasActivePlan: !secretaryActive && hasActivePlan,
      activeWorkspace: secretaryActive || Boolean(activeWorkspace),
      sidebarNode,
      messagesNode: mainMessagesNode,
      composerNode: <><NowPinSlot />{secretaryComposerNode}</>,
      approvalToastsNode,
      updateToastNode,
      errorToastsNode,
      homeNode,
      mainHeaderNode: secretaryActive ? null : mainHeaderNode,
      tabletNavNode,
      tabBarNode,
      gitDiffPanelNode: secretaryActive && secretaryPanelSource
        ? <SecretaryPanel source={secretaryPanelSource} /> : gitDiffPanelNode,
      gitDiffViewerNode: secretaryActive ? null : gitDiffViewerNode,
      planPanelNode: secretaryActive ? null : planPanelNode,
      debugPanelNode,
      debugPanelFullNode,
      terminalDockNode: secretaryActive ? null : terminalDockNode,
      compactEmptyCodexNode,
      compactEmptyGitNode,
      compactGitBackNode,
      onSidebarResizeStart,
      onChatDiffSplitPositionResizeStart,
      onRightPanelResizeStart,
      onPlanPanelResizeStart,
    },
    topbar: {
      isCompact,
      desktopTopbarLeftNode: secretaryActive ? null : desktopTopbarLeftNode,
      hasActiveWorkspace: Boolean(activeWorkspace),
      backendMode: appSettings.backendMode,
      remoteThreadConnectionState: compactThreadConnectionState,
    },
  });

  return (
    <NowProvider source={nowSource} active={
      secretaryActive && secretaryHasAssociation && composerNode && secretaryView!.association!.workspaceId
        ? { workspaceId: secretaryView!.association!.workspaceId, threadId: secretaryView!.association!.threadId }
        : !isNewAgentDraftMode && composerNode && activeWorkspaceId && activeThreadId
        ? { workspaceId: activeWorkspaceId, threadId: activeThreadId } : null
    }>
      <MainAppShell {...mainAppShellProps} />
    </NowProvider>
  );
}
