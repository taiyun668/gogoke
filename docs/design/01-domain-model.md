# 核心领域模型

- 状态：v0.5-p31-d3-p17-namespace-handoff
- 目标：为 UI、协调数据库、Repository Protocol、Workflow Engine 和 Harness Adapter 提供共同语义
- P31：下列对象语义是 D1 产品契约草案加 D3 `P31-D3-P17-01` P17 namespace handoff；wire 由 P32 冻结，Owner `ACCEPTED` 前不得作为实现授权

## 1. 对象层级

```text
Control Plane
  ├─ LocalControlHost / OperatorIdentity
  ├─ Host / Runner
  │   └─ CapabilityEvidence × N
  ├─ HarnessInstallation / ModelProfile / ProviderReadiness
  └─ Project
      ├─ RepositoryBinding (MVP: 1 source; control colocated; P17 immutable namespace binding)
      ├─ ProjectNamespace / worktree (P17 unique owner; project_id/assignment_id/attempt_id)
      ├─ ContextEntry / CollaborationProposal / ContextCandidateSet
      ├─ Party
      │   ├─ RoleTemplate
      │   └─ AgentInstance × N
      │       └─ Session × N
      └─ WorkflowRun
          └─ Task × N
              └─ Assignment × N
                  ├─ Attempt × N
                  ├─ CommandRecord × N
                  ├─ ResultCapsule × N
                  └─ Evidence × N
```

GateEvaluation、Decision、Fact、ContextSnapshot、RuntimeObservation 和 PublicationCapsule 通过稳定 ID 与这些对象关联，而不是依赖目录名或聊天窗口标题。

## 2. 组织与执行实体

### 2.1 Project

最强的默认隔离边界。Project 有稳定 `project_id`，包含自己的仓库绑定、Party、Policy、事实、任务、协调状态、凭据引用、预算和审计记录。不得从 cwd、Git remote 或窗口标题猜测 Project 身份。

产品生命周期：

```text
PROVISIONING -> ACTIVE -> READ_ONLY | ARCHIVED
任意非墓碑状态 -> DELETION_PROPOSED -> DELETION_PENDING -> TOMBSTONED
ARCHIVED | READ_ONLY --authorized restore--> ACTIVE
```

- `Open` 不是状态，而是 operator 对 `ACTIVE`、`READ_ONLY` 或 `ARCHIVED` 的附着。
- `ARCHIVED` 与 `READ_ONLY` 停止新 claim；已在途 Attempt 仍按 Stop/Quit 合同收敛。
- 删除是独立破坏性提案，必须带 exact scope、引用预览、HOLD 检查、幂等键和显式确认；卸载默认不进入该路径。
- 切换 Project 必须清空或换域：feed cursor、搜索、动作草稿、选中对象、Inspector、事件订阅、Session/Artifact 引用、凭据 scope 和缓存键。跨 Project 引用 fail-closed。

Source plane：身份与生命周期在 Coordination Plane；接受后的配置可投影 Canonical Fact Plane。Authority owner：Controller。Retention：tombstone 保留身份。非权威 UI：Projects 列表与 scope bar。

### 2.2 RepositoryBinding

- `control`：保存 GOGO 协议对象和已接受事实的可审阅账本
- `source`：Agent 修改或审计的源码仓库
- `docs`：可选文档仓库
- `artifact`：可选大型产物存储引用

MVP 产品基数：一个 Project 恰好一个 `Source Repository`。Control Repository 默认同仓 `.gogo/`（M0 `COLOCATED_DOT_GOGO_DEFAULT`）。同一 Git 仓库在 MVP 中同时承担 `control` 与 `source` 两个角色。

`Project -> SourceRepository/repository_binding_id` 对每个 Attempt 是不可变绑定。P17 是该 binding 的唯一 namespace owner；source write 不得改写、猜测或临时替换 binding。协议仍保留 sidecar Control Repository 与多 Source 能力，但那不是 MVP 交付。若后继启用多 Source，每个 source revision 必须绑定 `repository_binding_id`，不得让 revision 在多仓库下歧义。

### 2.3 Party 与 RoleTemplate

Party 是 Project 当前采用的、可版本化的成员组织。历史 Assignment 绑定创建时的 `party_version`，后续变更不得追溯篡改历史。

RoleTemplate 至少声明目标、职责、Prompt、输入可见性、输出 Schema、工具/文件/网络权限、停止条件、Harness 能力要求、预算、超时和审计独立性要求。Project 或 Agent Override 必须形成新版本并可追溯到基础模板。

### 2.4 AgentInstance

Project 内持久存在的成员身份。AgentInstance 绑定 Role Version、目标偏好策略和权限边界，但不是进程或聊天窗口。它可以偏好某个 HarnessProfile；具体 Harness、模型、Profile、CredentialHandle、Host 和能力证据由 Assignment/Attempt 的 ExecutionTarget 固定。Session 轮换后 Agent 身份继续存在。

### 2.5 Session

某个 AgentInstance 在某个 Harness 中的一次上下文生命周期。Session 可以处于 `CREATING | READY | ACTIVE | WAITING | ROTATING | ARCHIVED | FAILED | UNKNOWN`，并保存 Harness 原生 Session ID、ContextSnapshot、有效提示词摘要和运行引用。

Session 不是正式项目事实源。一个 Agent 可以有历史 Session；默认只能有一个可写 Active Session，除非 Workflow 显式允许并行 Assignment。

### 2.6 Host、Runner 与 CapabilityEvidence

Host 是执行机器；Runner 是 Host 上受控执行服务。Runner 负责能力探测、凭据本地解析、进程监督、观察和回执。工作区准备只消费 P17 给出的 ProjectNamespace/worktree，不得自行发明 binding、root 或路径。

CapabilityEvidence 是带来源、版本和有效期的能力证明，而不是静态布尔值，至少包含：

- `harness_id`、`harness_version`、`adapter_version`
- `host_id`、OS 与 execution profile
- 支持级别与集成模式
- 探测命令/协议握手的非敏感证据
- `verified_at`、`expires_at`、Conformance Suite 版本与结果

### 2.7 HarnessInstallation、ModelProfile、ProviderReadiness 与 ExecutionTarget

HarnessInstallation 是 Host 上某个公众/批准发行实例的稳定引用，包含来源、版本、binary/package digest 和探测证据。ModelProfile 是模型能力、上下文容量、工具/结构化输出和计价证据的版本化描述。ProviderReadiness 是脱敏、带 scope 和有效期的运行就绪证据，不保存凭据或账号详情。

ExecutionTarget 把一次实际执行所需的 HarnessInstallation、Adapter、HarnessProfile、ModelProfile、CredentialHandle、Runner/Host、CapabilityEvidence、ProviderReadiness、pricing snapshot 和 fallback policy 绑定为一个不可变 digest。它的完整合同见 `19-execution-target-model-routing.md`。

Assignment 保存 `target_requirements + target_preferences + allowed_target_set_digest + fallback_policy`。Attempt 保存 `selected_execution_target_id + target_digest`。TerminalReceipt 同时引用 requested/selected/observed target；三者不一致或 observed 无法证明时必须显式报告，不能由 Adapter 自行纠正。

## 3. 工作实体

### 3.1 WorkflowDefinition 与 WorkflowRun

WorkflowDefinition 是可版本化任务图模板；WorkflowRun 是 Project 中的一次实例化，绑定当时的 Party、Policy、Workflow 与 source revision。

### 3.2 Task

Task 描述“必须得到什么”，不绑定特定 Agent 或 Session。依赖满足后才可 READY。

```text
DRAFT -> READY -> ACTIVE -> VALIDATING -> ACCEPTED -> COMPLETED
  └──────────────> BLOCKED / CANCELLED
                         VALIDATING -> REJECTED -> READY | CANCELLED
```

- `ACCEPTED`：任务产出通过任务 Gate。
- `COMPLETED`：其完成条件和必需后继/收尾 Gate 均满足。
- Task 完成不等于代码已 merge、deploy 或 release；这些是单独授权动作。

### 3.3 Assignment

Task 与 AgentInstance 的一次工作绑定，包含不可变输入 Snapshot、权限、预期输出、预算、超时、调度约束和目标需求/偏好。Assignment 不以 `harness_id` 代替完整 ExecutionTarget 选择。

```text
PENDING -> CLAIMED -> DISPATCHING -> RUNNING | WAITING
       -> RECEIPT_SUBMITTED -> VALIDATING -> ACCEPTED | REJECTED
       -> FAILED | EXPIRED | CANCELLED
```

活跃进程是否存在属于 RuntimeObservation；观察不确定时投影为 `UNKNOWN`，不得伪造 Assignment 终态。

### 3.4 Claim、Lease 与 Attempt

Claim 是 Scheduler 对可运行 Assignment 的原子认领。Lease 是有期限的占用权，包含 `lease_id`、`holder_id`、`expires_at` 和单调递增 `fencing_token`。

Attempt 是 Assignment 的一次实际执行尝试，并绑定一个精确 ExecutionTarget。重试必须创建新 `attempt_id`，不得覆写旧证据。切换模型、Profile、CredentialHandle、Host、Harness 或协议等级也必须创建新 Attempt 或显式 target generation。Lease 过期只说明占用权失效，不证明旧进程已经终止；重新派发前必须通过 Runner reconciliation、强制终止证据或人工决定确认不会双重写入。

### 3.5 CommandRecord

Controller 希望 Runner 执行的持久命令。所有命令必须包含 project、command、epoch、幂等和过期字段；对象 scope 按命令类型要求。Assignment/Attempt 命令必须包含 `assignment_id + attempt_id + fencing_token`，Agent 初始化类命令则包含 `agent_id + provisioning_operation_id`，不得用空字符串伪造 Assignment。Assignment 启动命令示例：

```yaml
command_id: stable-id
project_id: project-id
assignment_id: assignment-id
attempt_id: attempt-id
command_type: session.create|assignment.start|session.steer|attempt.cancel|session.archive
idempotency_key: stable-key
expected_revision: 7
controller_epoch: 12
fencing_token: 3
expires_at: RFC3339
context_snapshot_digest: sha256:...
capability_requirement_digest: sha256:...
policy_decision_id: decision-id
```

Runner 必须以 `command_id + fencing_token` 去重，并拒绝旧 epoch、旧 fencing token、过期命令和 Project 不匹配命令。

### 3.6 ContextSnapshot 与 EffectivePromptManifest

ContextSnapshot 是不可变的最小权威上下文集合，引用 Fact、Task、Repository Revision、Decision、Policy 与 Evidence，但不嵌入完整 transcript。

Snapshot 还绑定用于预算和 projection 的 ExecutionTarget/ModelProfile capacity snapshot。目标变化时必须重新选择 Context、重新投影并取得新 LoadProof，不能沿用另一个模型的 token 预算或加载证明。

EffectivePromptManifest 记录各层 Prompt 的版本、顺序、来源、内容 Hash 和最终编译 Hash，使“Agent 实际收到什么职责”可审计。Harness 原生生成文件只是 Manifest 的投影。

### 3.7 ResultCapsule

Agent 对 Assignment 的结构化提案，至少包含：

- `verdict`、`summary`、`repository_revisions`
- `artifacts`、`evidence_refs`、`required_changes`、`unresolved`
- `boundary_statement`、`next_action_proposal`
- `context_snapshot_id`、`effective_prompt_digest`
- `requested_target_digest`、`selected_execution_target_id`、`observed_native_target_evidence`
- `producer`、`session_id`、`attempt_id`

ResultCapsule 在 Gate 接受并完成仓库物化前均为不可信 Proposal。

### 3.8 Fact、Evidence、GateEvaluation 与 Decision

Fact 生命周期：

```text
PROPOSED -> PROMOTION_PENDING -> ACCEPTED -> SUPERSEDED | REVOKED | EXPIRED
```

Fact 必须记录来源、适用 revision、接受 Decision、仓库对象 Hash 和过期条件。

Evidence 是 commit、diff、测试报告、命令退出码、文件 Hash、截图或外部 URL 等可检查材料。敏感值不得明文保存。

GateEvaluation 统一为 `PASS | FAIL | INCONCLUSIVE | APPROVAL_REQUIRED`；`INCONCLUSIVE` 不得降级为 PASS。Decision 是 Controller、Policy 或人类对 Proposal、Gate、冲突、授权和下一步做出的权威记录；人工 Override 也必须记录理由。

### 3.9 Event、RuntimeObservation 与 PublicationCapsule

- Command：希望发生的动作。
- Accepted Event：Controller 已去重、验证并接受的状态变化。
- RuntimeObservation：Runner 或 Harness 报告的现场证据，尚不自动等于 Accepted Event。
- PublicationCapsule：Project 主动跨隔离边界发布的最小信息包。

进程启动命令已发送，不等于 `assignment.started`；只有带真实 process/session 证据的观察被 Controller 接受后才能产生该 Event。

### 3.10 Usage、Cost 与 BudgetReservation

UsageObservation 记录 Adapter/Runner 观察到的 token、时长、工具调用或计算资源，并携带 `EXACT | PROVIDER_REPORTED | ESTIMATED | UNKNOWN` 证据等级。CostEntry 记录币种、计价来源、pricing snapshot/version 和是否只是估算；不得把本地估算显示成供应商账单事实。

BudgetReservation 属于 Coordination Plane。Scheduler 在 claim 前按 Project/Workflow/Agent Policy 预留预算，Attempt 结束后结算或释放。超预算默认阻止新 claim，不强杀正在产生不可逆副作用的 Attempt。

### 3.11 AttentionItem

AttentionItem 是需要人类判断或确认的持久工作项，包含 severity、reason code、受影响对象、证据、已尝试的安全动作和允许的下一步。状态为 `OPEN | ACKNOWLEDGED | RESOLVED | SUPERSEDED`；解决必须引用 Decision，不能因 UI 被关闭而消失。

### 3.12 ResourcePool、CapacityRequest 与 CapacityPermit

ResourcePool 描述 Host、Runner、Provider、HarnessProfile、Model 或 Project budget 等共享稀缺资源的 scope、capacity generation、证据新鲜度和公平策略版本。CapacityRequest 将 Project/Assignment/Attempt 候选、资源单位、优先级类、target/universe/policy digest 绑定成一个原子请求。

CapacityPermit 是 Coordination Plane 中的 fenced lease：它绑定 exact pool/request/project/attempt、granted units、generation、expiry 和 fencing token。没有当前 permit 的 Attempt 不得启动；permit 释放必须经 terminal/reconciliation 证据，不得仅因 UI 关闭、PTY idle 或 owner 进程暂时不可见就猜测释放。

### 3.13 ContextEntry、CollaborationProposal 与 Instruction

`ContextEntry` 是 Project Context Ledger 中的协作对象，卡片只是它的人类可读投影。

- Source plane：提案在 Coordination Plane；接受后的规范摘要可投影 Canonical Fact Plane。
- Authority owner：Controller/Transition Engine。UI/Adapter 只能提交 Intent。
- Scope：`project_id`，可选 `workflow_run_id` / `task_id` / `agent_id` / visibility policy。
- Lifecycle：`DRAFT -> SUBMITTED -> ACCEPTED | REJECTED | SUPERSEDED | EXPIRED | ARCHIVED`。
- Retention：Project retention policy；被活跃 Task、Gate、Fact、Snapshot 或 Handoff 引用时 HOLD。GC owner 是 P36，执行通用 Artifact Store 的引用/tombstone 合同。
- 非权威 UI：Context Feed 卡片、筛选器和置顶。

`CollaborationProposal` 是从卡片动作或 composer 提交的结构化 Intent（Ask/Create task/Assign/Request audit/Include in snapshot/Create Attention）。它不得自行创建 Assignment、Fact、Decision 或 Gate PASS。

- Source plane：Coordination Plane。
- Authority owner：同一 Transition Engine。
- Scope：与目标对象相同，且必须包含 actor、expected revision、idempotency key 和副作用预览。
- Lifecycle：`PROPOSED -> VALIDATING -> ACCEPTED_INTENT | REJECTED | EXPIRED`。
- Retention：未接受前跟随对应 ContextEntry；接受后跟随所创建对象。
- 非权威 UI：allowed actions 与 command composer。

`Instruction` 是已接受的 standing directive（Canonical Fact Plane），不是聊天句，也不是 Attempt 层的 Assignment Instruction overlay。它进入后续 Snapshot 必须经过选择与可见性证明。

`ReactionSignal` 不是领域对象，不进入 MVP 权威面，没有 lifecycle 或 retention。UI 只可显示 `DISPLAY_ONLY` 气氛；P32 不得为其赋予 schema authority。结构化投票必须使用独立 Vote/Decision 对象，属 post-MVP。

### 3.14 ContextCandidateSet 与 SelectionBasis

`ContextCandidateSet` 是 Compiler 入口的冻结候选全集。它只包含 project-scoped 结构化 `Fact | Decision | Result | Evidence | ContextEntry | Instruction | Handoff`，并绑定 repository binding、source revision、visibility manifest、retention 过滤与 index watermark。检索或索引命中是可重建 projection，不能自行成为 Context Fact。

`SelectionBasis` 记录覆盖层级、mandatory 项、预算、omission、冲突和 provenance。覆盖顺序为 Platform Invariants → Project Policy → Role Template → Project Role Override → Agent Override → Assignment Instruction。后层不得放宽前层权限；同层冲突 fail-closed 并进入 Attention。

MVP 提供 Project 内结构化筛选。全文检索 UI 为 post-MVP。无论是否有搜索 UI，P19 必须诚实报告 index lag/unavailable/scope；Reveal Gate 前不得通过命中数量、排序、摘要或存在性泄漏其他 Auditor 内容。P12 不得在编译过程中查询变化中的 universe。

### 3.15 OperatorIdentity 与 LocalControlHost

`OperatorIdentity` 是首次启动签发的 GOGO 持久 `operator_id`。当前 OS principal 是必要的本机归属证明，不等于已授权用户。Control API 会话必须绑定 origin、CSRF、IPC identity 与 Host owner token。

`LocalControlHost` 是当前 OS principal 的 per-user 后台单实例进程，不是 Windows Service。它拥有 logical Controller、Coordination Store、reconciliation、Runner 连接和 notification broker。Desktop UI 只是客户端。完整生命周期见 `20-local-control-host-lifecycle.md`。

### 3.16 ProjectNamespace 与 worktree ownership/lease

P17 是唯一 Project/worktree namespace owner。P16、P33、P18 与 P22 只消费该权威，不得另建平行 namespace、root 或 source-write 路径。

- Source plane：Coordination Plane。接受后的 binding 身份可投影 Canonical Fact Plane。
- Authority owner：P17。Runner 只持有当前 worktree ownership/lease；UI/Adapter 不得选择或拼接路径。
- Immutable binding：`Project -> SourceRepository/repository_binding_id`。Attempt 开始后不得更换 Source Repository 或 `repository_binding_id`。
- Canonical namespace：已注册 runner root 下由 `project_id/assignment_id/attempt_id` 组成。D1 推荐路径中的 `agent-id` 段不是权威组成；Agent 可写隔离由 assignment/attempt 唯一性保证，同一 Project 的 Agent 仍不得共享可写工作目录。
- Worktree ownership/lease：绑定当前 Controller epoch、Assignment/Attempt fence，以及 Runner/process identity。lease 过期或进程身份不匹配不是安全写入许可。
- 每次 source write 前必须完成全部 binding/fence 校验。cross-Project、cross-Attempt、stale Attempt/epoch/fence/lease 或 path-escape 一律 zero-write reject。
- Dirty workspace：进入 Attention 与 human Gate。不得 auto-clean、auto-reuse 或 fallback。
- Scope：`project_id + assignment_id + attempt_id`，外加当前 `repository_binding_id`。
- Lifecycle：跟随 Attempt 与其 worktree ownership/lease；dirty 时进入 Attention/human Gate，只能经显式 Decision 继续、隔离或释放。lease 过期进入 reconciliation，不是写入许可。
- Retention：跟随 Attempt/Evidence。GC 不得抹掉仍被活跃 Attempt/Gate 引用的 worktree identity。
- 非权威 UI：Attempt/Inspector 上的脱敏路径与 dirty/lease 状态。显示名、任务标题或 Agent 昵称不得成为路径段。

## 4. 四平面归属

| 对象 | 权威平面 | 主要写入者 |
|---|---|---|
| Fact、Accepted Result、Decision、Repository Revision | Canonical Fact Plane | Controller 经 Gate 后物化至 Control Repository |
| Task 活跃状态、Assignment、Claim、Lease、Attempt、Command、消费位点 | Coordination Plane | 持有有效 leader fencing 的 Controller |
| Heartbeat、PID、PTY、stream cursor、usage estimate | Live Observation Plane | Runner；Controller 只接受/投影 |
| Transcript、native checkpoint、Harness session cache | Native Session Plane | Harness |
| BudgetReservation、ResourcePool/CapacityRequest/CapacityPermit、AttentionItem | Coordination Plane | Controller 经 Policy/Transition；Store 只执行 exact plan CAS |
| ExecutionTarget、TargetSelectionRecord、ProviderReadiness snapshot | Coordination Plane；接受的非敏感证据可投影至 Canonical Fact Plane | Controller；Runner/Adapter 只提交观察 |
| Provider-confirmed Cost/Evidence | Canonical Fact Plane | Controller 经来源验证后物化 |
| ContextEntry、CollaborationProposal、ContextCandidateSet/SelectionBasis | 提案与候选集在 Coordination Plane；接受后的 Instruction/Fact 投影 Canonical Fact Plane | Controller；P12 只消费冻结候选集；UI 只提交 Intent |
| OperatorIdentity、LocalControlHost identity/lease | Coordination Plane | Local Control Host；UI 不得成为 owner |
| ProjectNamespace、worktree ownership/lease、immutable `Project -> SourceRepository/repository_binding_id` | Coordination Plane | P17 唯一 owner；P16/P33/P18/P22 只消费，不得另建 authority |
| ReactionSignal | 无权威平面 | 无；仅 display-only UI |

没有一个存储可以回答所有问题。UI 必须显示状态来自哪个平面以及证据新鲜度。

## 5. 公共身份字段

所有持久对象至少包含：

```yaml
schema_version: gogo/v1alpha1
id: stable-id
project_id: project-id
created_at: RFC3339
created_by:
  actor_type: human|controller|agent|runner
  actor_id: stable-actor-id
revision: 1
```

运行对象按适用范围携带 `workflow_run_id`、`task_id`、`assignment_id`、`attempt_id`、`agent_id`、`session_id`、`runner_id`、`context_snapshot_id`、`correlation_id` 和 `causation_id`。

## 6. 关键不变量

1. 所有主键、唯一键、缓存键、事件订阅和文件命名空间都必须包含或可验证 `project_id`。
2. 一个 Session 只能属于一个 Project 和一个 AgentInstance。
3. Assignment 使用的 ContextSnapshot 和 EffectivePromptManifest 在 Attempt 内不可原地修改。
4. Agent 不得修改其他 Agent 的 ResultCapsule；独立审计结果在 Reveal Gate 前互不可见。
5. Result 接受、Task 完成、merge、deploy 和 release 是不同状态与授权。
6. Coordination Store 丢失不能仅靠 Git 安全重建 in-flight claim/lease；此时必须进入 reconciliation/UNKNOWN，而不是自动重派。
7. RuntimeObservation 缺失或矛盾时必须显示 `UNKNOWN`，不得依据窗口标题或自然语言猜测。
8. 跨项目信息默认不可见，只有经 Policy 接受的 PublicationCapsule 例外。
9. 同一 Command、Event 或 Result 的重复投递必须幂等；不同 payload 使用相同幂等键必须拒绝。
10. 只有持有当前 Controller epoch 与 Assignment fencing token 的写入才能推进协调状态。
11. Harness、模型、模型版本、HarnessProfile、CredentialHandle、Host、Runner 或协议等级的变化必须形成新的 TargetSelectionRecord；Adapter 和 UI 都不得静默替换。
12. Desktop UI 的存在、窗口关闭或托盘状态不得推进 Controller、Assignment、Attempt 或 Attention 状态。
13. 所有共享 Host/Runner/Provider/Profile/Model 容量必须在 Attempt 启动前获得与当前 Controller epoch 和 Attempt fence 绑定的 CapacityPermit；Adapter、Runner、UI 不能自行选择受益 Project。
14. MVP 一个 Project 恰好一个 Source Repository；跨 Project 的 ContextEntry、mention、reply、candidate、permit 与凭据引用必须 fail-closed。
15. Context Compiler 只能消费冻结的 ContextCandidateSet/SelectionBasis；检索命中、卡片正文和 reaction 都不能自行成为 Fact。
16. operator_id、OS principal 和 Host owner token 必须同时可验证；缺一则 Control API 拒绝危险动作。
17. P17 是唯一 Project/worktree namespace owner。每次 source write 前必须验证 immutable `Project -> SourceRepository/repository_binding_id`、已注册 runner root 下的 `project_id/assignment_id/attempt_id` namespace，以及绑定当前 Controller epoch/fence 与 Runner/process identity 的 worktree ownership/lease。cross-Project、cross-Attempt、stale 或 path-escape 一律 zero-write reject。dirty workspace 进入 Attention/human Gate，不得 auto-clean、auto-reuse 或 fallback。P16/P33/P18/P22 不得另建 namespace authority。
