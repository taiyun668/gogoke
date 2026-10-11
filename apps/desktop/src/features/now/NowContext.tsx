import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { NowBlock, NowPin, StopMenu, useInView, type NowActions } from "./NowBlock";
import { batchWord, type BatchWord, type NowBatch } from "./nowModel";

/** Legacy UI IDs locate a row; native IDs come only from the host's explicit association. */
export type NowConversation = {
  workspaceId: string;
  threadId: string;
  domainId: string;
  leadSessionId: string;
  currentBatchId: string | null;
  pendingInput: boolean;
  batches: Array<{
    batch: NowBatch;
    leadTurnId: string;
    outputItemId: string;
  }>;
  actions?: NowActions & {
    stopTurn?: () => void;
    stopWork?: () => void;
  };
};

/** Every successful read is a complete replacement, including all visible conversations. */
export type NowSnapshot = {
  readAt: string;
  conversations: NowConversation[];
};

/** Null or rejection means the host cannot currently be read. */
export type NowSource = { read: () => Promise<NowSnapshot | null> };

type LegacyPair = { workspaceId: string; threadId: string };
type ReadState = { source: NowSource; token: object; snapshot: NowSnapshot | null; frozen: boolean; error: string | null };
type Anchor = { key: string; element: HTMLElement };
type ContextValue = {
  source: NowSource | null;
  token: object;
  read: ReadState | null;
  active: LegacyPair | null;
  anchor: Anchor | null;
  setAnchor: (next: Anchor | null | ((previous: Anchor | null) => Anchor | null)) => void;
};

const Context = createContext<ContextValue | null>(null);

const samePair = (a: LegacyPair, b: LegacyPair) =>
  a.workspaceId === b.workspaceId && a.threadId === b.threadId;
const anchorKey = (pair: LegacyPair, batchId: string) =>
  JSON.stringify([pair.workspaceId, pair.threadId, batchId]);
const present = (value: string | undefined | null) => typeof value === "string" && value.length > 0;

function exactConversation(snapshot: NowSnapshot, pair: LegacyPair): NowConversation | null {
  const matches = snapshot.conversations.filter((entry) => samePair(entry, pair));
  if (matches.length !== 1) return null;
  const conversation = matches[0];
  if (!present(conversation.domainId) || !present(conversation.leadSessionId)) return null;
  return conversation;
}

function exactBatch(conversation: NowConversation, batchId: string) {
  const matches = conversation.batches.filter((entry) =>
    entry.batch.id === batchId && present(entry.leadTurnId) && present(entry.outputItemId),
  );
  return matches.length === 1 ? matches[0] : null;
}

export function NowProvider({
  source,
  active,
  children,
}: {
  source: NowSource | null;
  active: LegacyPair | null;
  children: ReactNode;
}) {
  const [read, setRead] = useState<ReadState | null>(null);
  const [anchor, setAnchor] = useState<Anchor | null>(null);
  // A -> B -> A starts a new read lifecycle, including reuse of source A.
  const token = useMemo(() => ({}), [source]);

  useEffect(() => {
    if (!source) return undefined;
    let live = true;
    let timer: number | undefined;
    const refresh = async () => {
      let snapshot: NowSnapshot | null = null;
      let error: string | null = null;
      try {
        snapshot = await source.read();
      } catch (cause) {
        error = String(cause);
      }
      if (!live) return;
      setRead((previous) => {
        const last = previous?.token === token ? previous.snapshot : null;
        return { source, token, snapshot: snapshot ?? last, frozen: snapshot === null, error };
      });
      timer = window.setTimeout(refresh, 2500);
    };
    void refresh();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [source, token]);

  const value = useMemo<ContextValue>(
    () => ({ source, token, read, active, anchor, setAnchor }),
    [source, token, read, active, anchor],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export type NowConversationView = {
  status: "known" | "frozen";
  conversation: NowConversation;
  batchWord: BatchWord;
  frozenAt?: string;
  readError?: string;
};

/** Exact lookup for any visible pinned, recent, or conversation row. */
export function useNowConversation(workspaceId: string | null, threadId: string | null): NowConversationView | null {
  const context = useContext(Context);
  if (!workspaceId || !threadId || !context?.source || context.read?.token !== context.token || !context.read.snapshot) return null;
  const conversation = exactConversation(context.read.snapshot, { workspaceId, threadId });
  if (!conversation) return null;
  const current = conversation.currentBatchId
    ? exactBatch(conversation, conversation.currentBatchId)?.batch ?? null
    : null;
  return {
    status: context.read.frozen ? "frozen" : "known",
    conversation,
    batchWord: current ? batchWord(current, conversation.pendingInput) : conversation.pendingInput ? "待处理" : null,
    frozenAt: context.read.frozen ? context.read.snapshot.readAt : undefined,
    readError: context.read.frozen ? context.read.error ?? undefined : undefined,
  };
}

/** The active conversation uses the same exact lookup as every visible row. */
export function useNowActiveConversation(): NowConversationView | null {
  const active = useContext(Context)?.active;
  return useNowConversation(active?.workspaceId ?? null, active?.threadId ?? null);
}

/** The original read error remains available even before the first snapshot. */
export function useNowReadError(): string | null {
  const context = useContext(Context);
  return context?.source && context.read?.token === context.token ? context.read.error : null;
}

/** Render only batches explicitly anchored to this exact lead output item. */
export function NowOutputSlot({ workspaceId, threadId, itemId }: {
  workspaceId: string | null;
  threadId: string | null;
  itemId: string | null;
}) {
  const view = useNowConversation(workspaceId, threadId);
  const context = useContext(Context);
  const currentKey = view?.conversation.currentBatchId && workspaceId && threadId
    ? anchorKey({ workspaceId, threadId }, view.conversation.currentBatchId)
    : null;
  const setAnchor = context?.setAnchor;
  const setCurrentAnchor = useCallback((element: HTMLDivElement | null) => {
    if (!setAnchor || !currentKey) return;
    setAnchor((previous) => {
      if (element) return previous?.key === currentKey && previous.element === element
        ? previous : { key: currentKey, element };
      return previous?.key === currentKey ? null : previous;
    });
  }, [setAnchor, currentKey]);

  if (!view || !context || !workspaceId || !threadId || !itemId) return null;
  const conversation = view.conversation;
  const entries = conversation.batches.filter((entry) =>
    entry.outputItemId === itemId && exactBatch(conversation, entry.batch.id) === entry,
  );
  if (!entries.length) return null;
  return <>
    {entries.map(({ batch }) => {
      const key = anchorKey({ workspaceId, threadId }, batch.id);
      return <div key={batch.id} ref={key === currentKey ? setCurrentAnchor : undefined}>
        <NowBlock
          batch={batch}
          pendingInput={batch.id === conversation.currentBatchId && conversation.pendingInput}
          frozenAt={view.frozenAt}
          actions={view.status === "known" ? conversation.actions : undefined}
        />
      </div>;
    })}
  </>;
}

/** Pin only the host's current batch when its actual block is mounted out of view. */
export function NowPinSlot() {
  const context = useContext(Context);
  const active = context?.active;
  const view = useNowConversation(active?.workspaceId ?? "", active?.threadId ?? "");
  const batchId = view?.conversation.currentBatchId;
  const entry = view && batchId ? exactBatch(view.conversation, batchId) : null;
  const key = active && batchId ? anchorKey(active, batchId) : null;
  const element = context?.anchor?.key === key ? context.anchor.element : null;
  const ref = useMemo(() => ({ current: element }), [element]);
  const inView = useInView(ref);
  if (!entry || !element || inView) return null;
  return <NowPin batch={entry.batch} frozenAt={view?.frozenAt} onJump={() => element.scrollIntoView({ behavior: "smooth", block: "center" })} />;
}

/** The host supplies the available actions; click does not invent a stop result. */
export function NowComposerStop({ workspaceId, threadId, disabled = false }: {
  workspaceId: string | null;
  threadId: string | null;
  disabled?: boolean;
}) {
  const view = useNowConversation(workspaceId, threadId);
  if (!view) return null;
  const { stopTurn, stopWork } = view.conversation.actions ?? {};
  if (!stopTurn && !stopWork) return null;
  return <StopMenu onStopTurn={stopTurn} onStopWork={stopWork} disabled={disabled || view.status === "frozen"} />;
}
