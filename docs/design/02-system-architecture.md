# 系统架构

- 状态：v0.4-p31-d1-product-contract-draft
- 架构风格：本地优先控制平面 + 持久协调内核 + 仓库事实账本 + 可注册 Runner + Harness Adapter Kernel
- P31：产品边界草案；本文件不授予 Schema/实现权威

## 1. 总体架构

```text
Web/Desktop UI / CLI / future MCP
              │ versioned API + event stream
              ▼
┌──────────────────── GOGO Control Plane ────────────────────┐
│ Project & Party │ Workflow │ Gate/Policy │ Context/Session │
│ Scheduler       │ Attention│ Repository  │ Reconciliation │
│ Execution Target Registry  │ Local Host Lifecycle         │
│                 logical Controller authority              │
└───────────────┬───────────────────────┬─────────────────────┘
                │                       │
       SQL transaction + outbox         │ projector / reactor
                ▼                       ▼
       Coordination Store       Control Repository (Git)
       claims/leases/commands    accepted facts/results/
       attempts/offsets          decisions/protocol objects
                │                       │
                └──────────┬────────────┘
                           │ fenced Runner Protocol
                           ▼
                    Host Runner × N
          workspace │ secret refs │ process/PTY │ evidence
                           │
                           ▼
                Harness Adapter Kernel
                           │
      Codex / Grok CLI / Claude Code / Pi / OpenCode / Generic CLI / ...

Live Observation Store: heartbeat, process evidence, output cursor, usage
Native Session Store: each Harness's transcript, checkpoint and session cache
```

## 2. 四个状态平面

### 2.1 Canonical Fact Plane

保存已接受、可审阅、可进入后续 ContextSnapshot 的项目事实：Accepted Result、Fact、Decision、任务成果摘要、证据引用和精确 repository revision。其交换与恢复载体是 Control Repository。

Gate PASS 首先产生 `PROMOTION_PENDING`。Repository Projector 把确定内容以幂等方式写入本地 Control Repository 并取得 commit/object hash；Controller 记录物化回执后才产生 `ACCEPTED`。Remote push 是后续同步状态，不阻塞本地事实接受，除非 Project Policy 明确要求远端确认。

### 2.2 Coordination Plane

保存活跃工作所需的事务状态：Task 状态、依赖、Assignment、Claim、Lease、Attempt、Command、幂等记录、leader epoch、consumer offset、retry timer 和 transactional outbox。

默认实现可以是 SQLite，但它是持久协调数据库，不是可随意删除的缓存。它需要 WAL/崩溃恢复、迁移、备份和一致性检查。原子 claim、compare-and-swap 与 fencing 在这里发生。

### 2.3 Live Observation Plane

保存高频且可重建的现场观察：Runner heartbeat、PID/process identity、PTY/stream cursor、session activity、usage estimate、能力探测新鲜度。它可以与 Coordination Store 使用同一 SQLite 实例，但必须在语义和表结构上分离。

观察缺失或矛盾时状态是 `UNKNOWN` 或 `RECONCILING`，不是 `STOPPED`。

### 2.4 Native Session Plane

由 Harness 自己保存 transcript、native session、checkpoint、tool history 和本地缓存。GOGO 只保存不敏感引用、摘要和连续性证明，不复制完整对话作为权威历史。

## 3. 单一逻辑 Controller 与 fencing

“单一 Controller”是逻辑权威，不要求永远只有一个物理进程。每个 Project/控制域同一时刻只能有一个持有有效 leader lease 的 Controller；每次接管获得单调递增 `controller_epoch`。

- 所有状态推进和 Runner Command 必须携带当前 epoch。
- Runner 必须拒绝旧 epoch 和旧 Assignment fencing token。
- Standby 可以读取和投影，但不得推进状态。
- Controller 失联后，接管者先 reconciliation，再决定是否重派。
- UI、Agent、Repository watcher 和 Runner 都不能绕过 Controller 直接推进权威状态。

## 4. 核心组件

### 4.1 Workflow / Gate / Policy

Workflow Engine 解释版本化 DAG、条件分支、fan-out/fan-in、重试、超时和人工暂停。Policy 回答动作是否允许：`ALLOW | DENY | ASK`；Gate 回答产出是否足以推进：`PASS | FAIL | INCONCLUSIVE | APPROVAL_REQUIRED`。两者不得混用。

手动、一键和自动运行都向同一 Transition Engine 提交 Command，只改变触发者和自动继续上限。

### 4.2 Scheduler

Scheduler/Controller通过Coordination Store的无过滤、稳定排序、versioned keyset stream读取该Project完整`SelectionUniverse`。Store只返回typed facts、opaque continuation token、selection version与完整end witness；不判断依赖、eligibility、capability、policy，不排名，也不选择winner。Controller/Core在事务外按版本化选择合同评估依赖、Harness capability evidence、ModelProfile、ProviderReadiness、Host execution profile、Project locality、credential reference、并发限制、预算、隔离和数据驻留策略，生成带完整 selection witness、TargetSelectionRecord、selected/relevant exact refs、IDs、epoch/fence和CAS期望的typed `ClaimAssignmentPlan`。

Store在短`BEGIN IMMEDIATE`事务内只验证current selection version、selected/relevant refs和exact plan，执行机械CAS并原子写Attempt/Claim/Lease/Command/Event/Outbox、bounded selection/component deltas、composition与outcome；它不得重新查询候选、重新选择或用SQL补一套eligibility状态机。selection version漂移时整个plan零写拒绝，Controller从新version重读完整stream。

Scheduler 不得因为候选不足而自动放宽权限或使用未验证能力。

### 4.2.1 Execution Target Registry

Registry 维护 Host 上经过探测的 HarnessInstallation、Adapter/Conformance、HarnessProfile、ModelProfile、CredentialHandle readiness 和 ProviderReadiness。Scheduler 只能从固定、project-scoped 的 target universe 选择，Attempt/Receipt 绑定精确 target digest。

Registry 不保存 secret，也不把本地 token 统计当官方额度。Harness、模型、Profile、账号句柄、Host 或协议等级变化必须重新选择；Adapter 无权自行 fallback。完整产品合同见 `19-execution-target-model-routing.md`。

### 4.2.2 ResourceBroker 与跨 Project 容量

ResourceBroker 位于 logical Controller/Core，使用同一完整 SelectionUniverse 同时评估 target 与 Host/Runner/Provider/Profile/Model 容量。它按 P31 D1 产品契约草案的 Project share（默认 weight 1）、priority、aging、reserve 与 starvation bound（等权双 Project 最多连续旁路 2 次）形成 exact CapacityPermit plan；SQLite 只在原子 claim 事务中做 version/fence/CAS。Runner 启动前必须验证 permit identity/generation/fence、selected target digest 与 Controller epoch（P41）。

Attempt/Claim/Lease/CapacityPermit/Command/Event/Outbox 必须同一事务成功或零写失败。Adapter/Runner 只报告脱敏 capacity/readiness observation，不排队、不优先选某个 Project、不释放 permit。完整合同见 `21-cross-project-resource-arbitration.md`。

### 4.3 Repository Projector 与 Reactor

- Projector：消费 transactional outbox，把确定性协议对象物化到 Git，并回报 object/commit hash。
- Reactor：发现本地文件、Git ref 或 GitHub webhook 变化，生成 Candidate Signal。

Watcher 只负责发现。Candidate Signal 必须经过 schema、project、producer、revision、权限、幂等和 Gate 验证；仓库中的任意文本或 commit message 不能直接启动 Agent。

### 4.4 Context Compiler 与 Session Governor

Context Compiler 只消费已经冻结的 `ContextCandidateSet` 与 `SelectionBasis`，再从其中的 Canonical Facts、Role、Assignment、Policy 和精确 repository revision 生成不可变 ContextSnapshot 和 EffectivePromptManifest，并通过 Harness-specific Projector 生成原生项目文档。Compiler 不得在编译过程中查询变化中的仓库、数据库或检索索引。

候选全集由 Controller 按 project-scoped 结构化 inventory 形成，并绑定 query snapshot、repository binding、source revision、visibility manifest 与 index watermark。P19 报告水位与迟滞；检索命中只是可重建 projection。Reveal Gate 前不得通过命中数量、排序、摘要或存在性泄漏其他 Auditor 内容。MVP 只要求结构化筛选；全文检索 UI 为 post-MVP。

Session Governor 综合事实漂移、任务边界、进展停滞、等待交互、Session 年龄、崩溃/限流/churn 与上下文使用情况，建议或执行安全轮换。UI 和 Adapter 不得仅凭 token 百分比、PTY idle 或 transcript 猜测换窗。轮换必须在安全点生成 provenance-bound Handoff，保留旧 Session，并如实命名 `WARM_REATTACH | NATIVE_FORK | COLD_REBUILD`。

### 4.5 Projection / Attention Service

UI 是四平面的证据投影，不是状态权威。每个状态显示：规范状态、观察状态、最后证据、证据年龄、当前命令/租约、下一安全动作和阻塞原因。

任何无法自动安全解决的冲突进入 Attention Inbox，例如：旧进程可能仍在写、commit outcome 不确定、仓库分叉、能力证据过期、盲审泄露风险或需要权限提升。

### 4.6 Host Runner

Runner 通过认证的出站连接注册，报告 Host execution profile 和 CapabilityEvidence，解析本机 credential handle，准备隔离 workspace，执行 fenced Command，监督进程并返回 RuntimeObservation 与 terminal receipt。

Runner 不上传原始凭据，不自行决定 Workflow，不把 stdout 文本中的“完成”当成成功，也不得在回执提交成功前销毁唯一证据。

### 4.7 Windows Local Control Host

Local Control Host 是 Windows MVP 的当前 OS 用户级后台单实例进程，不是 Windows Service。它承载 logical Controller、Coordination Store owner、reconciliation、Runner connection、Repository/Artifact service 和 notification broker。Desktop UI 通过本地认证 API 连接它；窗口关闭或崩溃不改变在途 Attempt。

Host 启动时先验证数据版本和 owner identity，再恢复 Store/outbox/spool 并对账 Runner/Harness/Git；完成前只允许诚实的只读 `RECOVERING/UNKNOWN` 投影。Pause、Stop、Quit 和 Force terminate 是不同 Command/Decision，不能映射成窗口事件。完整合同见 `20-local-control-host-lifecycle.md`。

## 5. 事务边界与自动接力

每次协调状态推进在一个数据库事务内完成：

1. 检查exact composition/selection/component predecessors、`expected_revision`、leader epoch、fencing token与幂等键；same key/same digest先查询原complete group。
2. 按typed effect map读取有界old refs、执行domain CAS，并生成至多256项、全operation canonical bytes合计不超过1 MiB的selection/component old-new deltas；任何漏axis、错axis或summary漂移都在COMMIT前失败关闭。
3. 按固定顺序追加selection batch（如有）、business/execution/controller component-delta root row（仅受影响axis）、唯一composition、durable outcome；component root是bounded delta-chain head + bounded current summary，不是全Project物化aggregate hash。
4. 写入同事务Accepted Event、审计记录和outbox，不在事务中调用Git、Runner或网络。Command semantic ACK是Command/Outbox/Event/selection/business/execution/composition/outcome的独立完整事务，不等同mechanical delivery ACK。
5. Dispatcher/Projector至少一次投递；接收端按原identity幂等处理。外部结果通过新事务确认；COMMIT响应不确定只查询原operation identity并按`ABSENT | COMPLETE | PARTIAL_CORRUPTION | INDETERMINATE`收敛。

候选读取使用同一selection version的跨短read-transaction keyset分页。token过期但version未变时续签并保留cursor/page chain；只有version或token epoch变化才从新`Begin`，不得给无界universe设置一个导致永久page-zero重启的总时限。

结果通过 Gate 后先进入 `PROMOTION_PENDING`；仓库物化成功后进入 `ACCEPTED`，随后同一 Transition Engine 解锁后继任务。这样人工按钮、本地回执和 Git 信号不会形成三套不同逻辑。

如果 Git commit 或 Runner command 的结果不确定，状态必须是 `COMMIT_INDETERMINATE` 或 `COMMAND_OUTCOME_UNKNOWN`，先查询/对账，不得盲目重放。

## 6. Reconciliation 循环

Controller 不假设命令必然成功：

1. 读取 Coordination Plane 的期望状态。
2. 获取 Runner、Git 和 Harness 的最新可验证观察。
3. 验证 Project、revision、epoch、fencing token 和 process identity。
4. 计算差异并选择最小安全动作。
5. 发送幂等 Command 或创建 Attention Item。
6. 收到证据后推进状态；无法证明则保持 `UNKNOWN`。

Lease 过期、心跳丢失或窗口关闭都不足以单独证明旧执行已停止。

## 7. 项目仓库拓扑

- Co-located：Source Repository 内含 `.gogo/`，适合单仓与低复杂度项目。
- Sidecar：独立 Control Repository 引用一个或多个 Source Repository，适合多仓、高并发或严格控制记录分离。

协议必须同时兼容两种拓扑。P31 D1 产品基数：MVP 只交付 Co-located，且一个 Project 恰好一个 Source Repository。Sidecar/多 Source 不进入 MVP；若后继启用，每个 source revision 必须绑定 `repository_binding_id`。领域对象不得把相对路径当作 Project 身份。

## 8. API 边界

- HTTP：资源查询、Command、配置和审批
- SSE/WebSocket：投影与 Attention 增量；仅为通知通道，不是持久事件总线
- Runner Protocol：注册、能力证据、fenced Command、观察、receipt 与 artifact upload
- Repository Protocol：可持久、可审阅对象的 Schema
- CLI：所有关键操作的非 UI 入口
- Local desktop IPC：Desktop UI 发现并认证单一 Local Control Host；不是第二套状态协议
- future MCP：Agent 操作入口，但不是内部唯一协议

## 9. Windows 与跨平台方向

Windows 是首个原生 Control Plane 验收平台，而不是承诺所有 Harness 的所有交互模式都能原生运行。执行能力分为：

1. Windows Native Control Plane
2. Windows Native SDK/headless CLI Runner
3. WSL Runner（需要 Linux PTY/tmux 或仅 Linux 支持的 Harness）
4. Remote Linux Runner

UI 必须显示每个 Host/Profile 的真实能力和退化项。Windows Job Object 可治理进程树，但不等于文件系统、网络或凭据沙箱。

安装包必须把 Desktop UI 与 Local Control Host 的生命周期、单实例、升级/回滚和数据目录兼容作为显式合同；不得让 Electron/窗口进程意外成为后台任务 owner。

## 10. Chief of Staff

Chief of Staff 是 **post-MVP** 路线。它只可生成 Proposal、调度建议和解释，必须通过同一 Policy、Gate 和 Controller。它没有隐藏的任务、审批、权限、删除、发布或终态 authority，也不承担协调数据库或 Adapter Kernel 的职责。MVP 验收不得依赖该角色存在。Hermes Studio 仅作为 `HS-*` 负向需求与产品壳参考，不进入生产依赖。

## 11. 治理 publication 与有效派发权

治理候选是静态、内容寻址的设计/施工输入，不在自身 commit 中记录该 commit ID，也不持久化 `effective_dispatch_authorized=true`。Owner 在提交候选后创建外部 annotated tag；tag message 绑定 manifest digest，Controller 从 tag object、peeled commit tree、Gate exact predicates、acceptance contract 和当前 route scope 派生唯一有效派发权。

缺 tag、轻量 tag、tag/commit 对象不匹配、manifest 或任一 governed file 的 commit-tree digest 不匹配时均保持 dispatch denied。`TECHNICALLY_READY` 是技术投影，不是权限；Desktop、API、CLI 和 Scheduler 必须消费同一个 Controller 派生字段，不能分别推断。
