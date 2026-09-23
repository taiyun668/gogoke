# Omnigent、Claudexor 源码审计

## Omnigent

### 实际 Harness 深度

Omnigent 不是只有一个 `run(command)` 包装。`omnigent/harness_capabilities.py` 明确声明：

- integration mode：SDK in-process、CLI subprocess、ACP subprocess、native TUI、native server；
- elicitation：hook、JSON-RPC、approval mirror、SSE permission；
- resume：none、warm reattach、cold rebuild；
- auth model、fork history、interrupt、streaming、steering、live queue、image、compaction。

`omnigent/runtime/harnesses/_scaffold.py` 进一步统一了 session-scoped event endpoint、单 turn 并发、心跳、interrupt、steering、tool result、approval、graceful shutdown、idle watchdog 和 absolute watchdog。未知事件 fail loud，stale tool result 才按可解释的竞态 no-op。

源码中存在 Claude、Codex、Cursor、Pi、OpenCode、Hermes、Goose、Qwen、Kimi、Copilot、ACP 等 native/SDK harness；测试目录有专门 Harness Bench，探测 basic turn、tool calling、streaming、policy allow/deny/ask、interrupt、fork replay、cost tracking 和 MCP。状态：`SOURCE + TEST-SOURCE / NOT-RUN`。

### 上下文和 Prompt

`omnigent/runtime/prompt.py:20-92` 规定唯一 Prompt 顺序：Agent 基础指令 → per-request 指令 → skills metadata → framework-owned 指令。框架治理不复制进每个 adapter。这直接支持 GOGO 的“通版提示词 + 项目覆盖 + 任务覆盖 + 控制平面元数据”模型。

Conversation 实体把 parent/root、agent、runner、host、labels、session state、usage、external session ID、workspace 和 project 作为显式字段；runner 首次认领使用 CAS，workspace 查询带 workspace scope。它证明 Session、Harness 原生 ID、Runner affinity 与 Project filing 可以同时存在，而不必复制完整对话到 Git。

### 风险与边界

- 这是一个很大的平台，不适合整块嵌入 MVP；应只借鉴 capability vocabulary、event contract、prompt composition、fork/resume 和 policy bridge。
- native Harness 的可控性不同。源码明确承认 Antigravity native 只能事后审计，无法在工具执行前阻断；GOGO 的 capability 必须区分 `preventive`、`approval-capable`、`post-hoc-only`。
- Windows 是**降级模式**：server、Web UI 和 SDK-based harness 可运行，并用 Job Object 管进程树；native tmux/PTY harness 不可用，文件系统和网络隔离也不等价于 Linux/macOS。不能用“支持 Windows”掩盖能力差异。
- 当前 session closed 还兼容 title suffix 迁移逻辑，说明展示字段不应承担生命周期权威状态。

## Claudexor

### 最值得借鉴的 Adapter Kernel

`packages/core/src/adapter.ts:42-99` 的 `HarnessAdapter` 把 discover、doctor、run、review、models、cancel 和 per-credential probe 分开，并明确“Adapter 只把原生 I/O 转成 typed events，不包含 orchestration logic”。这是 GOGO Adapter API 的最佳起点。

其 capability schema 比布尔表更严格：

- capability 只有在 engine 中存在消费者时才能进入契约；
- model/effort 能力带真值来源和验证版本；
- auth source、credential transport、credential relocation 独立声明；
- readonly 区分 fs sandbox、permission deny、tool allowlist、none；
- web access 区分 native、tools、uncontrolled、none；
- work report 区分 constrained、validated、unsupported；
- 明确 `unknown` signal quality，不把无证据当 false。

### ContextPack 与终态

- `packages/context/src/contextpack.ts:15-109` 构造可哈希 ContextPack，记录 included/omitted、指令来源和 token budget；显式 mandatory 文件缺失、越界、symlink 或敏感内容时 fail closed。
- `packages/core/src/adapter.ts:104-125` 的 InteractionChannel 暴露 pending count 和 suspension version，watchdog 可识别“正在等人”而不是误杀静默 Session。
- `packages/event-log/src/index.ts` 使用 append-only JSONL、递增 seq、秘密递归脱敏，并对 terminal receipt 做 durable journal → canonical receipt → per-run event 的提交栅栏；提交后本地 finalization 失败会 poison/fail closed，而不是再次写一个相互冲突的终态。
- `packages/orchestrator/src/harnessFailure.ts` 把 auth、capability、config、rate limit、timeout、process crash 分开，给不同下一步；这是 Attention Inbox reason code 的直接参考。

### 风险与边界

- Claudexor 更偏单次软件任务执行和质量验证，不是完整多项目 Party 面板。
- macOS GUI 与 Node CLI/daemon 是不同成熟度路径；不能据 macOS App 推断 Windows GUI 完整。
- 它强调产物、WorkReport 和终态一致性，正适合 GOGO 的 Result Capsule，但长期 Agent 身份、动态 Party 和跨项目 UI 仍需我们自己设计。

## 两者合并后的借鉴原则

Omnigent 提供“广度和真实 Session 连接”，Claudexor 提供“窄而严格的适配器/证据契约”。GOGO PARTY 应采用 Claudexor 风格的最小核心契约，再用 Omnigent 风格的 capability 扩展，而不是把所有 Harness 特例写进 Controller。
