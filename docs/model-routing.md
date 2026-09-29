# Codex 项目路由与 Controller 标准

本文只定义本项目的 Codex Controller/subagent 默认路由和行为边界，也约束 Controller 发起的 Grok 独立通道。Controller 负责目标、scope、委派、升级和最终技术判断，Owner 保留最终权威；现有治理、任务卡和更强规则继续有效。本标准不改变 Room 产品路由、M7、held work-package 路由，也不重新引入已废除的门控机制。

## 默认路由（后续 PR / 新派发）

| 角色/任务 | 默认路由 | 边界 |
| --- | --- | --- |
| 新任务 Controller | `gpt-6-sol` / `medium` | 目标路由；必须从当前会话身份确认，项目文档与配置不证明已运行 Controller 已切换 |
| 普通探索 / 证据收集 | `gpt-6-luna` / `high` | 只读摸底、直接数据源和证据整理 |
| 简单、明确、机械可验的施工 | `gpt-6-luna` / `high` | 使用显式派发；只执行批准范围 |
| 边界明确、工作量大或测试链长的施工 | `construction`：`gpt-6-luna` / `high` | 固定施工角色；max 仅在代表性任务校准后显式使用 |
| 复杂但已定界的核心实现 | `gpt-6-sol` / `high` | 使用显式派发，不新增固定角色文件 |
| 规划 / 根因分析 | `gpt-6-sol` / `high` | 方案和技术判断材料返回 Controller |
| 普通独立审计（fresh） | `gpt-6-sol` / `high` | 只读且独立；不把施工结果当作 acceptance |
| 高风险首次全轴风险审计（fresh） | `risk_auditor`：`gpt-6-astra` / `xhigh` | 一次覆盖全部互相独立的风险轴；与 Specialist 分开 |
| 已知缺陷修复后的聚焦 mutation 复核（fresh） | `gpt-6-sol` / `high` | 只复核已知缺陷、相关回归和受影响 invariant |
| 高风险 PR 最终全轴验收（fresh） | `risk_auditor`：`gpt-6-astra` / `xhigh` | 基于最新提交重新完成全轴审计，不继承旧候选结论 |
| 独立架构 Specialist | `gpt-6-astra` / `high` | 架构、安全、并发、矛盾证据或根因；不是高风险 Auditor |
| Grok 独立通道 | `grok-worker-pool` skill：`grok-4.6` / `high` / `standard` | 当前 Provider 只适合不要求终端、子进程和 mutation 的独立只读任务 |
| 最终异构审查 | 外部 Claude | 不经 Codex 原生 role 自动调用；中间审查不替代 Claude / Owner 最终裁决 |

原本派给 Astra 的任务，若同时满足本机网页通道 `web-chatgpt-subagent` skill 的适用条件，可优先经该 skill 交给 GPT-6 Pro（与 Astra 同一模型）处理。skill 额度用尽、不可用或结果不合格时，按上表回到 Codex 的 Astra 路由。需要运行正式命令或 mutation 的高风险首次或最终验收审计，仍由 `risk_auditor` 执行。

Sol 和 Astra 属于同一 provider family；独立 agent 不自动意味着异构审计。Grok 是独立 skill route，不是 Codex subagent 默认；不得为了使用 Grok 而派发。只有当前任务明确授权的 Grok Worker 任务才能调用；授权必须写清对象、次数、profile/fallback、工作区、允许修改和停止条件。当前 Provider 禁止终端和子进程，无法独立运行正式门禁或有效 production mutation，因此不能担任本项目 PR acceptance auditor；静态检查、预期判断或 Result Capsule 的 `completed` 都不等于正式审计 PASS。重新启用 acceptance 路由需要新的 Provider 能力证据和 Owner 明确授权。

## 选择与升级

Controller 根据任务形状选择：探索、证据和明确重复施工用 Luna/high；construction 固定角色先用 Luna/high，只有代表性任务的实际质量、延迟和返工证据证明需要时才显式校准 max；复杂共享 authority/protocol 或其他已定界核心实现使用 Sol/high 或由 Controller 直办；规划、根因分析、普通 fresh 审计和已知缺陷聚焦复核使用 Sol/high；状态权威、安全、并发、送达语义、验收防篡改和审计争议先使用 `risk_auditor` 的 Astra/xhigh 做全轴风险审计，全部聚焦复核通过后再用 fresh Astra/xhigh 对最新提交做最终全轴验收。Grok 只按上一段的 skill route 处理符合资格且已授权的独立只读任务包。

直办成本较低的简单任务不必委派；预派前必须明确目标、允许文件、成功标准和完成后的停止/返回条件。当不确定性、架构/API/schema、安全、迁移、并发、生产、跨模块 invariant 或设计不一致需要改变既定目标、scope、contract、安全边界或验收规则、超出当前授权，或同一失败两次时返回 Controller；最难的问题直接路由，不强制经过固定 ladder。仅在有明确收益时委派；并行只用于真正独立且写入 ownership 不重叠的任务包。任何 worker 都不能改变 goal/scope/contract、削弱安全或测试、或自我验收。

委派必须产生真实并行收益；派工说明、监控、review和集成的总成本接近或超过Controller直办成本时应直办，分钟数和文件数只作判断基准。短小且修复点与验证命令已明确的任务、1至2个紧密文件的小修、同一失败的连续fixture/字段/codec调试、critical path共享热点、schema/protocol/authority最终拼接，以及冲突处理、candidate cut、commit/push、gate判断和production-reachability终验默认由Controller完成。

优先委派可一次性交代目标、文件、成功标准和停止条件的独立整包，ownership明确不重叠且能真实并行的较长施工，批量机械修改、长测试链、独立证据收集，以及fresh/risk audit和specialist分析。默认并发上限为一个Controller critical-path工作、1至2个真正独立施工包和一个fresh auditor；额外并发须证明收益。Grok仅在Provider健康、Task Capsule完整且独占worktree成立时派发，通道不稳定时不为坚持路由反复派发。

## Worker 活性与回收

Controller 以可验证活动而非 `running` 标志判断进展：活跃命令、具名 diff、具体定位结论或明确可观察的等待对象均可继续；没有这些证据时应及时核对并回收。Worker 只在派发边界内施工和报告，Controller 负责活性核对、保全在飞成果、回收失去进展证据的任务及重新路由；等待不得用空泛状态更新替代证据。

小任务出现两轮协调或边界往返时，Controller重新计算委派盈亏；接管前先保全WIP/custody。任何agent不得为局部任务运行全仓格式化、清理或外围重构。

长回合每3至4小时只评估是否换班，不机械中断活跃事务。需要换班时，当前 Controller 先完成 durable checkpoint、commit/push，并核对 HEAD、diff、未推送成果、文件所有权、进程与生成物 custody，再自动创建后继 Codex 任务并发送自包含接管材料；后继任务回执 exact repo/branch/HEAD、工作树、gate/non-claim、critical path 与边界后，旧任务才停止。创建失败、工作树冲突或未收到确认时，原 Controller 继续负责，禁止出现无人接管空档。

## 高风险审计流程

高风险首次审计与最终审计开始前，先列出该 PR 的全部独立检查轴和对应 R 编号，并验证审计仪器、fixture、路径长度、实际加载文件和 build 身份。随后原样运行三条正式命令，记录退出码、用例数、fail 与 skip；再运行提交内 focused 测试，并对每个计划轴执行真实行为复现或有效 production mutation。

发现缺陷后先记录，不立即结束整轮审计。只要继续检查不会污染证据、扩大权限、破坏工作区，且后续轴不依赖该缺陷先修复，就继续完成其余互相独立的轴，一次返回当前候选的完整缺陷批次。只有审计仪器无效、工作区或运行时身份无法确认、发生不可恢复污染、后续轴严格依赖当前缺陷，或触及禁止范围时，才可提前停止；报告必须列出未执行轴和原因，不能写成通过。

施工集中修复后，由 fresh Sol/high 聚焦复核已知缺陷、相关回归和受影响 invariant。聚焦复核全部通过后，再由 fresh Astra/xhigh 基于实际最新提交完成最终全轴审计。有效 mutation 必须保留提交内测试，只撤销真实生产修复；mutation 后代码仍通过语法、类型或构建检查，并因行为断言退出非零；恢复后 focused 测试和正式门禁重新为绿。删除 import、制造语法错误、破坏 fixture、修改测试预期或只撤销测试替身均无效。

## 生效语义与边界

`.codex/config.toml` 设置新任务项目默认 `gpt-6-sol` + `medium`，以及 `[agents]` 的 subagent 默认值 `gpt-6-luna` + `high`。`.codex/agents/` 下的 standalone TOML 由当前 Codex custom-agent discovery 识别，每个角色显式声明 `name`、`description`、`developer_instructions`、`model`、`model_reasoning_effort`；没有 `config_file` 表。Codex 先解析显式 spawn 值、`[agents]` 默认值和父级值，再由 custom agent 文件中显式设置的 `model`/`model_reasoning_effort` 覆盖已解析的通用字段；固定 role 文件会覆盖显式 generic spawn 的模型和强度，提示词不能改变这些参数。需要 xhigh 时选 `risk_auditor`，或使用不加载该固定 role 的通用 spawn 显式参数。本次不宣称或测试 live custom-role discovery。这些默认不是 hard lock，也不会自动切换当前 session。

本次 construction OS 续接的当前 Controller 身份仍以 Owner 对当前线程的既有确认为 `gpt-5.6-sol` / `medium`；本次路由更新不把它伪称为已切换。新任务建议使用 `gpt-6-sol` / `medium`，但必须由新会话的实际身份或解析结果确认，不能从项目配置或文档推断；项目默认也不自动改变已运行 agent。

本路由只对后续 PR 和新派发的 agent 生效；现有或运行中的 agent 保留原设置，已准备的后续 PR agent 也不会因转交准备笔记改变旧 session，必须新建施工派发才采用新规则。提交或临时审计变更仍以当前任务的明确授权为准。

内置协作树达到线程上限时，Controller 依次：复用已完成或空闲的现有内置 agent；结束不再需要的内置 agent 后复用槽位；等待正在运行的 agent 完成；最后把无法派发的工作留在 Controller 队列。禁止用 `codex exec`、`create_thread` 或其他独立 Codex 会话补容量，不得创建会出现在 Owner 任务列表或支线程中的额外会话。任何 agent 遇到审批要求只能返回 Controller，不得要求 Owner 打开支线程或点击批准。

把三层分开：runtime-enforced 是现有 runtime 对 sandbox、approval、工具和其他边界的实际强制行为，本次配置不新增任何硬权限；runtime default 是本文件与 TOML 的默认选择；instruction-level policy 是 Owner 归属、角色窄边界、证据要求、停止条件和回执要求，依靠 agent 遵循及 Controller/Auditor 检查。不要把配置加载或本地 readiness 宣称为账号 readiness，也不自动声明 provider、权限、sandbox、approval、MCP、concurrency 或其他安全设置。

## Grok、技能与任务回执

Grok 只走 `grok-worker-pool` skill 的既有预检/模型检查和独立通道；不得跳过 skill check，不得从预检失败自动 fallback，不得把 Grok 当作 Codex subagent 默认。D 试点虽形成 `worker_completed`，但 Provider 禁止终端和子进程，正式命令与 mutation 均未执行，所以不能作为 acceptance；当前 B→K 不再发起真实 Grok Worker 请求，不切 profile、不重新 probe。施工、规划、审计和 specialist 都必须保留项目原任务卡格式，并追加：`TASK RESULT CURRENT_STATE FILES_CHANGED IMPORTANT_DIFF VALIDATION FAILURES INVARIANTS_CHECKED RISKS DEVIATIONS_FROM_PLAN OPEN_QUESTIONS RECOMMENDED_NEXT_ACTION`。失败时追加 `ROOT_CAUSE_IF_KNOWN ATTEMPTS_MADE WHY_BLOCKED WHAT_REQUIRES_CONTROLLER_DECISION`；未执行的验证必须明确写未执行，不能把跳过写成通过。最终异构 Claude 审查仍由外部流程按 Owner/Controller gate 处理，不能自动派发。

官方参考：[Codex Configuration Reference](https://learn.chatgpt.com/docs/config-file/config-reference) · [Codex Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents)
