# 上下文、提示词与 Session 治理

- 状态：v0.4-p31-d1-product-contract-draft
- 目标：自动为每个 Harness/Agent 建立可审计项目上下文，并在正确时机换窗而不丢失职责和事实
- P31：候选全集、覆盖层级、检索与轮换语义的 D1 草案

## 1. Prompt 编译模型

GOGO 不维护一段不断追加的超级 Prompt，而是编译有来源的分层 Manifest：

1. Platform Invariants：不可由项目放宽的安全、协议和回执要求
2. Project Policy：项目隔离、授权、仓库、成本和自动化边界
3. Role Template：通用职责、输入/输出契约和停止条件
4. Project Role Override：该项目对通用 Role 的受控定制
5. Agent Override：该 Agent 的个体特化
6. Assignment Instruction：本次任务目标、范围、验收和禁止项
7. Context References：经过筛选的 Fact、Decision、Evidence 和 repository revision
8. Harness Glue：适配目标 Harness 的格式、启动参数和工具说明

后层可以具体化前层，但不得放宽其权限和不变量。同层冲突 fail-closed 并进入 Attention，不得 silently last-write-wins。任何 override 必须有版本、actor、理由和 Hash；Harness 自动追加的隐藏/框架内容若无法观察，CapabilityEvidence 必须标为限制，不能声称精确 Prompt 等价。

EffectivePromptManifest 记录每一层的来源、版本、顺序、内容 Hash、最终编译 Hash和 Harness Projector 版本。

## 2. Agent 创建时自动准备

面板创建 Agent 时执行：

1. 验证 Project、Role Version、ExecutionTarget、Harness Profile、ModelProfile、ProviderReadiness 和 CapabilityEvidence。
2. 创建 AgentInstance 与最小权限边界。
3. 选择精确 source revision，编译初始 ContextSnapshot。
4. 生成 EffectivePromptManifest。
5. 在 Agent 隔离 workspace 中通过 Projector 生成 Harness 原生项目文档。
6. Adapter 验证实际加载路径、文件 Hash 和 precedence 能力。
7. 创建 Session；未完成上下文握手前状态不得为 READY。

生成项目文档是缓存投影。用户或 Agent 想永久修改职责时，应修改 Role/Override Proposal 并重新编译，而不是直接编辑生成文件。

## 3. ContextSnapshot 内容

每个 Snapshot 是不可变、带 Hash 的最小集合：

- Project identity 与 RepositoryBinding
- Agent/Role/Party/Policy 精确版本
- Assignment 目标、范围、验收、停止条件与输出 Schema
- 精确 base/observed source revision 和 dirty witness（若有）
- 当前有效 Canonical Facts 与 Decisions
- 必需 Evidence/Artifact 引用
- 前序已接受 Result 的结构化摘要
- 权限、预算、deadline 和自动化上限
- requested/selected ExecutionTarget、模型容量/计价证据和 fallback 边界
- 已知风险、未决问题和禁止假设

Snapshot 不默认包含完整 transcript、其他独立 Auditor 的未揭示结果、无关任务历史或跨项目资料。

### 3.1 ContextCandidateSet 与检索

Compiler 入口必须接收冻结的：

```text
ContextCandidateSet
  ├─ query snapshot identity
  ├─ repository_binding_id / source revision
  ├─ visibility manifest + Reveal Gate generation
  ├─ index watermark / lag / unavailable
  ├─ inventory: Fact | Decision | Result | Evidence | ContextEntry | Instruction | Handoff
  └─ omitted / conflict / provenance
SelectionBasis
  ├─ overlay hierarchy + mandatory items
  ├─ budget + omission policy
  └─ snapshot revision / compiler digest
```

候选只来自 project-scoped 结构化 inventory。检索或索引结果只是可重建 projection，不能自行成为 Context Fact。P12 不得搜索变化中的 universe。P19 是检索/read-model owner，必须显示索引滞后、不可用和 scope。P21 不得把迟滞结果伪装成全集。

MVP 检索面：Project 内结构化筛选（类型、作用域、revision、可见性）。全文检索 UI 为 post-MVP。Reveal Gate 前，命中数量、排序、摘要和存在性都不得泄漏其他 Auditor 内容。

`ContextEntry` 纳入后续 Snapshot 必须经过 `Include in next snapshot` CollaborationProposal，而不能因为出现在 Feed 或被 pin 就自动进入模型上下文。GC owner 是 P36；活跃 Task/Gate/Fact/incident/restore 对引用对象形成 HOLD。

## 4. Context Health

Session Governor 持续计算以下维度，不以单个 Token 百分比决定换窗：

- `revision_drift`：当前 source/fact/policy/role 是否已偏离 Snapshot
- `task_boundary`：是否进入新任务、新阶段或职责切换
- `progress_stall`：相同失败、无新证据、反复尝试或输出 churn
- `interaction_state`：是否等待人类许可/输入，是否适合迁移
- `session_age`：持续时长、工具调用数量、压缩次数
- `context_pressure`：Harness 原生 token/compaction 证据或保守估计
- `target_drift`：模型、Harness/Profile、Host、readiness 或 capacity snapshot 是否已改变
- `runtime_health`：crash、rate limit、resume 失败、进程丢失
- `instruction_integrity`：生成文档漂移、Prompt digest/加载路径不一致
- `handoff_readiness`：是否已有足够事实、产物和未决项形成安全交接

归一化状态：

```text
HEALTHY | WATCH | ROTATE_RECOMMENDED | ROTATE_REQUIRED | BLOCKED | UNKNOWN
```

硬 revision/权限/Prompt mismatch 直接 `BLOCKED`；观测缺失为 `UNKNOWN`，不得用估算伪装健康。

ExecutionTarget 变化不是普通 steer。它必须重新进行 target selection、Context budget、projection/LoadProof 和 continuity handshake；不同模型的 token/context window 估计不得直接沿用。

## 5. 换窗策略

### 5.1 默认安全点

优先在以下位置轮换：Task 完成、ResultCapsule 已耐久提交、工具无进行中副作用、工作区 revision 已固定、后继任务尚未 claim。

不得仅因为 Token 阈值在写文件、执行发布、等待权限确认或 terminal receipt 尚未提交时强制换窗。紧急崩溃按 recovery 路径处理，不假装正常 handoff。

### 5.2 连续性模式

- `WARM_REATTACH`：同一 native Session 重新连接；只在 Adapter 精确支持时使用。
- `NATIVE_FORK`：Harness 创建可验证的 fork/checkpoint；新 Session 仍绑定新 Snapshot。
- `COLD_REBUILD`：新建原生 Session，从 Canonical Facts + Handoff Capsule 重建；所有 Harness 必须至少支持或可退化到此模式。

UI 必须显示实际使用的模式，不能把冷重建称为 resume。

### 5.3 自动化级别

- MVP 默认提供推荐和“一键安全轮换”。
- 只有在 Task 边界、receipt 已提交、无副作用进行中且 Adapter 通过轮换 Conformance Test 时，才可策略化自动轮换。
- 任何 hard mismatch 可以阻止继续执行，但不自动删除旧 Session。

## 6. Handoff Capsule

轮换前由系统生成结构化交接包：

- 已完成工作和 Accepted Result/Fact 引用
- 当前 source revision、工作区 dirty witness 和 artifact Hash
- 未完成步骤、阻塞和未决问题
- 已执行验证及其证据
- 当前权限/预算/期限
- 下一安全动作与禁止重复动作
- 旧 Session/Attempt/Receipt 引用

它必须由 Repository/Coordination 的结构化事实编译；可以引用从 transcript 提取并脱敏的片段，但不能依赖人工复制整段聊天。

## 7. 连续性握手

新 Session 在接收新 Assignment 或继承未完成 Assignment 前必须回传结构化握手：

- project/agent/role/session identity
- context snapshot 与 effective prompt digest
- source revision 与 workspace identity
- 当前任务、已接受事实、未决项和禁止项
- 预期输出与权限边界

Controller 对照 Snapshot 验证。结果为 `PASS | FAIL | INCONCLUSIVE`；非 PASS 不得自动继续。旧 Session 先归档而非立即删除，以便审计和恢复。

## 8. 事实漂移与运行中更新

新 Fact 或 source revision 出现时，系统不把整套上下文悄悄注入运行中 Session。根据影响范围：

- 无关：记录即可。
- 可安全补充且 Adapter 支持 steer：发送带 revision 的增量 Context Update。
- 改变目标/权限/验收：暂停 Assignment，创建新 Snapshot 和 Decision。
- 与当前工作冲突：进入 `BLOCKED`/Attention，并在安全点轮换或重规划。

每次增量更新都是持久 Command，必须得到 Adapter/Session 接受证据。

## 9. 归档与保留

GOGO 保存 Session 元数据、Snapshot/Prompt digest、Handoff Capsule、关键事件、Result/Evidence 引用和 native session locator。完整 transcript 的保留期由 Harness 与 Project Policy 管理；默认不复制到 Control Repository，也不得自动升为 Artifact。归档不是删除。ContextEntry/Proposal/Snapshot 附件的 GC owner 是 P36；删除需要独立保留策略、HOLD 检查和人类授权。

## 10. 首发 Harness 的原生 Context 信号

Context Governor 对首发 Adapter 使用以下已验证信号：

- Codex：`thread/tokenUsage/updated` 的累计/本次 usage 和 model context window、`thread/compacted`、thread/turn/item lifecycle、warning/error。
- Grok Build：初始化模型的 `totalContextTokens`、`session/update` 的 `totalTokens`/event ID、compaction/update 信号和 prompt stop reason。

原生 token 信号只更新 `context_pressure`，compaction 只更新 `session_age` 与连续性风险；它们都不直接触发换窗。换窗仍必须结合 task boundary、revision drift、interaction state、instruction integrity、runtime health 和 handoff readiness，并遵守安全点与 terminal receipt fence。

模型/Provider capacity 信号还必须绑定当前 ExecutionTarget 和证据时间。ProviderReadiness 过期或 observed model 与 selected target 不一致时进入 `UNKNOWN/BLOCKED`，不得把另一个 Profile 或模型的窗口、额度或价格用于当前 Session。
