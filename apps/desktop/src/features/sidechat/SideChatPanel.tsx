import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Markdown } from "@/features/messages/components/Markdown";
import { PopoverMenuItem, PopoverSurface } from "@/features/design-system/components/popover/PopoverPrimitives";
import { VENDOR_ICONS } from "@/features/instances/vendorIcons";
import {
  canAsk,
  chatStarted,
  instanceById,
  seatById,
  sentLine,
  syncLine,
  type SideChat,
  type SideChatPage,
  type SideItem,
} from "./sideChatModel";
import "./sidechat.css";

export type NewSideChat = { seatId: string; instanceId: string; model: string; effort: string; question: string };

/** Host operations; a control is shown only when its operation exists. */
export type SideChatActions = {
  create?: (input: NewSideChat) => Promise<void>;
  ask?: (chatId: string, question: string) => Promise<void>;
  stop?: (chatId: string) => Promise<void>;
  retry?: (chatId: string) => Promise<void>;
  setModel?: (chatId: string, model: string, effort: string) => Promise<void>;
  archive?: (chatId: string) => Promise<void>;
  restore?: (chatId: string) => Promise<void>;
  remove?: (chatId: string) => Promise<void>;
};

export type SideChatSource = {
  /** Null while the host has no side chat read model for the current work. */
  read: () => Promise<SideChatPage | null>;
  actions: SideChatActions;
};

type View = { kind: "list" } | { kind: "new" } | { kind: "chat"; id: string };
type RunFn = (operation: () => Promise<void>) => Promise<boolean>;

const DRAFT_KEY = "gogoke.sidechat.drafts.v1";
function readDrafts(): Record<string, string> {
  try {
    return JSON.parse(window.localStorage.getItem(DRAFT_KEY) ?? "{}") as Record<string, string>;
  } catch {
    return {};
  }
}
function writeDrafts(drafts: Record<string, string>) {
  try {
    window.localStorage.setItem(DRAFT_KEY, JSON.stringify(drafts));
  } catch {
    // Drafts are a convenience; losing them never blocks sending.
  }
}

function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

const VendorMark = ({ vendor }: { vendor: keyof typeof VENDOR_ICONS }) => (
  <span
    className="sidechat-vendor"
    style={{ WebkitMaskImage: `url(${VENDOR_ICONS[vendor]})`, maskImage: `url(${VENDOR_ICONS[vendor]})` }}
    aria-hidden
  />
);

export function SideChatPanel({ source }: { source: SideChatSource }) {
  const [page, setPage] = useState<SideChatPage | null | undefined>(undefined);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [view, setView] = useState<View>({ kind: "list" });
  const [drafts, setDrafts] = useState<Record<string, string>>(readDrafts);
  const busyRef = useRef(false);

  const refresh = async () => {
    try {
      setPage(await source.read());
      setLoadError(null);
    } catch (cause) {
      setLoadError(errorText(cause));
    }
  };

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => {
      if (!busyRef.current) void refresh();
    }, 1500);
    return () => window.clearInterval(timer);
  }, [source]);

  const run: RunFn = async (operation) => {
    if (busyRef.current) return false;
    busyRef.current = true;
    setActionError(null);
    let ok = true;
    try {
      await operation();
    } catch (cause) {
      ok = false;
      setActionError(errorText(cause));
    } finally {
      busyRef.current = false;
      void refresh();
    }
    return ok;
  };

  const setDraft = (key: string, value: string) =>
    setDrafts((current) => {
      const next = { ...current, [key]: value };
      writeDrafts(next);
      return next;
    });

  if (page === undefined && !loadError) {
    return (
      <div className="sidechat">
        <div className="sidechat-help" role="status">
          正在读取旁聊…
        </div>
      </div>
    );
  }
  if (loadError) {
    return (
      <div className="sidechat">
        <div className="sidechat-help sidechat-error" role="alert">
          读不到旁聊：{loadError}
        </div>
      </div>
    );
  }
  if (!page) {
    return (
      <div className="sidechat">
        <div className="sidechat-overview">
          <span className="sidechat-title">旁聊</span>
          <span className="sidechat-help">旁聊数据还没接上，这里暂时用不了。</span>
        </div>
      </div>
    );
  }

  const current = view.kind === "chat" ? page.chats.find((chat) => chat.id === view.id) ?? null : null;
  if (view.kind === "chat" && !current) {
    return <ChatList page={page} canCreate={Boolean(source.actions.create)} onOpen={(id) => setView({ kind: "chat", id })} onNew={() => setView({ kind: "new" })} />;
  }

  return (
    <div className="sidechat">
      {actionError ? (
        <div className="sidechat-help sidechat-error" role="alert">
          {actionError}
        </div>
      ) : null}
      {view.kind === "list" ? (
        <ChatList
          page={page}
          canCreate={Boolean(source.actions.create)}
          onOpen={(id) => setView({ kind: "chat", id })}
          onNew={() => setView({ kind: "new" })}
        />
      ) : view.kind === "new" ? (
        <NewChat
          page={page}
          actions={source.actions}
          draft={drafts.new ?? ""}
          onDraft={(value) => setDraft("new", value)}
          run={run}
          onBack={() => setView({ kind: "list" })}
          onCreated={() => {
            setDraft("new", "");
            setView({ kind: "list" });
          }}
        />
      ) : current ? (
        <ChatView
          page={page}
          chat={current}
          actions={source.actions}
          draft={drafts[current.id] ?? ""}
          onDraft={(value) => setDraft(current.id, value)}
          run={run}
          onBack={() => setView({ kind: "list" })}
        />
      ) : null}
    </div>
  );
}

function ChatList({
  page,
  canCreate,
  onOpen,
  onNew,
}: {
  page: SideChatPage;
  canCreate: boolean;
  onOpen: (id: string) => void;
  onNew: () => void;
}) {
  const [showArchived, setShowArchived] = useState(false);
  const live = page.chats.filter((chat) => !chat.archived);
  const archived = page.chats.filter((chat) => chat.archived);
  const row = (chat: SideChat) => {
    const seat = seatById(page, chat.seatId);
    const instance = instanceById(page, chat.instanceId);
    return (
      <button key={chat.id} type="button" className="sidechat-item" onClick={() => onOpen(chat.id)}>
        <span className="sidechat-item-top">
          <span className="sidechat-item-title">{chat.title}</span>
          <span className="sidechat-item-time">{chat.updatedAt}</span>
        </span>
        <span className="sidechat-meta">
          {instance ? <VendorMark vendor={instance.vendor} /> : null}
          {seat?.name ?? "已删除的席位"} · {instance?.name ?? "实例不可用"}
          {chat.archived ? " · 已归档" : ""}
        </span>
        {chat.answering || (!chat.archived && chat.pendingLeadSegments) ? (
          <span className="sidechat-meta">
            {chat.answering ? (
              <span className="sidechat-badge">
                <span className="sidechat-dot is-busy" aria-hidden />
                回答中
              </span>
            ) : null}
            {!chat.archived && chat.pendingLeadSegments ? (
              <span className="sidechat-badge">主控有 {chat.pendingLeadSegments} 段新进展</span>
            ) : null}
          </span>
        ) : null}
      </button>
    );
  };
  return (
    <div className="sidechat-scroll">
      <div className="sidechat-overview">
        <span className="sidechat-title">旁聊</span>
        <span className="sidechat-help">
          私下问问直属席位或秘书长。这里说的不算数；要让主控知道，直接跟它说“告诉主控……”。
        </span>
      </div>
      {canCreate ? (
        <div className="git-root-actions">
          <button type="button" className="ghost git-root-button" onClick={onNew}>
            新开旁聊
          </button>
        </div>
      ) : null}
      {live.length ? live.map(row) : <div className="sidechat-help">这件工作还没有旁聊。</div>}
      {archived.length ? (
        <>
          {showArchived ? archived.map(row) : null}
          <button type="button" className="sidechat-link" onClick={() => setShowArchived((value) => !value)}>
            {showArchived ? "收起已归档的" : `已归档 ${archived.length} 个`}
          </button>
        </>
      ) : null}
    </div>
  );
}

function Bar({ title, onBack, children }: { title: string; onBack: () => void; children?: ReactNode }) {
  return (
    <div className="sidechat-bar">
      <button type="button" className="sidechat-back" onClick={onBack}>
        ‹ 旁聊
      </button>
      <span className="sidechat-bar-title">{title}</span>
      {children}
    </div>
  );
}

function NewChat({
  page,
  actions,
  draft,
  onDraft,
  run,
  onBack,
  onCreated,
}: {
  page: SideChatPage;
  actions: SideChatActions;
  draft: string;
  onDraft: (value: string) => void;
  run: RunFn;
  onBack: () => void;
  onCreated: () => void;
}) {
  const firstSeat = page.seats[0];
  const [seatId, setSeatId] = useState(firstSeat?.id ?? "");
  const [instanceId, setInstanceId] = useState(firstSeat?.defaultInstanceId ?? page.instances[0]?.id ?? "");
  const instance = instanceById(page, instanceId);
  const [model, setModel] = useState(instance?.models[0] ?? "");
  const [effort, setEffort] = useState(page.efforts[0] ?? "");
  const seat = seatById(page, seatId);

  if (!seat || !instance) {
    return (
      <>
        <Bar title="新开旁聊" onBack={onBack} />
        <div className="sidechat-scroll">
          <div className="sidechat-help">现在没有能问的直属席位或能用的实例。</div>
        </div>
      </>
    );
  }

  const send = () => {
    const question = draft.trim();
    if (!question || !actions.create) return;
    void run(() => actions.create!({ seatId, instanceId, model, effort, question })).then((ok) => ok && onCreated());
  };

  return (
    <>
      <Bar title="新开旁聊" onBack={onBack} />
      <div className="sidechat-scroll">
        <div className="sidechat-help">
          问{seat.name}，会参考主控到第 {page.leadRound} 轮的进展，只看本项目。你发第一个问题之前，不会调用模型。
          <br />
          在输入框下面选问谁、用哪个实例；发出第一个问题后就固定了。
        </div>
      </div>
      <Composer
        placeholder={`问${seat.name}…`}
        draft={draft}
        onDraft={onDraft}
        onSend={send}
        who={{
          label: `${seat.name} · ${instance.name}`,
          vendor: instance.vendor,
          fixed: false,
          seats: page.seats,
          instances: page.instances,
          seatId,
          instanceId,
          onSeat: (id) => {
            const next = seatById(page, id);
            setSeatId(id);
            if (next) {
              setInstanceId(next.defaultInstanceId);
              setModel(instanceById(page, next.defaultInstanceId)?.models[0] ?? model);
            }
          },
          onInstance: (id) => {
            setInstanceId(id);
            setModel(instanceById(page, id)?.models[0] ?? model);
          },
        }}
        modelChoice={{ models: instance.models, efforts: page.efforts, model, effort, onPick: (m, e) => { setModel(m); setEffort(e); } }}
        permission={seat.permission}
      />
    </>
  );
}

function ChatView({
  page,
  chat,
  actions,
  draft,
  onDraft,
  run,
  onBack,
}: {
  page: SideChatPage;
  chat: SideChat;
  actions: SideChatActions;
  draft: string;
  onDraft: (value: string) => void;
  run: RunFn;
  onBack: () => void;
}) {
  const [menu, setMenu] = useState(false);
  const [info, setInfo] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [showRaw, setShowRaw] = useState(false);
  const seat = seatById(page, chat.seatId);
  const instance = instanceById(page, chat.instanceId);
  const sync = syncLine(chat);
  const askable = canAsk(chat);

  const send = () => {
    const question = draft.trim();
    if (!question || !actions.ask) return;
    void run(() => actions.ask!(chat.id, question)).then((ok) => ok && onDraft(""));
  };

  return (
    <>
      <Bar title={chat.title} onBack={onBack}>
        <span className="sidechat-menu-anchor">
          <button
            type="button"
            className="ghost icon-button"
            aria-haspopup="menu"
            aria-expanded={menu}
            aria-label="旁聊的更多操作"
            onClick={() => setMenu((value) => !value)}
          >
            <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16" aria-hidden>
              <circle cx="5" cy="12" r="1.8" />
              <circle cx="12" cy="12" r="1.8" />
              <circle cx="19" cy="12" r="1.8" />
            </svg>
          </button>
          {menu ? (
            <PopoverSurface className="sidechat-menu" role="menu">
              <PopoverMenuItem role="menuitem" onClick={() => { setInfo((value) => !value); setMenu(false); }}>
                {info ? "收起旁聊信息" : "旁聊信息"}
              </PopoverMenuItem>
              {chat.archived ? (
                <>
                  {actions.restore ? (
                    <PopoverMenuItem role="menuitem" onClick={() => { setMenu(false); void run(() => actions.restore!(chat.id)); }}>
                      恢复
                    </PopoverMenuItem>
                  ) : null}
                  {actions.remove ? (
                    <PopoverMenuItem role="menuitem" className="is-danger" onClick={() => { setMenu(false); setConfirmDelete(true); }}>
                      删除
                    </PopoverMenuItem>
                  ) : null}
                </>
              ) : actions.archive ? (
                <PopoverMenuItem role="menuitem" onClick={() => { setMenu(false); void run(() => actions.archive!(chat.id)).then((ok) => ok && onBack()); }}>
                  归档
                </PopoverMenuItem>
              ) : null}
            </PopoverSurface>
          ) : null}
        </span>
      </Bar>
      {info ? (
        <dl className="sidechat-info">
          <dt>参考到</dt>
          <dd>
            主控第 {chat.referenceRound} 轮
            {chat.pendingLeadSegments ? `（之后又有 ${chat.pendingLeadSegments} 段，下次提问带上）` : ""}
          </dd>
          <dt>同步方式</dt>
          <dd>{chat.appendsImmediately ? "主控有新进展就直接追加过来" : "攒着，你下次提问时一起带上"}</dd>
          <dt>权限</dt>
          <dd>{seat?.permission ?? "—"}</dd>
          <dt>注意</dt>
          <dd>旁聊不是正式审查。正式审查是另起的干净会话，你在这里私下说的话不会带进去。</dd>
        </dl>
      ) : null}
      {confirmDelete && actions.remove ? (
        <div className="sidechat-problem">
          删除这个旁聊？只删旁聊里你们俩说的话，主控的记录不受影响。删除后不能恢复。
          <div className="git-root-actions">
            <button type="button" className="ghost git-root-button" onClick={() => void run(() => actions.remove!(chat.id)).then((ok) => ok && onBack())}>
              确认删除
            </button>
            <button type="button" className="ghost git-root-button" onClick={() => setConfirmDelete(false)}>
              取消
            </button>
          </div>
        </div>
      ) : null}
      <div className="sidechat-scroll" role="log" aria-label="旁聊消息">
        <div className="sidechat-messages">
          {chat.items.map((item) => (
            <Item key={item.id} item={item} />
          ))}
          {chat.answering ? <div className="sidechat-thinking">{seat?.name ?? "席位"}正在回答…</div> : null}
          {chat.problem?.kind === "answer-failed" ? (
            <div className="sidechat-problem">
              {chat.problem.summary}
              {chat.problem.raw ? (
                <>
                  {" "}
                  <button type="button" className="sidechat-link" onClick={() => setShowRaw((value) => !value)}>
                    {showRaw ? "收起原话" : "查看原话"}
                  </button>
                  {showRaw ? <pre className="sidechat-raw">{chat.problem.raw}</pre> : null}
                </>
              ) : null}
              {actions.retry ? (
                <div className="git-root-actions">
                  <button type="button" className="ghost git-root-button" onClick={() => void run(() => actions.retry!(chat.id))}>
                    重试
                  </button>
                </div>
              ) : null}
            </div>
          ) : null}
          {sync ? <div className="sidechat-sync">{sync}</div> : null}
        </div>
      </div>
      {chat.archived ? (
        <footer className="composer sidechat-composer">
          <div className="sidechat-help sidechat-center">已归档。点右上角 ⋯ 恢复后可以接着聊。</div>
        </footer>
      ) : chat.problem?.kind === "seat-removed" ? (
        <footer className="composer sidechat-composer">
          <div className="sidechat-problem is-warn">
            担任这个旁聊的“{chat.problem.seatName}”席位已经删除，这里只能看不能问。想接着问，就新开一个旁聊。
          </div>
        </footer>
      ) : (
        <>
          {chat.problem?.kind === "instance-full" ? (
            <div className="sidechat-problem is-warn sidechat-inset">
              {chat.problem.instanceName} 同时开的会话满了（{chat.problem.used}/{chat.problem.cap}）。等它空出来再问，或者新开一个旁聊用别的实例。
            </div>
          ) : null}
          {seat && instance ? (
            <Composer
              placeholder={chat.answering ? "写下一个问题，等它答完再发…" : `问${seat.name}…`}
              draft={draft}
              onDraft={onDraft}
              onSend={send}
              disabled={!askable || !actions.ask}
              answering={chat.answering}
              onStop={actions.stop ? () => void run(() => actions.stop!(chat.id)) : undefined}
              who={{ label: `${seat.name} · ${instance.name}`, vendor: instance.vendor, fixed: chatStarted(chat) }}
              modelChoice={
                actions.setModel
                  ? {
                      models: instance.models,
                      efforts: page.efforts,
                      model: chat.model,
                      effort: chat.effort,
                      onPick: (m, e) => void run(() => actions.setModel!(chat.id, m, e)),
                    }
                  : { models: [chat.model], efforts: [chat.effort], model: chat.model, effort: chat.effort, fixed: true }
              }
              permission={seat.permission}
            />
          ) : null}
        </>
      )}
    </>
  );
}

function Item({ item }: { item: SideItem }) {
  const [open, setOpen] = useState(false);
  switch (item.kind) {
    case "user":
      return (
        <div className="message user">
          <div className="bubble">{item.text}</div>
        </div>
      );
    case "answer":
      return (
        <div className="sidechat-answer">
          <div className="message assistant">
            <div className="bubble">
              <Markdown value={item.text} />
            </div>
          </div>
          <div className="sidechat-answer-foot">
            {item.leadUpdatedAfter ? <span className="sidechat-updated">之后主控已更新 · 下次提问会带上 </span> : null}
            <span className="sidechat-basis">依据：这条回答开始时的现场（{item.basis}）</span>
          </div>
        </div>
      );
    case "sent-to-lead":
      return (
        <button type="button" className="sidechat-sent" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
          <span className="sidechat-sent-mark" aria-hidden>
            ↗
          </span>
          <span className={item.result === "failed" ? "sidechat-sent-failed" : undefined}>{sentLine(item)}</span>
          {open ? (
            <span className="sidechat-sent-detail">
              “{item.text}”{item.error ? <span className="sidechat-error"> · {item.error}</span> : null}
              <span className="sidechat-help"> · {item.at}</span>
            </span>
          ) : null}
        </button>
      );
    case "from-lead":
      return (
        <div className="sidechat-from-lead">
          <div className="sidechat-from-lead-label">来自主控 · {item.at}</div>
          <div className="message assistant">
            <div className="bubble">
              <Markdown value={item.text} />
            </div>
          </div>
        </div>
      );
  }
}

type WhoChoice = {
  label: string;
  vendor: keyof typeof VENDOR_ICONS;
  fixed: boolean;
  seats?: SideChatPage["seats"];
  instances?: SideChatPage["instances"];
  seatId?: string;
  instanceId?: string;
  onSeat?: (id: string) => void;
  onInstance?: (id: string) => void;
};
type ModelChoice = {
  models: string[];
  efforts: string[];
  model: string;
  effort: string;
  fixed?: boolean;
  onPick?: (model: string, effort: string) => void;
};

/** Compact composer: input and send first; who · instance and model · effort as two chips; permission as a lock. */
function Composer({
  placeholder,
  draft,
  onDraft,
  onSend,
  disabled = false,
  answering = false,
  onStop,
  who,
  modelChoice,
  permission,
}: {
  placeholder: string;
  draft: string;
  onDraft: (value: string) => void;
  onSend: () => void;
  disabled?: boolean;
  answering?: boolean;
  onStop?: () => void;
  who: WhoChoice;
  modelChoice: ModelChoice;
  permission: string;
}) {
  const [pop, setPop] = useState<"who" | "model" | null>(null);
  const composing = useRef(false);
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey && !composing.current && !event.nativeEvent.isComposing) {
      event.preventDefault();
      if (!answering) onSend();
    }
  };
  return (
    <footer className="composer sidechat-composer">
      <div className="composer-input">
        <div className="composer-input-area">
          <div className="composer-input-row">
            <textarea
              rows={1}
              value={draft}
              placeholder={placeholder}
              aria-label="旁聊输入"
              disabled={disabled}
              onChange={(event) => onDraft(event.target.value)}
              onCompositionStart={() => (composing.current = true)}
              onCompositionEnd={() => (composing.current = false)}
              onKeyDown={onKeyDown}
            />
            <div className="composer-input-actions">
              {answering && onStop ? (
                <button type="button" className="composer-action is-stop" aria-label="停止" onClick={onStop}>
                  <span className="composer-action-stop-square" aria-hidden />
                </button>
              ) : (
                <button
                  type="button"
                  className="composer-action is-send"
                  aria-label="发问"
                  disabled={disabled || answering || !draft.trim()}
                  onClick={onSend}
                >
                  <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2.4" aria-hidden>
                    <path d="M12 19V5M6 11l6-6 6 6" />
                  </svg>
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
      <div className="sidechat-chips">
        {pop === "who" && !who.fixed ? (
          <PopoverSurface className="sidechat-pop" role="menu">
            <div className="sidechat-pop-label">问谁</div>
            {(who.seats ?? []).map((seat) => (
              <PopoverMenuItem key={seat.id} role="menuitemradio" active={seat.id === who.seatId} onClick={() => { who.onSeat?.(seat.id); setPop(null); }}>
                {seat.name}
              </PopoverMenuItem>
            ))}
            <hr className="sidechat-pop-sep" />
            <div className="sidechat-pop-label">用哪个实例</div>
            {(who.instances ?? []).map((item) => (
              <PopoverMenuItem key={item.id} role="menuitemradio" active={item.id === who.instanceId} onClick={() => { who.onInstance?.(item.id); setPop(null); }}>
                {item.name}
              </PopoverMenuItem>
            ))}
          </PopoverSurface>
        ) : null}
        {pop === "model" && !modelChoice.fixed ? (
          <PopoverSurface className="sidechat-pop is-right" role="menu">
            <div className="sidechat-pop-label">模型</div>
            {modelChoice.models.map((item) => (
              <PopoverMenuItem key={item} role="menuitemradio" active={item === modelChoice.model} onClick={() => { modelChoice.onPick?.(item, modelChoice.effort); setPop(null); }}>
                {item}
              </PopoverMenuItem>
            ))}
            <hr className="sidechat-pop-sep" />
            <div className="sidechat-pop-label">推理强度</div>
            {modelChoice.efforts.map((item) => (
              <PopoverMenuItem key={item} role="menuitemradio" active={item === modelChoice.effort} onClick={() => { modelChoice.onPick?.(modelChoice.model, item); setPop(null); }}>
                {item}
              </PopoverMenuItem>
            ))}
          </PopoverSurface>
        ) : null}
        <button
          type="button"
          className="sidechat-chip"
          disabled={who.fixed}
          title={who.fixed ? "旁聊开始后固定；想换就新开一个" : undefined}
          aria-haspopup={who.fixed ? undefined : "menu"}
          aria-expanded={pop === "who"}
          onClick={() => setPop((value) => (value === "who" ? null : "who"))}
        >
          <VendorMark vendor={who.vendor} />
          <span className="sidechat-chip-text">{who.label}</span>
          {who.fixed ? null : <span className="sidechat-caret" aria-hidden>▾</span>}
        </button>
        <button
          type="button"
          className="sidechat-chip"
          disabled={modelChoice.fixed}
          aria-haspopup={modelChoice.fixed ? undefined : "menu"}
          aria-expanded={pop === "model"}
          onClick={() => setPop((value) => (value === "model" ? null : "model"))}
        >
          <span className="sidechat-chip-text">
            {modelChoice.model} · {modelChoice.effort}
          </span>
          {modelChoice.fixed ? null : <span className="sidechat-caret" aria-hidden>▾</span>}
        </button>
        <span className="sidechat-lock" title={permission} aria-label={`权限：${permission}`} role="img">
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
            <rect x="5" y="11" width="14" height="9" rx="2" />
            <path d="M8 11V8a4 4 0 0 1 8 0v3" />
          </svg>
        </span>
      </div>
    </footer>
  );
}
