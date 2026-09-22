import { useCallback, useEffect, useRef } from "react";
import type { Dispatch } from "react";
import {
  loadDetachedReviewLinks,
  saveDetachedReviewLinks,
} from "@threads/utils/threadStorage";
import type { ThreadAction, ThreadState } from "./useThreadsReducer";

type UseDetachedReviewTrackingArgs = {
  activeThreadId: string | null;
  dispatch: Dispatch<ThreadAction>;
  recordThreadActivity: (
    workspaceId: string,
    threadId: string,
    timestamp?: number,
  ) => void;
  safeMessageActivity: () => void;
  threadsByWorkspace: ThreadState["threadsByWorkspace"];
  threadParentById: ThreadState["threadParentById"];
  updateThreadParent: (parentId: string, childIds: string[]) => void;
};

export function useDetachedReviewTracking({
  activeThreadId,
  dispatch,
  recordThreadActivity,
  safeMessageActivity,
  threadsByWorkspace,
  threadParentById,
  updateThreadParent,
}: UseDetachedReviewTrackingArgs) {
  const detachedReviewStartedNoticeRef = useRef<Set<string>>(new Set());
  const detachedReviewCompletedNoticeRef = useRef<Set<string>>(new Set());
  const detachedReviewParentByChildRef = useRef<Record<string, string>>({});
  const detachedReviewLinksByWorkspaceRef = useRef(loadDetachedReviewLinks());

  const registerDetachedReviewChild = useCallback(
    (workspaceId: string, parentId: string, childId: string) => {
      if (!workspaceId || !parentId || !childId || parentId === childId) {
        return;
      }
      detachedReviewParentByChildRef.current[childId] = parentId;
      const existingWorkspaceLinks =
        detachedReviewLinksByWorkspaceRef.current[workspaceId] ?? {};
      if (existingWorkspaceLinks[childId] !== parentId) {
        const nextLinksByWorkspace = {
          ...detachedReviewLinksByWorkspaceRef.current,
          [workspaceId]: {
            ...existingWorkspaceLinks,
            [childId]: parentId,
          },
        };
        detachedReviewLinksByWorkspaceRef.current = nextLinksByWorkspace;
        saveDetachedReviewLinks(nextLinksByWorkspace);
      }

      const timestamp = Date.now();
      recordThreadActivity(workspaceId, parentId, timestamp);
      dispatch({
        type: "setThreadTimestamp",
        workspaceId,
        threadId: parentId,
        timestamp,
      });

      const noticeKey = `${parentId}->${childId}`;
      if (!detachedReviewStartedNoticeRef.current.has(noticeKey)) {
        detachedReviewStartedNoticeRef.current.add(noticeKey);
        dispatch({
          type: "addAssistantMessage",
          threadId: parentId,
          text: `Detached review started. [Open review thread](/thread/${childId})`,
        });
      }

      if (parentId !== activeThreadId) {
        dispatch({ type: "markUnread", threadId: parentId, hasUnread: true });
      }
      safeMessageActivity();
    },
    [activeThreadId, dispatch, recordThreadActivity, safeMessageActivity],
  );

  useEffect(() => {
    const linksByWorkspace = detachedReviewLinksByWorkspaceRef.current;
    Object.entries(threadsByWorkspace).forEach(([workspaceId, threads]) => {
      const workspaceLinks = linksByWorkspace[workspaceId];
      if (!workspaceLinks) {
        return;
      }
      const threadIds = new Set(threads.map((thread) => thread.id));
      Object.entries(workspaceLinks).forEach(([childId, parentId]) => {
        if (!childId || !parentId || childId === parentId) {
          return;
        }
        if (!threadIds.has(childId) || !threadIds.has(parentId)) {
          return;
        }
        if (threadParentById[childId]) {
          return;
        }
        updateThreadParent(parentId, [childId]);
      });
    });
  }, [threadParentById, threadsByWorkspace, updateThreadParent]);

  const handleReviewExited = useCallback(
    (workspaceId: string, threadId: string) => {
      const parentId = detachedReviewParentByChildRef.current[threadId];
      if (!parentId) {
        return;
      }
      delete detachedReviewParentByChildRef.current[threadId];

      const timestamp = Date.now();
      recordThreadActivity(workspaceId, parentId, timestamp);
      dispatch({
        type: "setThreadTimestamp",
        workspaceId,
        threadId: parentId,
        timestamp,
      });
      const noticeKey = `${parentId}->${threadId}`;
      const alreadyNotified = detachedReviewCompletedNoticeRef.current.has(noticeKey);
      if (!alreadyNotified) {
        detachedReviewCompletedNoticeRef.current.add(noticeKey);
        dispatch({
          type: "addAssistantMessage",
          threadId: parentId,
          text: `Detached review completed. [Open review thread](/thread/${threadId})`,
        });
      }
      if (parentId !== activeThreadId) {
        dispatch({ type: "markUnread", threadId: parentId, hasUnread: true });
      }
      safeMessageActivity();
    },
    [activeThreadId, dispatch, recordThreadActivity, safeMessageActivity],
  );

  return {
    registerDetachedReviewChild,
    handleReviewExited,
  };
}
