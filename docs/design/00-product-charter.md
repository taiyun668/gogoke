# 产品章程

- 状态：v0.4-p31-d1-product-contract-draft
- 产品名：GOGO PARTY
- 当前范围：CLI Harness；Web Chat Agent 仅保留扩展边界
- P31：D1 产品契约草案；Owner `ACCEPTED` 前不得作为实现授权

## 1. 问题定义

使用多个 AI Harness 和模型时，人类被迫在多个窗口之间搬运目标、仓库状态、任务、证据、审计结果和授权边界。人类实际上承担了消息总线、上下文数据库、调度器和冲突检测器的职责。

多模型不是临时过渡。不同模型和 Harness 在实施、研究、审计、长上下文、工具使用、成本和供应商可用性上长期存在差异，因此产品目标不是消灭差异，而是治理差异。

## 2. 产品定义

GOGO PARTY 是：

> 以持久协调内核驱动活跃任务，以仓库承载可审阅项目事实，以 Agent 为隔离执行单元，以 Party 为动态组织方式的多项目指挥中心。

产品将人类从信息搬运者提升为目标制定者、例外处理者和最终决策者。

## 3. 已冻结的产品意图

以下意图来自产品讨论，后续设计不得无声改变：

1. 一个控制面可以同时管理一个或多个 Project。
2. 每个 Project 有独立仓库、Party、Agent、Session、任务、运行状态、权限和成本边界。
3. Party 按项目自由组织；同一 Role 可以实例化一个或多个 Agent，例如一个或十个 Auditor。
4. Role Prompt 可以使用通用模板，也可以由 Project 或 Agent 做受控覆盖。
5. 创建 Agent 时，系统自动为目标 Harness 生成其能识别的项目文档和上下文。
6. 仓库中保存结构化成果与事实，不要求重建每一段对话。
7. 本地 Git 仓库和带 GitHub Remote 的仓库都可作为持久协作载体。
8. 任务创建、启动和完成回执既可人工触发，也必须支持一键与策略化自动触发。
9. 上一 Agent 的有效结果信号可由本地回执或仓库变更触发；经去重、权限、revision 与 Gate 验证后，使用同一 Transition Engine 启动下一 Agent。
10. Session 轮换由任务状态、事实漂移和上下文健康共同决定，不能只等待 Token 用满。
11. 不同 Project 默认不可互见；同一 Project 内 Agent 也按职责与阶段隔离。
12. 人类可以随时暂停自动化、否决结果、修改 Party 或在 Gate 处接管。

### 3.1 P31 D1 产品约束草案（非授权）

以下约束由已冻结意图与 P31 D0 十一类缺口推导，并在 D1 收成单一草案。它们在独立 fresh audit `PASS` 与 Owner `ACCEPTED` 绑定同一 contract-manifest digest 之前不得作为实现授权，也不得写成“已冻结”：

13. Assignment 可以表达 Harness/模型能力和成本偏好，但每个 Attempt 必须绑定精确 ExecutionTarget；模型、Profile、账号、Host 或协议等级不得静默替换。已有工具活动或 workspace 修改时，显式 fallback 必须新 Attempt/target generation、重新编译 Context，并经人工 Gate。
14. Windows Desktop UI 是本地 Control Host 的客户端；关闭窗口不能改变已在途任务的权威状态。`Close window`、`Pause automation`、`Stop current attempt`、`Quit` 和 `Force terminate` 必须明确区分。
15. 面向大众的首发产品必须提供可恢复的首次启动：operator/auth、单 Source Repository 的 Project、Harness detection + manual official guide、用户可见登录、模板复制后受控编辑、加密 backup/restore 和卸载默认保留数据。不得要求用户理解内部进程，也不得隐式安装或登录。
16. 跨 Project 共享的 Host、Runner、Provider、Profile 和模型并发容量由 Controller 的 ResourceBroker 统一仲裁，使用可租约、可 fencing、可恢复的 CapacityPermit；Adapter 不能自行选择受益 Project。等权双 Project 在持续合格时最多被连续旁路 2 次 grant decision。
17. MVP 一个 Project 恰好一个 Source Repository；后继多仓 revision 必须绑定 `repository_binding_id`。
18. `ContextEntry`/`CollaborationProposal` 是协作权威对象；`ReactionSignal` 不进入 MVP 权威面。Context Compiler 只消费冻结候选集；MVP 仅提供结构化筛选。
19. `operator_id` 与当前 OS principal 绑定但不等价；本地进程存在不能自动等同于被授权用户。
20. Local Control Host 是当前 OS principal 的 per-user 后台进程，不是 Windows Service；默认按需启动，登录自启 opt-in。
21. Chief of Staff 只作为 post-MVP proposal-only 能力，不得拥有隐藏的任务、审批、权限、删除或终态 authority。

## 4. 产品边界

### 4.1 GOGO PARTY 负责

- Project、Party、Role、Agent 和 Session 生命周期
- 本地与远端仓库绑定
- 权威事实、任务、结果、证据和决策的 Schema
- 活跃任务的依赖、原子认领、租约、命令幂等、回执与故障协调
- 工作流、Gate、fan-out、fan-in、重试和自动接力
- Harness 探测、启动、状态归一化、停止、恢复和轮换
- ExecutionTarget、ModelProfile、ProviderReadiness、成本/额度证据与显式 fallback 治理
- Context Pack 生成及 Harness 原生文档投影
- Worktree、读写范围、凭据、网络和结果可见性隔离
- Attention Inbox、状态面板、结果比较、成本与审计时间线
- Runner 离线、重复事件、进程崩溃和仓库分叉后的恢复
- Local Control Host 单实例、UI 重连、Windows 重启、通知、备份/恢复和卸载数据边界

### 4.2 Harness 负责

- 模型调用和原生会话
- 原生工具与权限请求
- 推理过程和临时对话历史
- Harness 自己支持的 resume、checkpoint、usage 和 structured output

### 4.3 v0.2 不负责

- 自动控制封闭网页聊天产品
- 复制或同步所有 Harness 的完整对话历史
- 替代 Git、GitHub、CI、IDE 或模型供应商
- 无人监管地绕过本地权限、仓库保护或人工 Gate
- 面向组织的计费、多租户 SaaS 和公开 Agent 市场

## 5. 设计原则

### 5.1 Repository-backed, not transcript-backed

正式历史由 Task、Fact、Result、Evidence、Decision 和 Event 构成。对话仅作为 Harness 私有运行材料，除非显式提取为带来源的成果。

这里的 repository-backed 不表示“只用 Git 作为数据库”。Git 保存可审阅、可交换的事实账本；活跃任务的 claim、lease、command 和消费位点由持久协调数据库保证原子性。

### 5.2 Persistent actor, disposable session

Agent 的身份、职责、能力和工作历史持续存在；Session 可以因任务边界、状态漂移或上下文健康而安全替换。

### 5.3 Proposal before promotion

Agent 输出永远先是 Proposal。通过 Schema、权限、证据和 Workflow Gate 验证后，才可成为 Accepted Result 或 Canonical Fact。

### 5.4 Same engine for manual and automatic

人工按钮和自动事件调用同一个 Transition Engine。人工模式只改变谁触发，不改变验证规则。

### 5.5 Explainable state

状态必须显示证据。面板不得仅从自然语言中推断“已完成”；绿色状态必须绑定进程、回执、产物或 Gate 记录。

### 5.6 Least context and least authority

Agent 只得到完成 Assignment 所需的事实、工具和权限。独立审计默认在 Reveal Gate 前互相不可见。

### 5.7 Adapter capability, not adapter optimism

每个 Harness 的功能通过运行时探测和契约测试确认。核心层不得假设所有 Harness 都支持 resume、实时 steering、精确 Token、结构化输出或相同权限模型。

## 6. 北极星体验

用户创建 Project，选择仓库和 Party 模板，配置一个 Implementer 与多个独立 Auditor，点击“运行到下一人工 Gate”。系统自动准备隔离工作区、编译 Harness 项目文档、启动 Agent、收集标准结果、并行发起盲审、比较结论，并只在发生冲突、越权、失败或授权请求时打断用户。

首发可验证路线固定包含一次跨 Harness 接力：Implementer 与 Auditor 使用两个不同的精确 ExecutionTarget。用户无需知道 native Session 或搬运上下文，但可以随时看到实际 Harness、模型、Profile、Host、能力新鲜度和是否发生过显式 fallback。

Command Center 必须把“技术条件通过”和“当前可派发”分开显示。`TECHNICALLY_READY` 只来自 Gate 的精确谓词评估；真正的 dispatch authorization 是 Controller 根据 M0、外部治理 publication、依赖、scope、lease/fence 与当前 hold 派生的单一结果。UI、Adapter、Runner 和计划文件中的布尔自报都不能单独开启执行。

## 7. 成功指标

MVP 阶段优先衡量：

- 人工跨窗口复制粘贴次数下降
- 自动接力成功率
- 重复事件不造成重复执行的比例
- 错误项目/错误 HEAD/过期 Snapshot 在执行前被阻止的比例
- Agent 完成到面板出现可验证回执的延迟
- Session 轮换后上下文握手通过率
- 人类只处理真实 Attention Item 的比例
- Controller 重启、重复回执和短暂断线后不产生双重执行的比例
- 同一 Workflow 跨 Harness 自动接力且无需人工复制的成功率
- 两个 Project 同时有活跃 Attempt 时跨 Project 数据、Profile、预算和工作区泄漏为零
- UI 关闭/重开与 Windows 重启后仍能诚实恢复任务和 Attention 的比例
