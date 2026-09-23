# GOGO PARTY 上游复用蓝图

- 日期：2026-08-11；参考车道补充：2026-08-20
- 范围：合并已完成的上游源码审计、Grok App 源码复用审计，以及 Hermes Studio / DeepSeek Harness 参考-only 车道
- 原则：按模块选择性借鉴，不引入任何上游作为总控制器。`PRODUCT-PATTERN` / `SEMANTIC-PORT` 不是生产准入

## 1. 复用等级

| 等级 | 含义 |
|---|---|
| `SOURCE-CANDIDATE` | 许可已确认且源码边界清楚；依赖、NOTICE、平台和测试审计通过后才可升级为派生源码 |
| `SOURCE-DERIVED` | 已选择固定 commit 和具体文件，完成 provenance、依赖与测试 Gate，由 GOGO 自己构建和发布 |
| `SEMANTIC-PORT` | 移植状态机、协议、测试思想和不变量，不复制具体存储/UI/运行时实现 |
| `PRODUCT-PATTERN` | 只借信息架构、交互和产品表达，重新实现 |
| `REJECT` | 明确不进入 GOGO 的依赖、权威模型或产品路径 |

已确认所有列入矩阵的仓库顶层许可证，但尚未完成每个候选模块的依赖、NOTICE、平台和供应链闭包。因此可复制候选仍先标为 `SOURCE-CANDIDATE`，不能因为“开源”就直接复制。2026-08-20 的 Hermes Studio 与 DeepSeek Harness 不是这一档：前者因 BSL 1.1 商业边界以 `PRODUCT-PATTERN` / `SEMANTIC-PORT` / `REJECT` 记录；后者因 developer preview 与 Control Plane 禁令以 Adapter 目标 `SEMANTIC-PORT` 记录。两者都不是生产准入。

## 2. 目标模块与主要来源

| GOGO PARTY 模块 | 主参考 | 复用内容 | 当前等级 |
|---|---|---|---|
| Task Ledger | Beads | Task/Dependency、ready query、原子 claim、lease、heartbeat、CAS、indeterminate commit、provenance | `SEMANTIC-PORT` |
| Workflow Engine | Gas City、agtx | attempt 分类、retry/exhaust、quarantine、幂等下一步、人工/一键/自动共用状态机 | `SEMANTIC-PORT` |
| Party/Role 模型 | Gas Town、Gas City | 通版角色、项目覆盖、成员覆盖、动态 fan-out/fan-in、handoff | `SEMANTIC-PORT` |
| Adapter Kernel | Claudexor | discover/doctor/run/review/cancel、typed events、terminal receipt、failure taxonomy | `SEMANTIC-PORT` |
| Capability Registry | Omnigent、Claudexor、Mission Control | integration mode、resume、fork、steering、approval、event quality、enforcement、evidence freshness | `SEMANTIC-PORT` |
| Runner Supervisor | xAI Grok Build、OpenAI Codex、Grok App、Gas City | lifecycle、ProcessScope/PTY/Windows Job、watchdog、stall、interrupt、Session 治理 | 小型 Rust 部件 `SOURCE-CANDIDATE`；治理 `SEMANTIC-PORT` |
| Codex Adapter | OpenAI Codex、Clay、AgentDeck | 官方 app-server protocol/schema/tests，事件映射经验 | 官方协议为 Adapter-only `SOURCE-CANDIDATE` |
| Grok Adapter | xAI Grok Build、Grok App | 官方 ACP typed transport/test support，host-side reverse RPC、resume、usage、terminal-event fence | `SOURCE-CANDIDATE`，不得连接 Grok App |
| Context Compiler | Omnigent、Claudexor、Clay、Open Harness | Prompt precedence、ContextPack、included/omitted manifest、指令投影、mandatory fail-closed | `SEMANTIC-PORT` |
| Session Governor | Gas City、Grok App、Omnigent | task boundary、stall、wait、age、rate limit、crash/churn、context pressure、resume identity | `SEMANTIC-PORT` |
| Repository Projector | Beads、Gas City、Gas Town | Task/Result/Fact/Decision 投影、handoff、revision/provenance；Git 不承担实时 claim | `SEMANTIC-PORT` |
| Attention Inbox | Mission Control、Claudexor、Grok App | approval、question、plan、auth、rate limit、stall、crash、unknown reason codes | `PRODUCT-PATTERN` + typed semantics |
| Operations Dashboard | Mission Control | Projects、Agents、Tasks、Activity、Cost、Audit、Security、Approvals 的信息架构；Hermes Studio 审批/工作流注意力面为附加 `PRODUCT-PATTERN` | `PRODUCT-PATTERN` |
| Fleet/Session UI | AgentPulse、AgentDeck、CliDeck、Squad、Gas Town | 多主机/多 Session 状态、筛选、快速 attach、Party 可视化 | `PRODUCT-PATTERN` |
| Group Chat UX | Hermes Studio | 多 Agent 群聊、结构化 mention、relay 与上下文投影的产品表达；不得用 transcript 推断收件人 | `PRODUCT-PATTERN` |
| Desktop packaging/update | Hermes Studio | Electron updater、本地 runtime 托管、WebUI 托管的产品缝；不得成为 Controller | `PRODUCT-PATTERN` |
| One-click Workflow UI | agtx | 阶段按钮、可预览命令、人工与自动使用同一 Command；Hermes Studio 工作流 UX 为附加模式，调度所有权仍归 GOGO PARTY lease | `PRODUCT-PATTERN` |
| Future DSH Adapter | deepseek-ai/deepseek-harness | Cordis 可逆插件思想、append-only SessionEvent→`AdapterEvent.nativeSeq`、resume/fork/replay、Codex/Claude Code/ACP/DSH SDK subagent、Typert/API 与 stdio JSON-RPC；subagent finish 只进 `NativeTerminalEvidence` | Adapter 目标 `SEMANTIC-PORT`；永不作为 Control Plane |
| Sandbox/Instruction Projection | Open Harness、Claudexor、Clay、Squad | manifest/path guard、ContextPack、Harness 文档生成、provider path、hash 和漂移提示；DSH Windows ACL 仅 partial 能力证据，不得写成完整隔离 | 小型纯函数 `SOURCE-CANDIDATE`；整体 `SEMANTIC-PORT`；DSH ACL `REJECT` 作为隔离完成证明 |
| Future GOGO Web Control | Grok App `mirror/` 的源码结构 | Axum/WebSocket、token、fan-out、只读模式的实现思路；协议必须重写为 GOGO 自有 API | `SOURCE-CANDIDATE`，不接入 Grok App |

## 3. MVP 最值得复用的六块

1. **官方 Runtime bake-off**：比较 Grok Build `ProcessScope` 与 Codex PTY/Windows Job，选一套公共进程基础；Grok App primitive 作为补充候选。
2. **Grok Build lifecycle + Claudexor Adapter Contract**：内部 contributor registry 与外部最小严格接口分层，不把 orchestration 写进 Adapter。
3. **Omnigent Capability Vocabulary**：表达不同 Harness 的真实差异，并用 Harness Bench 验证。
4. **Beads Claim/Lease/CAS**：实现可靠派活和自动接力，Git 只接收事实投影。
5. **Gas City Session Governance**：把 UNKNOWN、等待交互、卡死、年龄、上下文漂移和 crash churn 纳入换 Session 判断。
6. **Mission Control UI IA**：用一个面板承载 Project、Party、Task、Attention、Audit 和 Cost，但底层状态改用 GOGO 自己的四平面模型。

Hermes Studio 与 DeepSeek Harness 不进入上述六块 MVP 复用。前者只在面板阶段提供 `PRODUCT-PATTERN`；后者只在后续 Adapter 扩展中作为执行层候选，且必须遵守 nativeSeq / NativeTerminalEvidence 边界。

## 4. 组合后的 GOGO 内核

```text
Mission Control / AgentDeck 风格的指挥面板
                  |
      GOGO Party + Workflow Engine
      (Gas City / Gas Town / agtx)
                  |
   Task Ledger + Claim/Lease/CAS/Provenance
                 (Beads)
                  |
 Adapter Contract + Capability Evidence + Bench
          (Claudexor / Omnigent)
                  |
 Runner Supervisor + Context/Session Governor
   (Grok Build / Codex / Gas City / Grok App)
                  |
  Codex Adapter | Grok Adapter | Future Adapters
  （DeepSeek Harness 若进入，只作为 Future Adapter 目标，
    不得上移为 Control Plane、账本或 Transition Engine）
```

这不是把上游堆在一起。GOGO PARTY 自己拥有 Project、Agent、Assignment、Attempt、Gate、Fact Promotion 和 Controller authority；上游只贡献已经被证明有效的局部机制。

## 5. 明确拒绝

- 不接入 Grok App 壳、进程、RPC、IPC、store 或原生 Session。
- 不把 Omnigent、Mission Control 或其他上游整体嵌入为 GOGO 后端。
- 不要求 tmux、Dolt、某个云服务或单一 Harness 成为 Core 依赖。
- 不采用 Gas Town 固定角色体系；角色和 Auditor 数量由项目自由组织。
- 不把 Mission Control 的进程内 EventEmitter 当持久消息总线。
- 不用时间戳、终端最后几行或自然语言“完成了”提升任务状态。
- 不让 Prompt 决定权限、事实提升、claim、lease 或自动接力等确定性治理。
- 不在模块级 provenance、依赖、NOTICE 与测试 Gate 完成前复制候选源码。
- 不采用 AgentPulse 当前 select-then-update launch claim、Squad 无 fencing lease、AgentDeck 无跨进程锁 registry 或 CliDeck PTY ask 作为调度内核。
- 不同时引入多套重叠 PTY/process supervisor；先 bake-off，再固定唯一公共基础。
- 无单独商业许可时，不复制 EKKOLearnAI/hermes-studio（BSL 1.1，Additional Use Grant 仅非商业，Change Date 2029-05-10）源码，不接入其 desktop/server runtime。
- 不把 DeepSeek Harness 当作 GOGO PARTY Control Plane；不把其 job/goal/plan/schedule/worker 替换耐久账本、lease/fencing、spool、`TerminalReceipt`/`ReceiptRecord`/Gate、Project 隔离、Context `LoadProof` 或 Transition Engine。
- DSH SessionEvent 序号只映射到 `AdapterEvent.nativeSeq`；subagent 结束只映射到 `NativeTerminalEvidence`。
- 不把 DSH Windows ACL 包宣传为完整隔离；能力保持 partial，不支持或不完整时 fail-closed。
- 不把参考负向矩阵写成已实现测试；场景保持 `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`，不改可执行 Fake 套件。
- 现有文档中的 “Hermes” 高层助理与 `EKKOLearnAI/hermes-studio` 不是同一对象；后者只提供 Studio 产品参考。

## 6. 推荐实施顺序

1. 完成 Process runtime、Lifecycle/Adapter、Context projection 三个有限 bake-off，并提交通用内核选择 ADR。
2. 定义 Claudexor 风格 Adapter Contract、Grok Build 风格 lifecycle contributor 和 Omnigent 风格 CapabilityEvidence。
3. 实现 Beads 风格且带 monotonic fencing token 的最小 Task/Dependency/Claim/Lease/CAS 内核。
4. 建立唯一公共 Runner Supervisor；只移植 bake-off 获胜的进程部件和必要 primitive。
5. 分别用官方 Codex app-server 与公众 Grok CLI ACP 实现两个 Adapter，并跑同一套 golden/conformance。
6. 实现 Context Projector 及 Harness 加载 doctor，再完成 Implementer → Auditor → Gate → 下一任务的自动接力闭环。
7. 再用 Mission Control、AgentPulse、AgentDeck、CliDeck、Hermes Studio 等产品模式建设统一面板；Studio 只借 IA，不接入 runtime。
8. 最后扩展 Pi、Z Code、OpenCode、可替换 Hermes 式助理和 GOGO 自有 Web 控制面。DeepSeek Harness 若进入，只作为独立 Adapter 目标，且必须先钉死 commit/包版本并满足 nativeSeq 与 NativeTerminalEvidence 边界。
