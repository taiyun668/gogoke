import { useCallback, useRef } from "react";
import type { Dispatch, MutableRefObject } from "react";
import type {
  DebugEntry,
  ConversationItem,
  ThreadListSortKey,
  ThreadSummary,
  WorkspaceInfo,
} from "@/types";
import {
  archiveThread as archiveThreadService,
  forkThread as forkThreadService,
  listThreads as listThreadsService,
  listWorkspaces as listWorkspacesService,
  resumeThread as resumeThreadService,
  readThread as readThreadService,
  nativeConversationAssociation,
  startThread as startThreadService,
} from "@services/tauri";
import {
  buildItemsFromThread,
  getThreadTimestamp,
} from "@utils/threadItems";
import { extractThreadCodexMetadata } from "@threads/utils/threadCodexMetadata";
import {
  buildThreadSummaryFromThread,
  extractThreadFromResponse,
} from "@threads/utils/threadSummary";
import { asString } from "@threads/utils/threadNormalize";
import {
  getParentThreadIdFromThread,
  shouldHideSubagentThreadFromSidebar,
} from "@threads/utils/threadRpc";
import { saveThreadActivity } from "@threads/utils/threadStorage";
import {
  buildResumeHydrationPlan,
  buildWorkspacePathLookup,
  buildWorkspaceThreadListState,
  getThreadListNextCursor,
  resolveWorkspaceIdForThreadPath,
} from "@threads/utils/threadActionHelpers";
import type { ThreadAction, ThreadState } from "./useThreadsReducer";

const THREAD_LIST_TARGET_COUNT = 20;
const THREAD_LIST_PAGE_SIZE = 100;
const THREAD_LIST_MAX_PAGES_OLDER = 6;
const THREAD_LIST_MAX_PAGES_DEFAULT = 6;
const THREAD_LIST_CURSOR_PAGE_START = "__gogoke_page_start__";

function nativeInterruptedItems(
  response: Record<string, unknown> | null,
  thread: Record<string, unknown>,
): ConversationItem[] | null {
  const result = (response?.result ?? response) as Record<string, unknown> | null;
  const history = result?.nativeHistory as Record<string, unknown> | undefined;
  if (!history || history.state !== "COMPLETE") {
    throw new Error("Original native history is not completely source-qualified.");
  }
  const partials = history.partialMessages;
  if (partials === undefined) return null;
  if (!Array.isArray(partials)) throw new Error("Original interrupted partial metadata is invalid.");
  if (partials.length === 0) return null;
  const number = (value: unknown): bigint => {
    if (typeof value !== "string" || !/^[1-9][0-9]*$/.test(value) || value.length > 20) {
      throw new Error("Original partial source number is not canonical.");
    }
    const n = BigInt(value);
    if (n > 18446744073709551615n) throw new Error("Original partial source number is outside u64.");
    return n;
  };
  const high = number(history.highWater);
  const source = (value: unknown) => {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Original partial source is missing.");
    const fields = value as Record<string, unknown>;
    const names = ["operationId", "generation", "sourceEpoch", "sourceCursor", "rawSourceId"];
    if (Object.keys(fields).length !== names.length || names.some((name) => typeof fields[name] !== "string" || fields[name] === "")) {
      throw new Error("Original partial source identity is invalid.");
    }
    number(fields.generation);
    const cursor = number(fields.sourceCursor);
    const raw = number(fields.rawSourceId);
    if (raw > high) throw new Error("Original partial source exceeds the history high-water.");
    return { stream: JSON.stringify(names.slice(0, 3).map((name) => fields[name])),
      identity: JSON.stringify(names.map((name) => fields[name])), cursor, raw };
  };
  if (!Array.isArray(history.sourceRefs)) throw new Error("Original partial source pool is missing.");
  const pool = new Map<string, string>();
  for (const value of history.sourceRefs) {
    const ref = source(value);
    if (pool.has(ref.raw.toString())) throw new Error("Original history source pool repeats a raw source.");
    pool.set(ref.raw.toString(), ref.identity);
  }
  const turns = thread.turns as Record<string, unknown>[];
  const seen = new Set<string>();
  const byTurn = new Map<string, { index: number; first: bigint; item: ConversationItem }[]>();
  const vendorIds = new Set(turns.flatMap((turn) => (turn.items as Record<string, unknown>[]).map((item) => item.id)));
  for (const value of partials) {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Original partial message is invalid.");
    const partial = value as Record<string, unknown>;
    if (Object.keys(partial).length !== 6 || partial.kind !== "interruptedAgentMessage" ||
      typeof partial.turnId !== "string" || !partial.turnId || typeof partial.itemId !== "string" || !partial.itemId ||
      typeof partial.text !== "string" || !Array.isArray(partial.sourceRefs) || partial.sourceRefs.length < 2) {
      throw new Error("Original partial message identity or source is invalid.");
    }
    const matching = turns.filter((turn) => turn.id === partial.turnId);
    if (matching.length !== 1 || matching[0].status !== "interrupted" || matching[0].itemsView !== "notLoaded" ||
      !Array.isArray(matching[0].items) || matching[0].items.some((item) => (item as Record<string, unknown>).id === partial.itemId) ||
      typeof partial.itemIndex !== "number" || !Number.isSafeInteger(partial.itemIndex) ||
      partial.itemIndex < 0 || partial.itemIndex > matching[0].items.length) {
      throw new Error("Original partial conflicts with its turn, final item or position.");
    }
    const id = `gogoke-partial:${JSON.stringify([thread.id, partial.turnId, partial.itemId])}`;
    if (seen.has(id) || vendorIds.has(id)) throw new Error("Original partial message is repeated.");
    seen.add(id);
    const refs = partial.sourceRefs.map(source);
    refs.forEach((ref, index) => {
      if (pool.get(ref.raw.toString()) !== ref.identity || (index > 0 &&
        (ref.stream !== refs[index - 1].stream || ref.cursor <= refs[index - 1].cursor || ref.raw <= refs[index - 1].raw))) {
        throw new Error("Original partial source order or source-pool membership is invalid.");
      }
    });
    const group = byTurn.get(partial.turnId) ?? [];
    group.push({ index: partial.itemIndex, first: refs[0].raw,
      item: { id, kind: "message", role: "assistant",
        text: partial.text ? `中断时的部分输出（未收到最终消息）\n\n${partial.text}` : "中断前尚未收到文本（未收到最终消息）" } });
    byTurn.set(partial.turnId, group);
  }
  const items: ConversationItem[] = [];
  for (const turn of turns) {
    const group = byTurn.get(String(turn.id)) ?? [];
    group.sort((a, b) => a.index - b.index || (a.first < b.first ? -1 : a.first > b.first ? 1 : 0));
    const original = turn.items as Record<string, unknown>[];
    for (let index = 0; index <= original.length; index += 1) {
      items.push(...group.filter((partial) => partial.index === index).map((partial) => partial.item));
      if (index < original.length) items.push(...buildItemsFromThread({ turns: [{ items: [original[index]] }] }));
    }
  }
  return items;
}

type UseThreadActionsOptions = {
  dispatch: Dispatch<ThreadAction>;
  itemsByThread: ThreadState["itemsByThread"];
  threadsByWorkspace: ThreadState["threadsByWorkspace"];
  activeThreadIdByWorkspace: ThreadState["activeThreadIdByWorkspace"];
  activeTurnIdByThread: ThreadState["activeTurnIdByThread"];
  threadParentById: ThreadState["threadParentById"];
  threadListCursorByWorkspace: ThreadState["threadListCursorByWorkspace"];
  threadStatusById: ThreadState["threadStatusById"];
  threadSortKey: ThreadListSortKey;
  onDebug?: (entry: DebugEntry) => void;
  getThreadProjectionRevision?: (threadId: string) => number;
  getCustomName: (workspaceId: string, threadId: string) => string | undefined;
  threadActivityRef: MutableRefObject<Record<string, Record<string, number>>>;
  loadedThreadsRef: MutableRefObject<Record<string, boolean>>;
  replaceOnResumeRef: MutableRefObject<Record<string, boolean>>;
  applyCollabThreadLinksFromThread: (
    workspaceId: string,
    threadId: string,
    thread: Record<string, unknown>,
  ) => void;
  updateThreadParent: (parentId: string, childIds: string[]) => void;
  onSubagentThreadDetected: (workspaceId: string, threadId: string) => void;
  onThreadCodexMetadataDetected?: (
    workspaceId: string,
    threadId: string,
    metadata: { modelId: string | null; effort: string | null },
  ) => void;
};

export function useThreadActions({
  dispatch,
  itemsByThread,
  threadsByWorkspace,
  activeThreadIdByWorkspace,
  activeTurnIdByThread,
  threadParentById,
  threadListCursorByWorkspace,
  threadStatusById,
  threadSortKey,
  onDebug,
  getThreadProjectionRevision,
  getCustomName,
  threadActivityRef,
  loadedThreadsRef,
  replaceOnResumeRef,
  applyCollabThreadLinksFromThread,
  updateThreadParent,
  onSubagentThreadDetected,
  onThreadCodexMetadataDetected,
}: UseThreadActionsOptions) {
  const resumeInFlightByThreadRef = useRef<Record<string, number>>({});
  const nativeHydratedAssociationRef = useRef<Record<string, string>>({});
  const nativeReadInvocationRef = useRef<Record<string, number>>({});
  const nativeReadSequenceRef = useRef<Record<string, number>>({});
  const threadStatusByIdRef = useRef(threadStatusById);
  const activeTurnIdByThreadRef = useRef(activeTurnIdByThread);
  threadStatusByIdRef.current = threadStatusById;
  activeTurnIdByThreadRef.current = activeTurnIdByThread;

  const applyThreadMetadata = useCallback(
    (
      workspaceId: string,
      threadId: string,
      thread: Record<string, unknown>,
      options?: { notifySubagent?: boolean },
    ) => {
      const codexMetadata = extractThreadCodexMetadata(thread);
      if (codexMetadata.modelId || codexMetadata.effort) {
        onThreadCodexMetadataDetected?.(workspaceId, threadId, codexMetadata);
      }
      const sourceParentId = getParentThreadIdFromThread(thread);
      if (sourceParentId) {
        updateThreadParent(sourceParentId, [threadId]);
        if (options?.notifySubagent) {
          onSubagentThreadDetected(workspaceId, threadId);
        }
      }
    },
    [
      onSubagentThreadDetected,
      onThreadCodexMetadataDetected,
      updateThreadParent,
    ],
  );

  const dispatchPreviewMessage = useCallback(
    (threadId: string, text: string, timestamp: number) => {
      dispatch({
        type: "setLastAgentMessage",
        threadId,
        text,
        timestamp,
      });
    },
    [dispatch],
  );

  const extractThreadId = useCallback(
    (response: Record<string, unknown> | null | undefined) => {
      const thread = extractThreadFromResponse(response);
      return String(thread?.id ?? "");
    },
    [],
  );

  const startThreadForWorkspace = useCallback(
    async (workspaceId: string, options?: { activate?: boolean }) => {
      const shouldActivate = options?.activate !== false;
      onDebug?.({
        id: `${Date.now()}-client-thread-start`,
        timestamp: Date.now(),
        source: "client",
        label: "thread/start",
        payload: { workspaceId },
      });
      try {
        const response = await startThreadService(workspaceId);
        onDebug?.({
          id: `${Date.now()}-server-thread-start`,
          timestamp: Date.now(),
          source: "server",
          label: "thread/start response",
          payload: response,
        });
        const threadId = extractThreadId(response);
        if (threadId) {
          dispatch({ type: "ensureThread", workspaceId, threadId });
          if (shouldActivate) {
            dispatch({ type: "setActiveThreadId", workspaceId, threadId });
          }
          loadedThreadsRef.current[threadId] = true;
          return threadId;
        }
        return null;
      } catch (error) {
        onDebug?.({
          id: `${Date.now()}-client-thread-start-error`,
          timestamp: Date.now(),
          source: "error",
          label: "thread/start error",
          payload: error instanceof Error ? error.message : String(error),
        });
        throw error;
      }
    },
    [dispatch, extractThreadId, loadedThreadsRef, onDebug],
  );

  const resumeThreadForWorkspace = useCallback(
    async (
      workspaceId: string,
      threadId: string,
      force = false,
      replaceLocal = false,
    ) => {
      if (!threadId) {
        return null;
      }
      // Preserve invocation order across the asynchronous attachment lookup.
      const readSequence = (nativeReadInvocationRef.current[threadId] ?? 0) + 1;
      nativeReadInvocationRef.current[threadId] = readSequence;
      let native: Awaited<ReturnType<typeof nativeConversationAssociation>>;
      try {
        native = "__TAURI_INTERNALS__" in window
          ? await nativeConversationAssociation(workspaceId) : null;
      } catch (error) {
        dispatch({ type: "addAssistantMessage", threadId,
          text: error instanceof Error ? error.message : String(error) });
        onDebug?.({ id: `${Date.now()}-native-read-association-error`, timestamp: Date.now(),
          source: "error", label: "Native conversation association read",
          payload: error instanceof Error ? error.message : String(error) });
        return null;
      }
      if (!native && !force && loadedThreadsRef.current[threadId]) {
        return threadId;
      }
      const nativeKey = `${workspaceId}:${threadId}`;
      if (native && !force && loadedThreadsRef.current[threadId] &&
          nativeHydratedAssociationRef.current[nativeKey] === JSON.stringify(native)) {
        return threadId;
      }
      const status = threadStatusByIdRef.current[threadId];
      if (!native && status?.isProcessing && loadedThreadsRef.current[threadId] && !force) {
        onDebug?.({
          id: `${Date.now()}-client-thread-resume-skipped`,
          timestamp: Date.now(),
          source: "client",
          label: "thread/resume skipped",
          payload: { workspaceId, threadId, reason: "active-turn" },
        });
        return threadId;
      }
      // Cache hits and failed lookups must not supersede an in-flight read.
      if ((nativeReadSequenceRef.current[threadId] ?? 0) > readSequence) {
        return null;
      }
      nativeReadSequenceRef.current[threadId] = readSequence;
      onDebug?.({
        id: `${Date.now()}-client-thread-resume`,
        timestamp: Date.now(),
        source: "client",
        label: native ? "thread/read" : "thread/resume",
        payload: { workspaceId, threadId },
      });
      const inFlightCount =
        (resumeInFlightByThreadRef.current[threadId] ?? 0) + 1;
      resumeInFlightByThreadRef.current[threadId] = inFlightCount;
      if (inFlightCount === 1) {
        dispatch({ type: "setThreadResumeLoading", threadId, isLoading: true });
      }
      const projectionRevision = getThreadProjectionRevision?.(threadId);
      try {
        if (native && projectionRevision === undefined) {
          throw new Error("Native full history reconciliation has no projection ordering source.");
        }
        const response =
          (await (native ? readThreadService(workspaceId, threadId)
                         : resumeThreadService(workspaceId, threadId))) as
            | Record<string, unknown>
            | null;
        onDebug?.({
          id: `${Date.now()}-server-thread-resume`,
          timestamp: Date.now(),
          source: "server",
          label: native ? "thread/read response" : "thread/resume response",
          payload: response,
        });
        const thread = extractThreadFromResponse(response);
        if (native && (!thread || thread.id !== threadId || !Array.isArray(thread.turns))) {
          throw new Error("Original native thread/read did not return the selected thread and full turns.");
        }
        if (native) {
          const current = await nativeConversationAssociation(workspaceId);
          if (JSON.stringify(current) !== JSON.stringify(native)) {
            throw new Error("The original native attachment changed during full history reconciliation.");
          }
          if (nativeReadSequenceRef.current[threadId] !== readSequence) return null;
          if (getThreadProjectionRevision?.(threadId) !== projectionRevision) {
            throw new Error("Newer conversation facts superseded this native history snapshot; full reconciliation remains incomplete.");
          }
        }
        const interruptedItems = native && thread ? nativeInterruptedItems(response, thread) : null;
        if (thread) {
          dispatch({ type: "ensureThread", workspaceId, threadId });
          applyThreadMetadata(workspaceId, threadId, thread, {
            notifySubagent: true,
          });
          applyCollabThreadLinksFromThread(workspaceId, threadId, thread);
          const localItems = itemsByThread[threadId] ?? [];
          const shouldReplace =
            Boolean(native) || replaceLocal || replaceOnResumeRef.current[threadId] === true;
          if (shouldReplace) {
            replaceOnResumeRef.current[threadId] = false;
          }
          const hydrationPlan = buildResumeHydrationPlan({
            thread,
            workspaceId,
            threadId,
            replaceLocal: shouldReplace,
            localItems: native ? [] : localItems,
            localStatus: threadStatusByIdRef.current[threadId],
            localActiveTurnId: activeTurnIdByThreadRef.current[threadId] ?? null,
            getCustomName,
          });
          if (interruptedItems) hydrationPlan.mergedItems = interruptedItems;
          if (!hydrationPlan.shouldHydrate) {
            loadedThreadsRef.current[threadId] = true;
            return threadId;
          }
          if (hydrationPlan.keepLocalProcessing) {
            onDebug?.({
              id: `${Date.now()}-client-thread-resume-keep-processing`,
              timestamp: Date.now(),
              source: "client",
              label: "thread/resume keep-processing",
              payload: { workspaceId, threadId },
            });
          }
          dispatch({
            type: "markProcessing",
            threadId,
            isProcessing: hydrationPlan.shouldMarkProcessing,
            timestamp: hydrationPlan.processingTimestamp,
          });
          dispatch({
            type: "setActiveTurnId",
            threadId,
            turnId: hydrationPlan.resumedActiveTurnId,
          });
          dispatch({
            type: "markReviewing",
            threadId,
            isReviewing: hydrationPlan.reviewing,
          });
          if (native || hydrationPlan.mergedItems.length > 0) {
            dispatch({
              type: "setThreadItems",
              threadId,
              items: hydrationPlan.mergedItems,
            });
          }
          if (hydrationPlan.threadName) {
            dispatch({
              type: "setThreadName",
              workspaceId,
              threadId,
              name: hydrationPlan.threadName,
            });
          }
          if (
            hydrationPlan.lastMessageText &&
            hydrationPlan.lastMessageTimestamp !== null
          ) {
            dispatchPreviewMessage(
              threadId,
              hydrationPlan.lastMessageText,
              hydrationPlan.lastMessageTimestamp,
            );
          }
        }
        loadedThreadsRef.current[threadId] = true;
        if (native) nativeHydratedAssociationRef.current[nativeKey] = JSON.stringify(native);
        return threadId;
      } catch (error) {
        if (native && nativeReadSequenceRef.current[threadId] !== readSequence) return null;
        if (native) {
          loadedThreadsRef.current[threadId] = false;
          delete nativeHydratedAssociationRef.current[nativeKey];
          dispatch({ type: "addAssistantMessage", threadId,
            text: error instanceof Error ? error.message : String(error) });
        }
        onDebug?.({
          id: `${Date.now()}-client-thread-resume-error`,
          timestamp: Date.now(),
          source: "error",
          label: native ? "thread/read error" : "thread/resume error",
          payload: error instanceof Error ? error.message : String(error),
        });
        return null;
      } finally {
        const nextCount = Math.max(
          0,
          (resumeInFlightByThreadRef.current[threadId] ?? 1) - 1,
        );
        if (nextCount === 0) {
          delete resumeInFlightByThreadRef.current[threadId];
          dispatch({ type: "setThreadResumeLoading", threadId, isLoading: false });
        } else {
          resumeInFlightByThreadRef.current[threadId] = nextCount;
        }
      }
    },
    [
      applyThreadMetadata,
      applyCollabThreadLinksFromThread,
      dispatchPreviewMessage,
      dispatch,
      getCustomName,
      getThreadProjectionRevision,
      itemsByThread,
      loadedThreadsRef,
      onDebug,
      replaceOnResumeRef,
    ],
  );

  const forkThreadForWorkspace = useCallback(
    async (
      workspaceId: string,
      threadId: string,
      options?: { activate?: boolean },
    ) => {
      if (!threadId) {
        return null;
      }
      const shouldActivate = options?.activate !== false;
      onDebug?.({
        id: `${Date.now()}-client-thread-fork`,
        timestamp: Date.now(),
        source: "client",
        label: "thread/fork",
        payload: { workspaceId, threadId },
      });
      try {
        const response = await forkThreadService(workspaceId, threadId);
        onDebug?.({
          id: `${Date.now()}-server-thread-fork`,
          timestamp: Date.now(),
          source: "server",
          label: "thread/fork response",
          payload: response,
        });
        const forkedThreadId = extractThreadId(response);
        if (!forkedThreadId) {
          return null;
        }
        dispatch({ type: "ensureThread", workspaceId, threadId: forkedThreadId });
        if (shouldActivate) {
          dispatch({
            type: "setActiveThreadId",
            workspaceId,
            threadId: forkedThreadId,
          });
        }
        loadedThreadsRef.current[forkedThreadId] = false;
        await resumeThreadForWorkspace(workspaceId, forkedThreadId, true, true);
        return forkedThreadId;
      } catch (error) {
        onDebug?.({
          id: `${Date.now()}-client-thread-fork-error`,
          timestamp: Date.now(),
          source: "error",
          label: "thread/fork error",
          payload: error instanceof Error ? error.message : String(error),
        });
        return null;
      }
    },
    [
      dispatch,
      extractThreadId,
      loadedThreadsRef,
      onDebug,
      resumeThreadForWorkspace,
    ],
  );

  const refreshThread = useCallback(
    async (workspaceId: string, threadId: string) => {
      if (!threadId) {
        return null;
      }
      replaceOnResumeRef.current[threadId] = true;
      return resumeThreadForWorkspace(workspaceId, threadId, true, true);
    },
    [replaceOnResumeRef, resumeThreadForWorkspace],
  );

  const resetWorkspaceThreads = useCallback(
    (workspaceId: string) => {
      const threadIds = new Set<string>();
      const list = threadsByWorkspace[workspaceId] ?? [];
      list.forEach((thread) => threadIds.add(thread.id));
      const activeThread = activeThreadIdByWorkspace[workspaceId];
      if (activeThread) {
        threadIds.add(activeThread);
      }
      threadIds.forEach((threadId) => {
        loadedThreadsRef.current[threadId] = false;
        const resetSequence = Math.max(
          nativeReadInvocationRef.current[threadId] ?? 0,
          nativeReadSequenceRef.current[threadId] ?? 0,
        ) + 1;
        nativeReadInvocationRef.current[threadId] = resetSequence;
        nativeReadSequenceRef.current[threadId] = resetSequence;
        delete nativeHydratedAssociationRef.current[`${workspaceId}:${threadId}`];
      });
    },
    [activeThreadIdByWorkspace, loadedThreadsRef, threadsByWorkspace],
  );

  const buildThreadSummary = useCallback(
    (
      workspaceId: string,
      thread: Record<string, unknown>,
      fallbackIndex: number,
    ): ThreadSummary | null =>
      buildThreadSummaryFromThread({
        workspaceId,
        thread,
        fallbackIndex,
        getCustomName,
      }),
    [getCustomName],
  );

  const listThreadsForWorkspaces = useCallback(
    async (
      workspaces: WorkspaceInfo[],
      options?: {
        preserveState?: boolean;
        sortKey?: ThreadListSortKey;
        maxPages?: number;
      },
    ): Promise<void> => {
      const targets = workspaces.filter((workspace) => workspace.id);
      if (targets.length === 0) {
        return;
      }
      // Native lists are scoped to their original workspace, unlike the old
      // shared CLI index. Never clear another workspace from an unread list.
      if ("__TAURI_INTERNALS__" in window && targets.length > 1) {
        const scoped: WorkspaceInfo[] = [];
        const shared: WorkspaceInfo[] = [];
        for (const workspace of targets) {
          try {
            (await nativeConversationAssociation(workspace.id) ? scoped : shared).push(workspace);
          } catch (error) {
            // Keep unresolved transports isolated. The singleton call retains
            // its actual original error and cannot clear another workspace.
            onDebug?.({ id: `${Date.now()}-thread-list-transport-error`, timestamp: Date.now(),
              source: "error", label: "thread/list transport qualification",
              payload: { workspaceId: workspace.id,
                reason: error instanceof Error ? error.message : String(error) } });
            scoped.push(workspace);
          }
        }
        if (scoped.length > 0) {
          for (const workspace of scoped) await listThreadsForWorkspaces([workspace], options);
          if (shared.length > 0) await listThreadsForWorkspaces(shared, options);
          return;
        }
      }
      const preserveState = options?.preserveState ?? false;
      const requestedSortKey = options?.sortKey ?? threadSortKey;
      const maxPages = Math.max(1, options?.maxPages ?? THREAD_LIST_MAX_PAGES_DEFAULT);
      if (!preserveState) {
        targets.forEach((workspace) => {
          dispatch({
            type: "setThreadListLoading",
            workspaceId: workspace.id,
            isLoading: true,
          });
          dispatch({
            type: "setThreadListCursor",
            workspaceId: workspace.id,
            cursor: null,
          });
        });
      }
      onDebug?.({
        id: `${Date.now()}-client-thread-list`,
        timestamp: Date.now(),
        source: "client",
        label: "thread/list",
        payload: {
          workspaceIds: targets.map((workspace) => workspace.id),
          preserveState,
          maxPages,
        },
      });
      try {
        const requester = targets.find((workspace) => workspace.connected) ?? targets[0];
        const association = "__TAURI_INTERNALS__" in window
          ? await nativeConversationAssociation(requester.id) : null;
        const native = Boolean(association);
        if (native && targets.length > 1) {
          throw new Error("The workspace transport changed during shared list qualification; no lists were replaced.");
        }
        const assertCurrentAttachments = async () => {
          if (!("__TAURI_INTERNALS__" in window)) return;
          for (const workspace of targets) {
            const expected = workspace.id === requester.id ? association : null;
            if (JSON.stringify(await nativeConversationAssociation(workspace.id)) !== JSON.stringify(expected)) {
              throw new Error("An original workspace attachment changed during thread/list; no lists were replaced.");
            }
          }
        };
        await assertCurrentAttachments();
        const hiddenThreads: { workspaceId: string; threadId: string }[] = [];
        const matchingThreadsByWorkspace: Record<string, Record<string, unknown>[]> = {};
        let workspacePathLookup = buildWorkspacePathLookup(targets);
        const targetWorkspaceIds = new Set(targets.map((workspace) => workspace.id));
        try {
          const knownWorkspaces = await listWorkspacesService();
          if (knownWorkspaces.length > 0) {
            workspacePathLookup = buildWorkspacePathLookup([
              ...targets,
              ...knownWorkspaces,
            ]);
          }
        } catch {
          workspacePathLookup = buildWorkspacePathLookup(targets);
        }
        const uniqueThreadIdsByWorkspace: Record<string, Set<string>> = {};
        const resumeCursorByWorkspace: Record<string, string | null> = {};
        targets.forEach((workspace) => {
          matchingThreadsByWorkspace[workspace.id] = [];
          uniqueThreadIdsByWorkspace[workspace.id] = new Set<string>();
          resumeCursorByWorkspace[workspace.id] = null;
        });
        let pagesFetched = 0;
        let cursor: string | null = null;
        do {
          const pageCursor = cursor;
          pagesFetched += 1;
          const response =
            (await listThreadsService(
              requester.id,
              cursor,
              THREAD_LIST_PAGE_SIZE,
              native ? undefined : requestedSortKey,
            )) as Record<string, unknown>;
          onDebug?.({
            id: `${Date.now()}-server-thread-list`,
            timestamp: Date.now(),
            source: "server",
            label: "thread/list response",
            payload: response,
          });
          const result = (response.result ?? response) as Record<string, unknown>;
          await assertCurrentAttachments();
          if (native && (!result.nativeHistory || typeof result.nativeHistory !== "object")) {
            throw new Error("The native thread/list response has no original history provenance.");
          }
          const data = Array.isArray(result?.data)
            ? (result.data as Record<string, unknown>[])
            : [];
          const nextCursor = getThreadListNextCursor(result);
          data.forEach((thread) => {
            const workspaceId = native ? requester.id : resolveWorkspaceIdForThreadPath(
              String(thread?.cwd ?? ""),
              workspacePathLookup,
              targetWorkspaceIds,
            );
            if (!workspaceId) {
              return;
            }
            const threadId = String(thread?.id ?? "");
            if (threadId && shouldHideSubagentThreadFromSidebar(thread.source)) {
              hiddenThreads.push({ workspaceId, threadId });
              return;
            }
            matchingThreadsByWorkspace[workspaceId]?.push(thread);
            if (!threadId) {
              return;
            }
            const uniqueThreadIds = uniqueThreadIdsByWorkspace[workspaceId];
            if (!uniqueThreadIds || uniqueThreadIds.has(threadId)) {
              return;
            }
            uniqueThreadIds.add(threadId);
            if (
              uniqueThreadIds.size > THREAD_LIST_TARGET_COUNT &&
              resumeCursorByWorkspace[workspaceId] === null
            ) {
              resumeCursorByWorkspace[workspaceId] =
                pageCursor ?? THREAD_LIST_CURSOR_PAGE_START;
            }
          });
          cursor = nextCursor;
          if (pagesFetched >= maxPages) {
            break;
          }
        } while (cursor);

        const nextThreadActivity = { ...threadActivityRef.current };
        let didChangeAnyActivity = false;
        for (const workspace of targets) {
          // Finish this workspace's qualification immediately before its
          // synchronous projection; checking another target cannot age it.
          if ("__TAURI_INTERNALS__" in window) {
            const expected = workspace.id === requester.id ? association : null;
            if (JSON.stringify(await nativeConversationAssociation(workspace.id)) !== JSON.stringify(expected)) {
              throw new Error("This workspace attachment changed before its thread/list projection; its previous list was retained.");
            }
          }
          hiddenThreads.filter((row) => row.workspaceId === workspace.id)
            .forEach(({ workspaceId, threadId }) => dispatch({ type: "hideThread", workspaceId, threadId }));
          const matchingThreads = matchingThreadsByWorkspace[workspace.id] ?? [];
          const activityByThread = nextThreadActivity[workspace.id] ?? {};
          const threadListState = buildWorkspaceThreadListState({
            workspaceId: workspace.id,
            matchingThreads,
            activityByThread,
            requestedSortKey,
            buildThreadSummary,
            activeThreadId: activeThreadIdByWorkspace[workspace.id],
            existingThreadIds: (threadsByWorkspace[workspace.id] ?? []).map(
              (thread) => thread.id,
            ),
            threadStatusById,
            threadParentById,
            threadListTargetCount: THREAD_LIST_TARGET_COUNT,
          });
          threadListState.uniqueThreads.forEach((thread) => {
            const threadId = String(thread?.id ?? "");
            if (!threadId) {
              return;
            }
            applyThreadMetadata(workspace.id, threadId, thread, {
              notifySubagent: true,
            });
          });
          if (threadListState.didChangeActivity) {
            nextThreadActivity[workspace.id] = threadListState.nextActivityByThread;
            didChangeAnyActivity = true;
          }
          dispatch({
            type: "setThreads",
            workspaceId: workspace.id,
            threads: threadListState.summaries,
            sortKey: requestedSortKey,
            preserveAnchors: true,
          });
          dispatch({
            type: "setThreadListCursor",
            workspaceId: workspace.id,
            cursor: resumeCursorByWorkspace[workspace.id] ?? cursor,
          });
          threadListState.previewUpdates.forEach(({ threadId, text, timestamp }) => {
            dispatchPreviewMessage(threadId, text, timestamp);
          });
        }
        if (didChangeAnyActivity) {
          threadActivityRef.current = nextThreadActivity;
          saveThreadActivity(nextThreadActivity);
        }
      } catch (error) {
        onDebug?.({
          id: `${Date.now()}-client-thread-list-error`,
          timestamp: Date.now(),
          source: "error",
          label: "thread/list error",
          payload: error instanceof Error ? error.message : String(error),
        });
      } finally {
        if (!preserveState) {
          targets.forEach((workspace) => {
            dispatch({
              type: "setThreadListLoading",
              workspaceId: workspace.id,
              isLoading: false,
            });
          });
        }
      }
    },
    [
      applyThreadMetadata,
      buildThreadSummary,
      dispatchPreviewMessage,
      dispatch,
      onDebug,
      activeThreadIdByWorkspace,
      threadParentById,
      threadActivityRef,
      threadStatusById,
      threadSortKey,
      threadsByWorkspace,
    ],
  );

  const listThreadsForWorkspace = useCallback(
    async (
      workspace: WorkspaceInfo,
      options?: {
        preserveState?: boolean;
        sortKey?: ThreadListSortKey;
        maxPages?: number;
      },
    ) => {
      await listThreadsForWorkspaces([workspace], options);
    },
    [listThreadsForWorkspaces],
  );

  const loadOlderThreadsForWorkspace = useCallback(
    async (workspace: WorkspaceInfo) => {
      const requestedSortKey = threadSortKey;
      const cursorValue = threadListCursorByWorkspace[workspace.id] ?? null;
      if (!cursorValue) {
        return;
      }
      const nextCursor =
        cursorValue === THREAD_LIST_CURSOR_PAGE_START ? null : cursorValue;
      let workspacePathLookup = buildWorkspacePathLookup([workspace]);
      const allowedWorkspaceIds = new Set([workspace.id]);
      const existing = threadsByWorkspace[workspace.id] ?? [];
      dispatch({
        type: "setThreadListPaging",
        workspaceId: workspace.id,
        isLoading: true,
      });
      onDebug?.({
        id: `${Date.now()}-client-thread-list-older`,
        timestamp: Date.now(),
        source: "client",
        label: "thread/list older",
        payload: { workspaceId: workspace.id, cursor: cursorValue },
      });
      try {
        const association = "__TAURI_INTERNALS__" in window
          ? await nativeConversationAssociation(workspace.id) : null;
        const native = Boolean(association);
        try {
          const knownWorkspaces = await listWorkspacesService();
          if (knownWorkspaces.length > 0) {
            workspacePathLookup = buildWorkspacePathLookup([
              workspace,
              ...knownWorkspaces,
            ]);
          }
        } catch {
          workspacePathLookup = buildWorkspacePathLookup([workspace]);
        }
        const matchingThreads: Record<string, unknown>[] = [];
        const hiddenThreads: { workspaceId: string; threadId: string }[] = [];
        const maxPagesWithoutMatch = THREAD_LIST_MAX_PAGES_OLDER;
        let pagesFetched = 0;
        let cursor: string | null = nextCursor;
        do {
          pagesFetched += 1;
          const response =
            (await listThreadsService(
              workspace.id,
              cursor,
              THREAD_LIST_PAGE_SIZE,
              native ? undefined : requestedSortKey,
            )) as Record<string, unknown>;
          onDebug?.({
            id: `${Date.now()}-server-thread-list-older`,
            timestamp: Date.now(),
            source: "server",
            label: "thread/list older response",
            payload: response,
          });
          const result = (response.result ?? response) as Record<string, unknown>;
          if ("__TAURI_INTERNALS__" in window &&
              JSON.stringify(await nativeConversationAssociation(workspace.id)) !== JSON.stringify(association)) {
            throw new Error("The original workspace attachment changed during older thread/list; no lists were replaced.");
          }
          if (native && (!result.nativeHistory || typeof result.nativeHistory !== "object")) {
            throw new Error("The native older thread/list response has no original history provenance.");
          }
          const data = Array.isArray(result?.data)
            ? (result.data as Record<string, unknown>[])
            : [];
          const next = getThreadListNextCursor(result);
          matchingThreads.push(
            ...data.filter(
              (thread) => {
                const workspaceId = native ? workspace.id : resolveWorkspaceIdForThreadPath(
                  String(thread?.cwd ?? ""),
                  workspacePathLookup,
                  allowedWorkspaceIds,
                );
                if (workspaceId !== workspace.id) {
                  return false;
                }
                const threadId = String(thread?.id ?? "");
                if (threadId && shouldHideSubagentThreadFromSidebar(thread.source)) {
                  hiddenThreads.push({ workspaceId, threadId });
                  return false;
                }
                return true;
              },
            ),
          );
          cursor = next;
          if (matchingThreads.length === 0 && pagesFetched >= maxPagesWithoutMatch) {
            break;
          }
          if (pagesFetched >= THREAD_LIST_MAX_PAGES_OLDER) {
            break;
          }
        } while (cursor && matchingThreads.length < THREAD_LIST_TARGET_COUNT);

        hiddenThreads.forEach(({ workspaceId, threadId }) => dispatch({ type: "hideThread", workspaceId, threadId }));
        const existingIds = new Set(existing.map((thread) => thread.id));
        const additions: ThreadSummary[] = [];
        matchingThreads.forEach((thread) => {
          const id = String(thread?.id ?? "");
          if (!id || existingIds.has(id)) {
            return;
          }
          applyThreadMetadata(workspace.id, id, thread);
          const summary = buildThreadSummary(
            workspace.id,
            thread,
            existing.length + additions.length,
          );
          if (!summary) {
            return;
          }
          additions.push(summary);
          existingIds.add(id);
        });

        if (additions.length > 0) {
          dispatch({
            type: "setThreads",
            workspaceId: workspace.id,
            threads: [...existing, ...additions],
            sortKey: requestedSortKey,
          });
        }
        dispatch({
          type: "setThreadListCursor",
          workspaceId: workspace.id,
          cursor,
        });
        matchingThreads.forEach((thread) => {
          const threadId = String(thread?.id ?? "");
          const preview = asString(thread?.preview ?? "").trim();
          if (!threadId || !preview) {
            return;
          }
          dispatch({
            type: "setLastAgentMessage",
            threadId,
            text: preview,
            timestamp: getThreadTimestamp(thread),
          });
        });
      } catch (error) {
        onDebug?.({
          id: `${Date.now()}-client-thread-list-older-error`,
          timestamp: Date.now(),
          source: "error",
          label: "thread/list older error",
          payload: error instanceof Error ? error.message : String(error),
        });
      } finally {
        dispatch({
          type: "setThreadListPaging",
          workspaceId: workspace.id,
          isLoading: false,
        });
      }
    },
    [
      applyThreadMetadata,
      buildThreadSummary,
      dispatch,
      onDebug,
      threadListCursorByWorkspace,
      threadsByWorkspace,
      threadSortKey,
    ],
  );

  const archiveThread = useCallback(
    async (workspaceId: string, threadId: string) => {
      try {
        await archiveThreadService(workspaceId, threadId);
      } catch (error) {
        onDebug?.({
          id: `${Date.now()}-client-thread-archive-error`,
          timestamp: Date.now(),
          source: "error",
          label: "thread/archive error",
          payload: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [onDebug],
  );

  return {
    startThreadForWorkspace,
    forkThreadForWorkspace,
    resumeThreadForWorkspace,
    refreshThread,
    resetWorkspaceThreads,
    listThreadsForWorkspaces,
    listThreadsForWorkspace,
    loadOlderThreadsForWorkspace,
    archiveThread,
  };
}
