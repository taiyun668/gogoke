import { Fragment, memo, useCallback, useMemo, type ReactNode } from "react";
import { NowOutputSlot, useNowConversation } from "@/features/now/NowContext";
import { useI18n } from "@/i18n";
import ChevronDown from "lucide-react/dist/esm/icons/chevron-down";
import ChevronUp from "lucide-react/dist/esm/icons/chevron-up";
import type {
  ConversationItem,
  OpenAppTarget,
  RequestUserInputRequest,
  RequestUserInputResponse,
} from "../../../types";
import { PlanReadyFollowupMessage } from "../../app/components/PlanReadyFollowupMessage";
import { RequestUserInputMessage } from "../../app/components/RequestUserInputMessage";
import { useFileLinkOpener } from "../hooks/useFileLinkOpener";
import { buildToolGroups, parseReasoning, type MessageListEntry } from "../utils/messageRenderUtils";
import {
  DiffRow,
  ExploreRow,
  MessageRow,
  ReasoningRow,
  ReviewRow,
  ToolRow,
  UserInputRow,
  WorkingIndicator,
} from "./MessageRows";
import { useMessagesViewState } from "./useMessagesViewState";

type MessagesProps = {
  items: ConversationItem[];
  threadId: string | null;
  workspaceId?: string | null;
  isThinking: boolean;
  isLoadingMessages?: boolean;
  processingStartedAt?: number | null;
  lastDurationMs?: number | null;
  showPollingFetchStatus?: boolean;
  pollingIntervalMs?: number;
  workspacePath?: string | null;
  openTargets: OpenAppTarget[];
  selectedOpenAppId: string;
  codeBlockCopyUseModifier?: boolean;
  showMessageFilePath?: boolean;
  userInputRequests?: RequestUserInputRequest[];
  onUserInputSubmit?: (
    request: RequestUserInputRequest,
    response: RequestUserInputResponse,
  ) => void;
  onPlanAccept?: () => void;
  onPlanSubmitChanges?: (changes: string) => void;
  onOpenThreadLink?: (threadId: string, workspaceId?: string | null) => void;
  onQuoteMessage?: (text: string) => void;
  afterItem?: (itemId: string) => ReactNode;
};

export const Messages = memo(function Messages({
  items,
  threadId,
  workspaceId = null,
  isThinking,
  isLoadingMessages = false,
  processingStartedAt = null,
  lastDurationMs = null,
  showPollingFetchStatus = false,
  pollingIntervalMs = 12000,
  workspacePath = null,
  openTargets,
  selectedOpenAppId,
  codeBlockCopyUseModifier = false,
  showMessageFilePath = true,
  userInputRequests = [],
  onUserInputSubmit,
  onPlanAccept,
  onPlanSubmitChanges,
  onOpenThreadLink,
  onQuoteMessage,
  afterItem,
}: MessagesProps) {
  const { tx } = useI18n();
  const activeUserInputRequestId =
    threadId && userInputRequests.length
      ? (userInputRequests.find(
          (request) =>
            request.params.thread_id === threadId &&
            (!workspaceId || request.workspace_id === workspaceId),
        )?.request_id ?? null)
      : null;
  const { openFileLink, showFileLinkMenu } = useFileLinkOpener(
    workspacePath,
    openTargets,
    selectedOpenAppId,
  );
  const handleOpenThreadLink = useCallback(
    (threadId: string) => {
      onOpenThreadLink?.(threadId, workspaceId ?? null);
    },
    [onOpenThreadLink, workspaceId],
  );

  const hasActiveUserInputRequest = activeUserInputRequestId !== null;
  const hasVisibleUserInputRequest = hasActiveUserInputRequest && Boolean(onUserInputSubmit);
  const userInputNode =
    hasActiveUserInputRequest && onUserInputSubmit ? (
      <RequestUserInputMessage
        requests={userInputRequests}
        activeThreadId={threadId}
        activeWorkspaceId={workspaceId}
        onSubmit={onUserInputSubmit}
      />
    ) : null;
  const {
    bottomRef,
    containerRef,
    updateAutoScroll,
    requestAutoScroll,
    expandedItems,
    toggleExpanded,
    collapsedToolGroups,
    toggleToolGroup,
    copiedMessageId,
    handleCopyMessage,
    handleQuoteMessage,
    reasoningMetaById,
    latestReasoningLabel,
    groupedItems,
    planFollowup,
    dismissPlanFollowup,
  } = useMessagesViewState({
    items,
    threadId,
    isThinking,
    activeUserInputRequestId,
    hasVisibleUserInputRequest,
    onPlanAccept,
    onPlanSubmitChanges,
    onQuoteMessage,
  });

  const now = useNowConversation(workspaceId, threadId);
  const anchoredItems = useMemo(() => {
    const batches = now?.conversation.batches ?? [];
    const completeAnchor = (entry: (typeof batches)[number]) =>
      typeof entry.leadTurnId === "string" && entry.leadTurnId.length > 0 &&
      typeof entry.outputItemId === "string" && entry.outputItemId.length > 0;
    const anchors = new Set(batches.filter((entry) => completeAnchor(entry) &&
      batches.filter((candidate) => candidate.batch.id === entry.batch.id && completeAnchor(candidate)).length === 1,
    ).map((entry) => entry.outputItemId));
    if (!anchors.size) return groupedItems;
    const visibleIds = new Set(groupedItems.flatMap((entry) =>
      entry.kind === "toolGroup" ? entry.group.items.map((item) => item.id) : [entry.item.id],
    ));
    const entries: Array<MessageListEntry | { kind: "nowAnchor"; item: ConversationItem; visible: boolean }> = [];
    let segment: ConversationItem[] = [];
    const flush = () => {
      entries.push(...buildToolGroups(segment));
      segment = [];
    };
    // Split at the host's exact original output ID before tool grouping can
    // merge that ID away or hide its block inside a collapsed group.
    for (const item of items) {
      const visible = visibleIds.has(item.id) || item.kind === "explore";
      if (anchors.has(item.id)) {
        flush();
        entries.push({ kind: "nowAnchor", item, visible });
      } else if (visible) {
        segment.push(item);
      }
    }
    flush();
    return entries;
  }, [items, groupedItems, now?.conversation.batches]);

  const planFollowupNode =
    planFollowup.shouldShow && onPlanAccept && onPlanSubmitChanges ? (
      <PlanReadyFollowupMessage
        onAccept={() => {
          dismissPlanFollowup();
          onPlanAccept();
        }}
        onSubmitChanges={(changes) => {
          dismissPlanFollowup();
          onPlanSubmitChanges(changes);
        }}
      />
    ) : null;

  const renderItemContent = (item: ConversationItem) => {
    if (item.kind === "message") {
      const isCopied = copiedMessageId === item.id;
      return (
        <Fragment key={item.id}>
        <MessageRow
          item={item}
          isCopied={isCopied}
          onCopy={handleCopyMessage}
          onQuote={onQuoteMessage ? handleQuoteMessage : undefined}
          codeBlockCopyUseModifier={codeBlockCopyUseModifier}
          showMessageFilePath={showMessageFilePath}
          workspacePath={workspacePath}
          onOpenFileLink={openFileLink}
          onOpenFileLinkMenu={showFileLinkMenu}
          onOpenThreadLink={handleOpenThreadLink}
        />
        </Fragment>
      );
    }
    if (item.kind === "reasoning") {
      const isExpanded = expandedItems.has(item.id);
      const parsed = reasoningMetaById.get(item.id) ?? parseReasoning(item);
      return (
        <ReasoningRow
          key={item.id}
          item={item}
          parsed={parsed}
          isExpanded={isExpanded}
          onToggle={toggleExpanded}
          showMessageFilePath={showMessageFilePath}
          workspacePath={workspacePath}
          onOpenFileLink={openFileLink}
          onOpenFileLinkMenu={showFileLinkMenu}
          onOpenThreadLink={handleOpenThreadLink}
        />
      );
    }
    if (item.kind === "review") {
      return (
        <ReviewRow
          key={item.id}
          item={item}
          showMessageFilePath={showMessageFilePath}
          workspacePath={workspacePath}
          onOpenFileLink={openFileLink}
          onOpenFileLinkMenu={showFileLinkMenu}
          onOpenThreadLink={handleOpenThreadLink}
        />
      );
    }
    if (item.kind === "userInput") {
      const isExpanded = expandedItems.has(item.id);
      return (
        <UserInputRow
          key={item.id}
          item={item}
          isExpanded={isExpanded}
          onToggle={toggleExpanded}
        />
      );
    }
    if (item.kind === "diff") {
      return <DiffRow key={item.id} item={item} />;
    }
    if (item.kind === "tool") {
      const isExpanded = expandedItems.has(item.id);
      return (
        <ToolRow
          key={item.id}
          item={item}
          isExpanded={isExpanded}
          onToggle={toggleExpanded}
          showMessageFilePath={showMessageFilePath}
          workspacePath={workspacePath}
          onOpenFileLink={openFileLink}
          onOpenFileLinkMenu={showFileLinkMenu}
          onOpenThreadLink={handleOpenThreadLink}
          onRequestAutoScroll={requestAutoScroll}
        />
      );
    }
    if (item.kind === "explore") {
      return <ExploreRow key={item.id} item={item} />;
    }
    return null;
  };

  const renderItem = (item: ConversationItem) => (
    <Fragment key={item.id}>
      {renderItemContent(item)}
      {afterItem?.(item.id)}
      <NowOutputSlot workspaceId={workspaceId} threadId={threadId} itemId={item.id} />
    </Fragment>
  );

  return (
    <div
      className="messages messages-full"
      ref={containerRef}
      onScroll={updateAutoScroll}
    >
      <div className="messages-inner">
        {anchoredItems.map((entry) => {
          if (entry.kind === "nowAnchor") {
            return <Fragment key={entry.item.id}>
              {entry.visible && renderItemContent(entry.item)}
              {entry.visible && afterItem?.(entry.item.id)}
              <NowOutputSlot workspaceId={workspaceId} threadId={threadId} itemId={entry.item.id} />
            </Fragment>;
          }
          if (entry.kind === "toolGroup") {
            const { group } = entry;
            const isCollapsed = collapsedToolGroups.has(group.id);
            const summaryParts = [
              group.toolCount === 1
                ? tx("{count} tool call", { count: group.toolCount })
                : tx("{count} tool calls", { count: group.toolCount }),
            ];
            if (group.messageCount > 0) {
              summaryParts.push(
                group.messageCount === 1
                  ? tx("{count} message", { count: group.messageCount })
                  : tx("{count} messages", { count: group.messageCount }),
              );
            }
            const summaryText = summaryParts.join(", ");
            const groupBodyId = `tool-group-${group.id}`;
            const ChevronIcon = isCollapsed ? ChevronDown : ChevronUp;
            return (
              <div
                key={`tool-group-${group.id}`}
                className={`tool-group ${isCollapsed ? "tool-group-collapsed" : ""}`}
              >
                <div className="tool-group-header">
                  <button
                    type="button"
                    className="tool-group-toggle"
                    onClick={() => toggleToolGroup(group.id)}
                    aria-expanded={!isCollapsed}
                    aria-controls={groupBodyId}
                    aria-label={isCollapsed ? tx("Expand tool calls") : tx("Collapse tool calls")}
                  >
                    <span className="tool-group-chevron" aria-hidden>
                      <ChevronIcon size={14} />
                    </span>
                    <span className="tool-group-summary">{summaryText}</span>
                  </button>
                </div>
                {!isCollapsed && (
                  <div className="tool-group-body" id={groupBodyId}>
                    {group.items.map(renderItem)}
                  </div>
                )}
              </div>
            );
          }
          return renderItem(entry.item);
        })}
        {planFollowupNode}
        {userInputNode}
        <WorkingIndicator
          isThinking={isThinking}
          processingStartedAt={processingStartedAt}
          lastDurationMs={lastDurationMs}
          hasItems={items.length > 0}
          reasoningLabel={latestReasoningLabel}
          showPollingFetchStatus={showPollingFetchStatus}
          pollingIntervalMs={pollingIntervalMs}
        />
        {!items.length && !userInputNode && !isThinking && !isLoadingMessages && (
          <div className="empty messages-empty">
            {threadId
              ? tx("Send a prompt to the agent.")
              : tx("Send a prompt to start a new agent.")}
          </div>
        )}
        {!items.length && !userInputNode && !isThinking && isLoadingMessages && (
          <div className="empty messages-empty">
            <div className="messages-loading-indicator" role="status" aria-live="polite">
              <span className="working-spinner" aria-hidden />
              <span className="messages-loading-label">{tx("Loading…")}</span>
            </div>
          </div>
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
});
