import { useEffect, useMemo, useState } from "react";
import FolderKanban from "lucide-react/dist/esm/icons/folder-kanban";
import UsersRound from "lucide-react/dist/esm/icons/users-round";
import MessagesSquare from "lucide-react/dist/esm/icons/messages-square";
import Inbox from "lucide-react/dist/esm/icons/inbox";
import GitBranch from "lucide-react/dist/esm/icons/git-branch";
import CircleHelp from "lucide-react/dist/esm/icons/circle-help";
import Activity from "lucide-react/dist/esm/icons/activity";
import Blocks from "lucide-react/dist/esm/icons/blocks";
import { RequestUserInputMessage } from "../app/components/RequestUserInputMessage";
import { I18nProvider } from "@/i18n";
import type { RequestUserInputRequest, RequestUserInputResponse } from "../../types";

type PreviewHost = {
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  setState: (state: "NOT_INSTALLED" | "NOT_LOGGED_IN" | "LOGGED_IN" | "ERROR") => void;
  setNewVersion: (visible: boolean) => void;
  setRuntimeIssues: (visible: boolean) => void;
};

type Area = "project" | "seats" | "sessions" | "inbox" | "sidechat" | "questions" | "worktrees" | "instances";
type Scenario = { id: string; area: Area; label: string; detail: string };

const scenarios: Scenario[] = [
  { id: "A1-QUEUED", area: "inbox", label: "排队中", detail: "可编辑、插话或取消；收件人绑定原会话，不随焦点改变。" },
  { id: "A1-STEERING", area: "inbox", label: "正在插入", detail: "显示插入中；没有原始回执前不能显示已送达。" },
  { id: "A1-TURN-ENDED", area: "inbox", label: "回合已结束", detail: "消息仍留在队列或可取消，不暗中转到下一回合。" },
  { id: "A1-DELIVERED", area: "inbox", label: "已送达", detail: "示例回执时间可见；不再提供编辑或撤回。" },
  { id: "A1-UNKNOWN", area: "inbox", label: "送达未知", detail: "只读核对原请求；不自动重发。" },
  { id: "A1-CANCELLED", area: "inbox", label: "已取消", detail: "保留原记录并划线显示。" },
  { id: "A1-FAILED", area: "inbox", label: "失败", detail: "重新排队创建新消息，旧记录仍在。" },
  { id: "A2-IDLE", area: "seats", label: "空闲", detail: "可派发、换实例、回收或调整长短期。" },
  { id: "A2-WORKING", area: "seats", label: "工作中", detail: "允许查看收件箱、请求停止；换实例不可用。" },
  { id: "A2-SWITCHING", area: "seats", label: "切换实例", detail: "显示切换原因；不会同时派到旧、新会话。" },
  { id: "A2-STOP-REQ", area: "seats", label: "已请求停止", detail: "StopFact 到达前持续显示停止请求，并禁发重叠工作。" },
  { id: "A2-STUCK", area: "seats", label: "停滞", detail: "沿配置的升级链展示进展；未到 Owner 不弹窗。" },
  { id: "A2-RECLAIMING", area: "seats", label: "回收中", detail: "先检查 lead 自身条件；不满足时拒绝回收。" },
  { id: "A2-RECLAIMED", area: "seats", label: "已回收", detail: "卡片置灰；历史账本仍可读。" },
  { id: "A3-NEW", area: "sidechat", label: "新建", detail: "选择席位、实例、账本位置和权限层；首次提问前不调用模型。" },
  { id: "A3-ACTIVE", area: "sidechat", label: "活跃", detail: "只读当前项目线程，写入权限受当前层级约束。" },
  { id: "A3-LEAD-PROGRESS", area: "sidechat", label: "主控有新进展", detail: "待同步差异会展示并附在下一问；同步本身不调用模型。" },
  { id: "A3-KEPT", area: "sidechat", label: "保留", detail: "默认可恢复；不复制主控上下文。" },
  { id: "A3-ARCHIVED", area: "sidechat", label: "已归档", detail: "可以恢复或删除。" },
  { id: "A3-DELETED", area: "sidechat", label: "已删除", detail: "只移除旁聊自己的内容，主控账本不变。" },
  { id: "A4-NOT-INSTALLED", area: "instances", label: "未安装", detail: "显示固定版本的安装说明。" },
  { id: "A4-NOT-LOGGED-IN", area: "instances", label: "未登录", detail: "登录在 CLI 内完成；宿主不读凭据文件。" },
  { id: "A4-LOGGED-IN", area: "instances", label: "已登录", detail: "显示并执行并发上限；v1 不显示额度。" },
  { id: "A4-NEW-VERSION", area: "instances", label: "有新版本", detail: "提示新版本，只允许用户手动升级。" },
  { id: "A4-EXHAUSTED-OR-ERROR", area: "instances", label: "额度耗尽或错误", detail: "显示来源原因和受影响席位，不静默切实例。" },
];

const projects = [
  { id: "market-redesign", name: "商城首页改版", path: "示例项目 · web-client", branch: "feature/catalog-cards" },
  { id: "docs-refresh", name: "帮助中心更新", path: "示例项目 · docs", branch: "docs/navigation" },
];
const seats = [
  { id: "lead-review", name: "主控", layer: "USER", role: "协调与交付", status: "进行中" },
  { id: "builder-ui", name: "界面施工", layer: "LEAD", role: "页面与组件", status: "工作中" },
  { id: "reviewer-ui", name: "界面复核", layer: "LEAD", role: "独立检查", status: "等待中" },
];
const sessions = [
  { id: "session-catalog", seatId: "builder-ui", title: "重排商品卡片", status: "A2-WORKING", turn: "turn 18" },
  { id: "session-review", seatId: "reviewer-ui", title: "检查窄屏布局", status: "A2-IDLE", turn: "尚未开始" },
];
const nav: { area: Area; title: string; Icon: typeof FolderKanban }[] = [
  { area: "project", title: "项目", Icon: FolderKanban },
  { area: "seats", title: "席位", Icon: UsersRound },
  { area: "sessions", title: "会话", Icon: Activity },
  { area: "inbox", title: "收件箱", Icon: Inbox },
  { area: "sidechat", title: "旁聊", Icon: MessagesSquare },
  { area: "questions", title: "问题卡", Icon: CircleHelp },
  { area: "worktrees", title: "工作树", Icon: GitBranch },
  { area: "instances", title: "实例", Icon: Blocks },
];

const request: RequestUserInputRequest = {
  workspace_id: "market-redesign",
  request_id: "preview-question-01",
  params: {
    thread_id: "session-catalog", turn_id: "turn-18-demo", item_id: "item-question-demo",
    questions: [{ id: "card-density", header: "商品卡片", question: "窄屏商品信息优先采用哪种排布？",
      options: [
        { label: "标题优先", description: "标题和价格完整显示，图片保持缩略。" },
        { label: "图片优先", description: "保留大图，次要描述折叠。" },
      ] }],
  },
};

export function Design37WorkspacePreview({ host }: { host: PreviewHost }) {
  const [area, setArea] = useState<Area>("project");
  const [projectId, setProjectId] = useState(projects[0].id);
  const [seatId, setSeatId] = useState(seats[1].id);
  const [sessionId, setSessionId] = useState(sessions[0].id);
  const [sideChatId, setSideChatId] = useState("sidechat-brief");
  const [scenarioId, setScenarioId] = useState("A1-QUEUED");
  const [messageStatus, setMessageStatus] = useState("A1-QUEUED");
  const [messageText, setMessageText] = useState("价格和标题都要完整显示，图片可以缩小。");
  const [editMessage, setEditMessage] = useState(false);
  const [requeuedId, setRequeuedId] = useState<string | null>(null);
  const [sideChats, setSideChats] = useState<sideChatsType>([
    { id: "sidechat-brief", title: "卡片信息层级", status: "KEPT", ledger: "主控 turn 16", permission: "只读建议" },
    { id: "sidechat-a11y", title: "窄屏可访问性", status: "ACTIVE", ledger: "主控 turn 17", permission: "项目内写入" },
  ]);
  const [sideChatDraft, setSideChatDraft] = useState("扩展导航说明");
  const [answerNotice, setAnswerNotice] = useState("");
  const [instance, setInstance] = useState<Record<string, unknown> | null>(null);
  const [instanceError, setInstanceError] = useState("");
  const [selectedWorktree, setSelectedWorktree] = useState("wt-ui-single");
  const [messageCheckNotice, setMessageCheckNotice] = useState("");

  const project = projects.find((item) => item.id === projectId)!;
  const seat = seats.find((item) => item.id === seatId)!;
  const session = sessions.find((item) => item.id === sessionId)!;
  const selectedScenario = scenarios.find((item) => item.id === scenarioId)!;
  const sideChat = sideChats.find((item) => item.id === sideChatId);
  const visibleMessageStatus = messageStatus;
  const visibleStateScenario = area === "inbox"
    ? scenarios.find((item) => item.id === messageStatus) ?? selectedScenario
    : selectedScenario;

  useEffect(() => {
    if (selectedScenario.area !== "instances") return;
    host.setRuntimeIssues(false);
    host.setNewVersion(selectedScenario.id === "A4-NEW-VERSION");
    host.setState(selectedScenario.id === "A4-NOT-INSTALLED" ? "NOT_INSTALLED"
      : selectedScenario.id === "A4-LOGGED-IN" || selectedScenario.id === "A4-NEW-VERSION" ? "LOGGED_IN"
        : selectedScenario.id === "A4-EXHAUSTED-OR-ERROR" ? "ERROR" : "NOT_LOGGED_IN");
    if (selectedScenario.id === "A4-EXHAUSTED-OR-ERROR") host.setRuntimeIssues(true);
    let active = true;
    void host.invoke("gogoke_design37_instances").then((value) => {
      if (!active) return;
      const model = value as { instances?: Record<string, unknown>[] };
      setInstance(model.instances?.[0] ?? null);
      setInstanceError("");
    }).catch((cause: unknown) => {
      if (active) setInstanceError(cause instanceof Error ? cause.message : String(cause));
    });
    return () => { active = false; };
  }, [host, selectedScenario, selectedScenario.area, selectedScenario.id]);

  const stateCard = useMemo(() => ({
    label: visibleStateScenario.label, id: visibleStateScenario.id, detail: visibleStateScenario.detail,
  }), [visibleStateScenario]);
  const chooseScenario = (scenario: Scenario) => {
    setScenarioId(scenario.id);
    setArea(scenario.area);
    if (scenario.id.startsWith("A1-")) setMessageStatus(scenario.id);
  };
  const scenarioBuckets = [
    { title: "收件箱 A1", rows: scenarios.filter((item) => item.id.startsWith("A1-")) },
    { title: "席位 A2", rows: scenarios.filter((item) => item.id.startsWith("A2-")) },
    { title: "旁聊 A3", rows: scenarios.filter((item) => item.id.startsWith("A3-")) },
    { title: "实例 A4", rows: scenarios.filter((item) => item.id.startsWith("A4-")) },
  ];
  const selectArea = (nextArea: Area) => {
    setArea(nextArea);
    const initialByArea: Partial<Record<Area, string>> = {
      inbox: "A1-QUEUED", seats: "A2-IDLE", sidechat: "A3-ACTIVE", instances: "A4-LOGGED-IN",
    };
    const initial = scenarios.find((item) => item.id === initialByArea[nextArea]);
    if (initial) {
      setScenarioId(initial.id);
      if (initial.id.startsWith("A1-")) setMessageStatus(initial.id);
    }
  };

  return <I18nProvider language="zh-CN">
    <main className="g37-preview" aria-label="G.1 工作台浏览器预览">
      <header className="g37-preview__topbar">
        <div>
          <p className="g37-preview__eyebrow">G.1 · 交互预览</p>
          <h1>工作台</h1>
        </div>
        <span className="g37-preview__demo-badge">K-UI 假数据 · 不连接真实宿主</span>
      </header>
      <div className="g37-preview__layout">
        <aside className="g37-preview__rail" aria-label="项目与导航">
          <div className="g37-preview__section-label">项目</div>
          {projects.map((item) => <button key={item.id} type="button"
            className={`g37-preview__project${projectId === item.id ? " is-selected" : ""}`}
            aria-pressed={projectId === item.id} onClick={() => setProjectId(item.id)}>
            <span className="g37-preview__project-mark" aria-hidden="true">{item.name.slice(0, 1)}</span>
            <span><strong>{item.name}</strong><small>{item.path}</small></span>
          </button>)}
          <div className="g37-preview__section-label">项目视图</div>
          <nav className="g37-preview__nav" aria-label="项目视图">
            {nav.map(({ area: itemArea, title, Icon }) => <button key={itemArea} type="button"
              aria-current={area === itemArea ? "page" : undefined}
              className={area === itemArea ? "is-active" : ""} onClick={() => selectArea(itemArea)}>
              <Icon size={16} aria-hidden="true" />{title}
              {itemArea === "inbox" ? <span className="g37-preview__nav-count">2</span> : null}
            </button>)}
          </nav>
          <details className="g37-preview__scenario-menu">
            <summary>状态样例 <span>{scenarios.length} 项</span></summary>
            <p>选择一条本地示例，查看相应状态卡与可用操作。不是 V15 实测。</p>
            {scenarioBuckets.map((bucket) => <section key={bucket.title}>
              <h2>{bucket.title}</h2>
              {bucket.rows.map((scenario) => <button key={scenario.id} type="button"
                aria-pressed={scenarioId === scenario.id}
                className={scenarioId === scenario.id ? "is-active" : ""}
                onClick={() => chooseScenario(scenario)}>
                <span>{scenario.label}</span><small>{scenario.id}</small>
              </button>)}
            </section>)}
          </details>
        </aside>

        <section className="g37-preview__content" aria-labelledby="g37-preview-title">
          <div className="g37-preview__breadcrumb"><span>{project.name}</span><span aria-hidden="true">/</span><span>{nav.find((item) => item.area === area)?.title}</span></div>
          <div className="g37-preview__heading-row">
            <div><h2 id="g37-preview-title">{areaTitle(area)}</h2><p>{project.path} · {project.branch}</p></div>
            <span className="g37-preview__source">当前项目：{project.id}</span>
          </div>

          <section className="g37-preview__status-card" aria-live="polite">
            <div><span className="g37-preview__section-label">状态样例 · {stateCard.id}</span><strong>{stateCard.label}</strong></div>
            <p>{stateCard.detail}</p>
          </section>

          {area === "project" ? <ProjectPanel project={project} session={session}
            onNavigate={selectArea} onScenario={chooseScenario} instance={instance} /> : null}
          {area === "seats" ? <SeatsPanel selectedId={seatId} onSelect={setSeatId} scenario={selectedScenario} /> : null}
          {area === "sessions" ? <SessionsPanel selectedId={sessionId} onSelect={setSessionId} /> : null}
          {area === "inbox" ? <InboxPanel status={visibleMessageStatus} text={messageText} setText={setMessageText}
            editing={editMessage} setEditing={setEditMessage} setStatus={setMessageStatus}
            requeuedId={requeuedId} setRequeuedId={setRequeuedId}
            checkNotice={messageCheckNotice} setCheckNotice={setMessageCheckNotice}
            onFocusChange={() => { setProjectId(projectId === projects[0].id ? projects[1].id : projects[0].id); }} /> : null}
          {area === "sidechat" ? <SideChatPanel chats={sideChats} selectedId={sideChatId} selected={sideChat}
            draft={sideChatDraft} setDraft={setSideChatDraft} onSelect={setSideChatId}
            onCreate={() => {
              const id = `sidechat-preview-${sideChats.length + 1}`;
              setSideChats((current) => [...current, { id, title: sideChatDraft || "新旁聊", status: "ACTIVE", ledger: "主控 turn 18", permission: "只读建议" }]);
              setSideChatId(id);
              setScenarioId("A3-ACTIVE");
            }} scenario={selectedScenario} onStatus={(status) => {
              setSideChats((current) => current.map((item) => item.id === sideChatId ? { ...item, status } : item));
              const scenarioForStatus: Record<string, string> = { KEPT: "A3-KEPT", ACTIVE: "A3-ACTIVE", ARCHIVED: "A3-ARCHIVED", DELETED: "A3-DELETED" };
              setScenarioId(scenarioForStatus[status] ?? "A3-ACTIVE");
            }} /> : null}
          {area === "questions" ? <QuestionPanel onSubmit={(requestValue, response) => {
            const answer = response.answers[requestValue.params.questions[0]?.id]?.answers.join(" / ") || "自由回答为空";
            setAnswerNotice(`本地预览答案“${answer}”绑定到 ${requestValue.params.thread_id} · ${requestValue.params.turn_id}；没有发送给宿主。`);
          }} answerNotice={answerNotice} /> : null}
          {area === "worktrees" ? <WorktreePanel selectedId={selectedWorktree} onSelect={setSelectedWorktree} project={project} /> : null}
          {area === "instances" ? <InstanceCard instance={instance} error={instanceError} /> : null}

          <footer className="g37-preview__disclaimer" role="note">
            <strong>预览边界</strong>
            <span>除实例状态卡读取现有 K-UI 假端口外，其余内容都是固定样例或本地界面状态。没有创建会话、旁聊、工作树或发送消息；V15 的真实边界验证仍为 NOT_RUN。</span>
          </footer>
          <div className="g37-preview__selected-context" aria-live="polite">
            当前选择：{project.name} · {seat.name} · {session.title} · 收件人固定 {seats[1].name} / turn 18
          </div>
        </section>
      </div>
    </main>
  </I18nProvider>;
}

function areaTitle(area: Area): string {
  return ({ project: "项目概览", seats: "席位", sessions: "会话", inbox: "收件箱", sidechat: "旁聊", questions: "问题卡", worktrees: "工作树图谱", instances: "实例状态" } as const)[area];
}

function ProjectPanel({ project, session, onNavigate, onScenario, instance }: {
  project: typeof projects[number]; session: typeof sessions[number]; onNavigate: (area: Area) => void;
  onScenario: (scenario: Scenario) => void; instance: Record<string, unknown> | null;
}) {
  return <div className="g37-preview__panel-grid">
    <section className="settings-toggle-row g37-preview__project-summary">
      <div><span className="g37-preview__section-label">进行中的工作</span><h3>{session.title}</h3>
        <p>界面施工 · {project.branch} · {session.turn}</p></div>
      <button type="button" className="primary settings-button-compact" onClick={() => onNavigate("sessions")}>查看会话</button>
    </section>
    <MiniCard label="席位活动" value="2 个 lead 层席位" detail="施工中 1 · 等待复核 1" onClick={() => onNavigate("seats")} />
    <MiniCard label="待处理消息" value="2 条排队" detail="收件人固定为界面施工席位" onClick={() => onNavigate("inbox")} />
    <MiniCard label="工作树" value="单席位树 · 1 个父节点" detail="所有关系都来自示例图谱" onClick={() => onNavigate("worktrees")} />
    <section className="settings-toggle-row g37-preview__project-summary">
      <div><span className="g37-preview__section-label">实例状态样例</span><h3>{String(instance?.instanceId ?? "Codex 测试实例")}</h3>
        <p>通过现有 K-UI 假端口读取，不是真实安装或登录状态。</p></div>
      <button type="button" className="ghost settings-button-compact" onClick={() => onScenario(scenarios.find((item) => item.id === "A4-LOGGED-IN")!)}>查看实例样例</button>
    </section>
  </div>;
}

function MiniCard({ label, value, detail, onClick }: { label: string; value: string; detail: string; onClick: () => void }) {
  return <button type="button" className="settings-toggle-row g37-preview__mini-card" onClick={onClick}>
    <span className="g37-preview__section-label">{label}</span><strong>{value}</strong><span>{detail}</span>
  </button>;
}

function SeatsPanel({ selectedId, onSelect, scenario }: { selectedId: string; onSelect: (id: string) => void; scenario: Scenario }) {
  const status = scenario.id.startsWith("A2-") ? scenario.label : "工作中";
  const blocked = ["A2-WORKING", "A2-SWITCHING", "A2-STOP-REQ", "A2-STUCK", "A2-RECLAIMING", "A2-RECLAIMED"].includes(scenario.id);
  return <div className="g37-preview__split">
    <div className="g37-preview__list" role="list" aria-label="席位状态卡">
      {seats.map((item) => <button key={item.id} type="button" aria-pressed={selectedId === item.id}
        className={`settings-toggle-row g37-preview__list-card${selectedId === item.id ? " is-selected" : ""}`}
        onClick={() => onSelect(item.id)}>
        <span className="g37-preview__avatar" aria-hidden="true">{item.name.slice(0, 1)}</span>
        <span><strong>{item.name}</strong><small>{item.role} · {item.layer} 层</small></span>
        <span className="g37-preview__state-tag">{selectedId === item.id ? status : item.status}</span>
      </button>)}
    </div>
    <section className="settings-toggle-row g37-preview__detail-card">
      <span className="g37-preview__section-label">席位状态卡 · {selectedId}</span>
      <h3>{seats.find((item) => item.id === selectedId)?.name}</h3>
      <p>{scenario.detail}</p>
      {scenario.id === "A2-STOP-REQ" ? <p role="status">正在等待 StopFact 示例；重叠工作保持禁用。</p> : null}
      <div className="settings-field-actions">
        <button type="button" className="primary settings-button-compact" disabled={blocked}>派发示例</button>
        <button type="button" className="ghost settings-button-compact" disabled={blocked}>更换实例</button>
        <button type="button" className="ghost settings-button-compact" disabled={!scenario.id.startsWith("A2-") || scenario.id !== "A2-WORKING"}>请求停止</button>
      </div>
      <p className="settings-help">操作控件只呈现状态约束，不发出席位操作。</p>
    </section>
  </div>;
}

function SessionsPanel({ selectedId, onSelect }: { selectedId: string; onSelect: (id: string) => void }) {
  return <div className="g37-preview__split">
    <div className="g37-preview__list" aria-label="会话列表">
      {sessions.map((item) => <button key={item.id} type="button" aria-pressed={selectedId === item.id}
        className={`settings-toggle-row g37-preview__list-card${selectedId === item.id ? " is-selected" : ""}`}
        onClick={() => onSelect(item.id)}>
        <Activity size={16} aria-hidden="true" /><span><strong>{item.title}</strong><small>{item.seatId} · {item.turn}</small></span>
        <span className="g37-preview__state-tag">{item.status === "A2-WORKING" ? "进行中" : "空闲"}</span>
      </button>)}
    </div>
    <section className="settings-toggle-row g37-preview__detail-card">
      <span className="g37-preview__section-label">会话记录</span><h3>{sessions.find((item) => item.id === selectedId)?.title}</h3>
      <ol className="g37-preview__timeline"><li><strong>已分配给界面施工</strong><small>示例记录 · turn 18</small></li><li><strong>待复核内容已加入收件箱</strong><small>示例记录 · 未发送</small></li></ol>
      <p className="settings-help">这里只展示会话导航和本地样例记录，不读取真实会话。</p>
    </section>
  </div>;
}

function InboxPanel({ status, text, setText, editing, setEditing, setStatus, requeuedId, setRequeuedId, checkNotice, setCheckNotice, onFocusChange }: {
  status: string; text: string; setText: (value: string) => void; editing: boolean; setEditing: (value: boolean) => void;
  setStatus: (value: string) => void; requeuedId: string | null; setRequeuedId: (value: string | null) => void;
  checkNotice: string; setCheckNotice: (value: string) => void; onFocusChange: () => void;
}) {
  const queued = status === "A1-QUEUED" || status === "A1-TURN-ENDED";
  const statusCopy = scenarios.find((item) => item.id === status)?.detail ?? "本地消息样例";
  return <div className="g37-preview__split">
    <div className="g37-preview__list" aria-label="消息队列">
      <button type="button" aria-current="true" className="settings-toggle-row g37-preview__list-card is-selected">
        <span className="g37-preview__unread" aria-hidden="true" /><span><strong className={status === "A1-CANCELLED" ? "is-struck" : ""}>商品卡片反馈</strong>
          <small>收件人：界面施工 · turn 18</small></span><span className="g37-preview__state-tag">{scenarios.find((item) => item.id === status)?.label ?? "排队中"}</span>
      </button>
      {requeuedId ? <button type="button" aria-current="false" className="settings-toggle-row g37-preview__list-card">
        <span><strong>重新排队的反馈</strong><small>新消息 {requeuedId} · 原失败记录仍保留</small></span><span className="g37-preview__state-tag">排队中</span>
      </button> : null}
    </div>
    <section className="settings-toggle-row g37-preview__detail-card">
      <span className="g37-preview__section-label">消息状态 · {status}</span>
      {editing ? <label className="g37-preview__field">编辑排队消息<textarea value={text} onChange={(event) => setText(event.target.value)} /></label>
        : <p className={status === "A1-CANCELLED" ? "is-struck" : ""}>{text}</p>}
      <p className="settings-help">收件人锁定：界面施工 · turn 18。项目焦点切换不会改变目标。</p>
      <p>{statusCopy}</p>
      {status === "A1-DELIVERED" ? <p role="status">示例回执时间：14:32 · interrupt-and-resume 由原回执说明。</p> : null}
      {status === "A1-UNKNOWN" ? <button type="button" className="ghost settings-button-compact" onClick={() => setCheckNotice("只读核对已显示；没有重新发送。")}>只读核对状态</button> : null}
      {checkNotice ? <p role="status">{checkNotice}</p> : null}
      <div className="settings-field-actions g37-preview__actions">
        {queued ? <><button type="button" className="ghost settings-button-compact" onClick={() => setEditing(!editing)}>{editing ? "保存本地草稿" : "编辑"}</button>
          <button type="button" className="primary settings-button-compact" onClick={() => { setStatus("A1-STEERING"); setEditing(false); }}>插话</button>
          <button type="button" className="ghost settings-button-compact" onClick={() => { setStatus("A1-CANCELLED"); setEditing(false); }}>取消</button></> : null}
        {status === "A1-STEERING" ? <button type="button" className="ghost settings-button-compact" onClick={() => setStatus("A1-TURN-ENDED")}>演示回合结束</button> : null}
        {status === "A1-FAILED" ? <button type="button" className="primary settings-button-compact" onClick={() => setRequeuedId(`preview-message-${Date.now()}`)}>重新排队</button> : null}
        <button type="button" className="ghost settings-button-compact" onClick={onFocusChange}>切换项目焦点</button>
      </div>
      <p className="settings-help">按钮只改变页面样例；“已送达”只能从状态样例菜单查看，不由本地插话动作伪造回执。</p>
    </section>
  </div>;
}

function SideChatPanel({ chats, selectedId, selected, draft, setDraft, onSelect, onCreate, onStatus, scenario }: {
  chats: sideChatsType; selectedId: string; selected: sideChatsType[number] | undefined;
  draft: string; setDraft: (value: string) => void; onSelect: (id: string) => void; onCreate: () => void;
  scenario: Scenario; onStatus: (status: string) => void;
}) {
  const status = scenario.id.startsWith("A3-") ? scenario.id : selected?.status ?? "A3-DELETED";
  return <div className="g37-preview__split">
    <div className="g37-preview__list" aria-label="旁聊列表">
      {chats.filter((item) => item.status !== "DELETED").map((item) => <button key={item.id} type="button" aria-pressed={selectedId === item.id}
        className={`settings-toggle-row g37-preview__list-card${selectedId === item.id ? " is-selected" : ""}`} onClick={() => onSelect(item.id)}>
        <MessagesSquare size={16} aria-hidden="true" /><span><strong>{item.title}</strong><small>{item.ledger} · {item.permission}</small></span><span className="g37-preview__state-tag">{item.status}</span>
      </button>)}
      <label className="g37-preview__field">新旁聊标题<input value={draft} onChange={(event) => setDraft(event.target.value)} /></label>
      <button type="button" className="primary settings-button-compact" onClick={onCreate}>创建本地样例旁聊</button>
    </div>
    <section className="settings-toggle-row g37-preview__detail-card">
      <span className="g37-preview__section-label">旁聊状态 · {status}</span>
      <h3>{selected?.title ?? "此旁聊已删除"}</h3>
      <p>项目 {projects[0].name} · {selected?.ledger ?? "原主控账本保持不变"}</p>
      <p>权限层：{selected?.permission ?? "无"} · 同步的内容只作为上下文，不会执行。</p>
      {status === "A3-LEAD-PROGRESS" ? <p role="status">主控有新差异；会在下一问之前作为待同步内容显示。</p> : null}
      <div className="settings-field-actions">
        {status === "A3-ARCHIVED" ? <button type="button" className="primary settings-button-compact" onClick={() => onStatus("KEPT")}>恢复旁聊</button>
          : status === "A3-ACTIVE" || status === "A3-KEPT" ? <button type="button" className="ghost settings-button-compact" onClick={() => onStatus("ARCHIVED")}>归档</button> : null}
        {selected && status !== "A3-DELETED" ? <button type="button" className="ghost settings-button-compact" onClick={() => onStatus("DELETED")}>删除此旁聊</button> : null}
      </div>
      <p className="settings-help">创建、归档、恢复、删除都只变更本地样例；不会调用 K-SIDE 或模型。</p>
    </section>
  </div>;
}

type sideChatsType = { id: string; title: string; status: string; ledger: string; permission: string }[];

function QuestionPanel({ onSubmit, answerNotice }: {
  onSubmit: (request: RequestUserInputRequest, response: RequestUserInputResponse) => void; answerNotice: string;
}) {
  return <div className="g37-preview__question-layout">
    <section className="settings-toggle-row g37-preview__detail-card">
      <span className="g37-preview__section-label">原有问题卡组件 · RequestUserInputMessage</span>
      <p>合成请求样例 · 席位 界面施工 · turn 18 · item item-question-demo</p>
      <RequestUserInputMessage requests={[request]} activeThreadId={request.params.thread_id}
        activeWorkspaceId={request.workspace_id} onSubmit={onSubmit} />
      {answerNotice ? <p role="status">{answerNotice}</p> : null}
    </section>
    <aside className="settings-toggle-row g37-preview__detail-card">
      <span className="g37-preview__section-label">绑定信息</span>
      <strong>{request.params.thread_id}</strong><span>{request.params.turn_id}</span>
      <p>自由回答和选项都在原问题卡里输入；提交后只显示绑定目标，不发往宿主。</p>
    </aside>
  </div>;
}

function WorktreePanel({ selectedId, onSelect, project }: { selectedId: string; onSelect: (id: string) => void; project: typeof projects[number] }) {
  const nodes = [
    { id: "wt-project-root", title: project.name, kind: "项目根", status: "已登记" },
    { id: "wt-ui-single", title: "界面施工树", kind: "SINGLE · 界面施工", status: "使用中" },
    { id: "wt-review-single", title: "界面复核树", kind: "SINGLE · 界面复核", status: "等待中" },
    { id: "wt-mixed-demo", title: "集成检查树", kind: "MIXED · 示例节点", status: "未运行" },
  ];
  const node = nodes.find((item) => item.id === selectedId)!;
  return <div className="g37-preview__graph-layout">
    <section className="settings-toggle-row g37-preview__graph" aria-label="示例工作树关系图">
      <button type="button" aria-pressed={selectedId === nodes[0].id} className={`settings-toggle-row g37-preview__graph-node${selectedId === nodes[0].id ? " is-selected" : ""}`} onClick={() => onSelect(nodes[0].id)}>
        <strong>{nodes[0].title}</strong><small>{nodes[0].kind} · {project.branch}</small>
      </button>
      <div className="g37-preview__graph-children">
        {nodes.slice(1).map((item) => <button key={item.id} type="button" aria-pressed={selectedId === item.id}
          className={`settings-toggle-row g37-preview__graph-node${selectedId === item.id ? " is-selected" : ""}`} onClick={() => onSelect(item.id)}>
          <strong>{item.title}</strong><small>{item.kind} · {item.status}</small>
        </button>)}
      </div>
    </section>
    <section className="settings-toggle-row g37-preview__detail-card">
      <span className="g37-preview__section-label">图谱状态卡 · {node.id}</span><h3>{node.title}</h3>
      <p>{node.kind} · {node.status}</p>
      <p>只显示预置关系样例；不会调用创建、注册、合并或清理操作。</p>
      <button type="button" className="ghost settings-button-compact" disabled>打开真实工作树</button>
    </section>
  </div>;
}

function InstanceCard({ instance, error }: { instance: Record<string, unknown> | null; error: string }) {
  return <section className="settings-toggle-row g37-preview__instance-card">
    <span className="g37-preview__section-label">K-UI 实例读模型样例</span>
    {error ? <p role="alert">假读模型错误：{error}</p> : instance ? <>
      <strong>{String(instance.instanceId)} · {String(instance.state)}</strong>
      <span>{String(instance.driverId)} · CLI {String(instance.version)} · 修订 {String(instance.revision)}</span>
      {typeof instance.newVersion === "string" ? <span>手动升级提示：{instance.newVersion}</span> : null}
      {Array.isArray(instance.runtimeIssues) ? <ul>{instance.runtimeIssues.map((issue, index) => <li key={index}>{String((issue as Record<string, unknown>).reason)}</li>)}</ul> : null}
    </> : <p role="status">正在读取 K-UI 假读模型…</p>}
  </section>;
}
