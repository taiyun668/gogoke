# Mission Control 源码审计

## 成立的优点

- 产品面完整：Projects、Agents、Tasks、Chat/Sessions、Activity、Logs、Cost、Audit、Security、Exec Approval 等面板已经形成“指挥中心”体验。
- SQLite schema 和 API 大量使用 `workspace_id`，并存在 workspace isolation、安全和 auth 测试。
- `src/lib/task-dispatch.ts:1618-1627` 用条件 UPDATE 原子认领 assigned task，避免并发 scheduler 双派发。
- task dispatch 已有 deferred completion reconciliation、Aegis review/requeue、重试上限、workspace cwd realpath 边界、Claude base-session/fork 和部分 token usage。
- `src/lib/agent-runtimes.ts:262-357` 的 runtime capability matrix 是诚实设计：每个 true 必须指向实际代码路径，未知时默认 false。
- Windows 有本地 PowerShell installer，CLI dispatch 也处理 `.exe` 与 argv 长度；作为面板和本地服务参考价值高。

## README 容易造成的高估

### Framework Adapter 很薄

`src/lib/adapters/adapter.ts:60-66` 的统一接口只有 register、heartbeat、reportTask、getAssignments、disconnect。OpenClaw、CrewAI、LangGraph、AutoGen、Claude SDK 和 generic 的实现基本都是同构 EventBus 转发；它们没有各自的启动、Session resume、interrupt、transcript normalization、permission bridge 或 capability probe。

所以“支持这些框架”在这层更接近**接收标准回报的接入协议**，不是 Omnigent/Claudexor 那种真实 Harness 驱动层。

### EventBus 不是持久总线

`src/lib/event-bus.ts:1-79` 是单进程 `EventEmitter`，用途明确是把数据库 mutation 广播到 SSE。它没有持久化、ack、offset、replay 或跨进程语义，不能作为自动接力的权威消息总线。

### Runtime 深度不对称

源码自己的矩阵说明：

- OpenClaw：可 dispatch/resume/PTY，但无 cwd、tool policy、budget cap 和结构化结果；
- Hermes：不可 dispatch，也不可 resume/PTY；
- Claude：可 dispatch/resume/cwd/tool policy/budget/JSON；
- Codex：只可 dispatch，其他均 false；
- OpenCode：只读扫描，不可 dispatch。

这与“统一管理所有 CLI Harness”的目标还有明显距离。它适合作为 UI/Control Plane 参考，不适合作为 GOGO 的 Adapter Kernel。

### 状态模型仍有启发式

`src/lib/sessions.ts` 从 OpenClaw `sessions.json` 读取时间戳，以 5 分钟/1 小时阈值映射 active/idle/offline；读取失败直接跳过。它没有把 unavailable 显式投影为 UNKNOWN。GOGO 不应复制这种三态推断作为权威状态。

### Windows PTY 是部分支持

Windows 能运行面板，但 `src/app/api/pty/setup/route.ts:36-53` 仍把 tmux 作为 terminal ready 条件，自动安装只支持 macOS/Linux。Windows 上的“面板可运行”和“能 attach 所有 CLI Session”必须分开标注。

## 应借鉴和不应借鉴

应借鉴：面板信息架构、任务板、Attention/Audit/Cost 视图、workspace-scoped 查询、原子 task claim、CLI dispatch sandbox 边界、诚实 capability UI。

不应借鉴：把同构事件转发器命名成多个成熟 Adapter、把 in-memory SSE bus 当自动化总线、以时间戳推断权威状态、在 capability 不足时静默降级到别的 provider。
