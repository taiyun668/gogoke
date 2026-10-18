import { FileTreePanel } from "../../../files/components/FileTreePanel";
import { GitDiffPanel } from "../../../git/components/GitDiffPanel";
import { GitDiffViewer } from "../../../git/components/GitDiffViewer";
import { PromptPanel } from "../../../prompts/components/PromptPanel";
import { useMemo } from "react";
import { PanelTabs, PanelTabsScope } from "../../components/PanelTabs";
import { SeatsPanel, type SeatsSource } from "../../../seats/SeatsPanel";
import type { SeatsPage } from "../../../seats/seatsPageModel";
import { SideChatPanel, type SideChatSource } from "../../../sidechat/SideChatPanel";
import { createDesign37SideChatPanelSource } from "../../../sidechat/design37SideChatSource";
import { useNowConversation } from "../../../now/NowContext";
import { createDesign37SeatsSource, design37UserFrame } from "@/services/tauri";
import type {
  LayoutGitSurface,
  LayoutNodesResult,
} from "./types";

export type GitLayoutNodesOptions = LayoutGitSurface;

type GitLayoutNodes = Pick<LayoutNodesResult, "gitDiffPanelNode" | "gitDiffViewerNode">;

function resolveGitDiffStyle({
  isPhone,
  splitChatDiffView,
  centerMode,
  userPreference,
}: {
  isPhone: boolean;
  splitChatDiffView: boolean;
  centerMode: GitLayoutNodesOptions["diffViewProps"]["centerMode"];
  userPreference: GitLayoutNodesOptions["diffViewProps"]["gitDiffViewStyle"];
}): GitLayoutNodesOptions["diffViewProps"]["gitDiffViewStyle"] {
  const shouldForceSingleColumn =
    isPhone || (splitChatDiffView && centerMode === "chat");
  return shouldForceSingleColumn ? "unified" : userPreference;
}

function buildGitDiffPanelNode(options: GitLayoutNodesOptions) {
  const selectedDiffPath =
    options.diffViewProps.centerMode === "diff"
      ? options.gitDiffViewerProps.selectedPath
      : null;

  if (options.filePanelMode === "seats" || options.filePanelMode === "sidechat") {
    return <Design37ProjectPanel options={options} />;
  }

  if (options.filePanelMode === "files" && options.fileTreeProps) {
    return <FileTreePanel {...options.fileTreeProps} />;
  }
  if (options.filePanelMode === "prompts") {
    return <PromptPanel {...options.promptPanelProps} />;
  }
  return (
    <GitDiffPanel
      {...options.gitDiffPanelProps}
      selectedPath={selectedDiffPath}
    />
  );
}

/** A legacy workspace id is a lookup key, never a native domain id. */
function Design37ProjectPanel({ options }: { options: GitLayoutNodesOptions }) {
  const native = useNowConversation(options.project?.workspaceId ?? null, options.project?.threadId ?? null);
  const domainId = native?.status === "known" ? native.conversation.domainId : null;
  const seats = useMemo<SeatsSource>(() => domainId
    ? createDesign37SeatsSource<SeatsPage>(domainId)
    : { read: async () => null, actions: {} }, [domainId]);
  const side = useMemo<SideChatSource>(() => domainId
    ? createDesign37SideChatPanelSource(domainId, design37UserFrame)
    : { read: async () => null, actions: {} }, [domainId]);
  return <>
    <PanelTabs active="git" onSelect={() => {}} />
    {options.filePanelMode === "seats" ? <SeatsPanel source={seats} /> : <SideChatPanel source={side} />}
  </>;
}

function buildGitDiffViewerNode(options: GitLayoutNodesOptions) {
  return (
    <GitDiffViewer
      {...options.gitDiffViewerProps}
      diffStyle={resolveGitDiffStyle({
        isPhone: options.diffViewProps.isPhone,
        splitChatDiffView: options.diffViewProps.splitChatDiffView,
        centerMode: options.diffViewProps.centerMode,
        userPreference: options.diffViewProps.gitDiffViewStyle,
      })}
    />
  );
}

export function buildGitNodes(options: GitLayoutNodesOptions): GitLayoutNodes {
  const panel = buildGitDiffPanelNode(options);
  return {
    gitDiffPanelNode: options.panelTabsSelection
      ? <PanelTabsScope value={options.panelTabsSelection}>{panel}</PanelTabsScope> : panel,
    gitDiffViewerNode: buildGitDiffViewerNode(options),
  };
}
