# MVP 范围与验收

- 状态：v0.5-p31-d3-p17-namespace-handoff
- 原则：先证明一个真实、可恢复、不会双跑的纵向闭环，再扩展 Harness 数量和复杂编排
- P31：MVP 边界是 D1 草案加 D3 `P31-D3-P17-01` P17 namespace handoff；HS/DSH 只作为负向需求进入验收，不作为功能承诺

## 1. MVP 要证明什么

用户能在一个面板中为真实仓库创建 Implementer 和 Auditor，把一个 Task 从创建推进到结构化实施结果、自动审计接力和人工 Gate；全程无需跨窗口复制任务/结果，且 Controller 重启、重复信号或 Runner 短暂断线不会造成不受控双重执行。

## 2. 首发边界

### 2.1 包含

- Windows Native Control Plane，本地单用户、无需云服务
- SQLite 持久 Coordination Store + 分离的 Live Observation 表
- 一个 Project 恰好一个 Source Repository；Co-located `.gogo/` Control Repository。协议保留 sidecar 能力，但不纳入 MVP 交付
- 两个真实、通过 Conformance Suite 的 structured/headless Harness Adapter；两者都必须至少完成一次真实 Assignment/Receipt 闭环，并在同一个 Workflow 中分别承担 Implementer/Auditor 完成自动接力
- 产品所有者指定的首发 Spike 组合：Codex + Grok CLI
- Grok 必须使用公众可获取的官方发行版完成安装与验收；私有 Provider/账号池不计入 MVP 能力证明
- 一个 Codex/Grok 使用不同精确 ExecutionTarget 的 Implementer -> 独立 Auditor -> 人工 Gate 工作流
- Task dependency、原子 claim、lease/fencing、Command outbox、receipt commit fence
- ContextSnapshot、EffectivePromptManifest 和 Harness 项目文档自动生成
- ResultCapsule、Evidence、Decision 与 Git canonical materialization
- 手动、单步一键、运行到下一个人工 Gate三种模式，共用 Transition Engine
- Session 健康提示和一键安全轮换；只在 Task 边界允许策略化自动冷重建
- 最小面板：Project/Agent/Task 状态、证据、Attention、时间线和停止控制
- Windows Local Control Host：当前 OS principal 的 per-user 后台进程（非 Service）、UI 关闭后后台继续、单实例、重启 reconciliation、按需启动、登录自启 opt-in、托盘/通知和可恢复首次启动
- Harness 缺失时仅 detection + manual official guide；登录/MFA 由用户在官方流程完成；检测回执不得伪装认证成功
- 第二个 Project 同时保持一个活跃 Attempt 的隔离负向测试；不要求两个 Project 都跑复杂 Workflow
- 版本化加密 portable backup/恢复、Host-bound credential/native Session 的 rebind/reauth 语义和卸载默认保留数据
- Context Workspace、`ContextEntry`/`CollaborationProposal`、display-only reaction、结构化 mention/reply，以及冻结 `ContextCandidateSet` 上的结构化筛选
- P17 作为唯一 Project/worktree namespace owner：immutable `Project -> SourceRepository/repository_binding_id`；已注册 runner root 下 `project_id/assignment_id/attempt_id`；source write 前 binding/fence 校验；dirty workspace 只经 Attention/human Gate

### 2.2 不包含

- Web Chat Agent 自动化
- 通用 Hermes/Chief of Staff 自主规划或任何隐藏 authority
- 十 Auditor fan-out、复杂 quorum、跨项目 Publication UI、reaction quorum
- Remote Linux Runner 与组织级 RBAC/SaaS
- 自动 merge/deploy/release
- Generic PTY 作为成功回执的首发主路径
- 完整 transcript 汇聚、语义向量记忆、跨项目知识库或全文检索 UI
- Controller-authorized one-click official install plan（超出 P11/P35/P23/P24 时须新开工作包）
- 多 Source Repository / sidecar 作为可交付拓扑
- `ReactionSignal` 作为权威对象
- 无限自动运行；默认只运行到下一个人工 Gate
- 把 `HS-*` / `DSH-*` 参考场景改名为已实现 Fake PASS 或加入生产依赖

## 3. 实现前 Kill Gate：Adapter Spike

在选技术栈和写完整控制面前，对两个候选 Harness 分别证明：

1. Windows Native 或明确 WSL Profile 下可非交互启动。
2. 能稳定关联 native session/process 与 GOGO attempt。
3. 能生成结构化 terminal receipt，或有可控 wrapper contract。
4. 能固定 cwd/source revision 并加载生成项目文档。
5. start/cancel 重复投递不双启，旧 fencing token 可拒绝。
6. 认证探测不读取或上传 secret。

任一 Harness 失败时必须形成能力证据和 Replan Decision，由产品所有者决定是否降级能力或更换候选；不得静默换成其他 Harness，也不得用 stdout heuristic 假装通过。Spike 只产出证据和决策，不开始大规模产品实现。

## 4. MVP 纵向流程

```text
Create Project + bind exactly one Source Repository
  -> establish operator_id + local Control API authentication
  -> detect public Codex/Grok or show manual official guide
  -> user-visible login (never implied by detection)
  -> copy Party/Role template as a new version
  -> create Party/Implementer/Auditor
  -> compile context + verify projections
  -> create Task
  -> claim/dispatch Implementer
  -> terminal receipt + ResultCapsule
  -> Gate + Git materialization
  -> automatically ready/claim Auditor
  -> independent audit receipt
  -> human Gate
  -> Task COMPLETED or correction Task READY
```

人类随时可以暂停“继续认领新任务”，但已运行 Attempt 的终止必须等待真实回执。

## 5. 最小 UI

### 5.1 Project 视图

显示 Project identity、repository revision/sync、Party、Workflow、预算、自动化模式和 Attention 数量。

### 5.2 Task/Agent 视图

每个 Task/Agent 显示规范状态与现场状态两列，并显示 Assignment/Attempt、Runner、Harness/版本、Session、Context Health、lease、最后证据时间、当前动作和下一安全动作。

### 5.3 时间线与 Attention

能从 Task 追踪 Intent、Decision、Command、ACK、Observation、Receipt、Git commit、Gate 和后继 Assignment。`UNKNOWN`、`INCONCLUSIVE`、旧能力证据、commit 不确定和权限请求不得被绿色状态掩盖。

## 6. 必须通过的验收场景

### A. 基本闭环

真实 Implementer 在固定 revision 上产出 ResultCapsule；Controller 验证并物化到 Git；Auditor 自动接力且只看到允许上下文；人工 Gate 后 Task 正确进入 COMPLETED 或 correction READY。

### B. 三种触发同构

分别用人工“开始”、一键“执行下一步”和“运行到人工 Gate”执行等价流程。三者产生相同状态机、Policy/Gate、Command/Receipt 和审计结构，只有触发 actor/automation limit 不同。

### C. 重复与乱序

重复提交 webhook、Command ACK、Observation 和 TerminalReceipt，不产生重复 Attempt、Result、Git 物化或后继 Assignment。乱序旧事件被拒绝或保留为非权威观察。

### D. Controller 崩溃恢复

分别在 DB outbox 提交后、Runner 启动前后、receipt 接受前后、Git commit outcome 不确定时强制重启。系统通过 idempotency/reconciliation 收敛，不双跑、不伪造成功；无法证明时进入 Attention。

### E. Runner 失联与 Lease 过期

Runner heartbeat 中断后状态为 UNKNOWN。仅 Lease 过期不得在另一 Runner 重启可写 Attempt；必须获得旧进程终止证据或人工隔离决定。

### F. Revision 与上下文漂移

Assignment 启动前 source revision 改变或 Prompt projection hash 不符时阻止执行。运行中发生重大 Fact/Policy 漂移时进入 BLOCKED/Attention，不静默注入。

### G. Session 轮换

在 Task 安全边界生成 Handoff Capsule，执行 cold rebuild，新 Session 连续性握手 PASS 后继续。握手故意提供错误 project/revision 时必须 FAIL。

### H. 项目与 Auditor 隔离

建立 Project B，尝试通过 API、event stream、cache、artifact path、Session ref 和 credential handle 读取 Project A，全部拒绝并留审计。Auditor 在 Reveal Gate 前无法读取 Implementer 未允许材料或其他审计结论。

### I. 权限与危险动作

Task 完成后尝试 merge/deploy/delete；无独立授权时必须 DENY/ASK。Agent 输出中的自然语言授权无效。

### J. Adapter 能力诚实性

修改 Harness 版本或使能力证据过期后 Scheduler 不再把它当作已验证候选。Generic PTY 没有 receipt contract 时不能让 Task 变绿。

### K. 公众发行与 Profile 隔离

Codex 与 Grok 的安装探测必须从官方公众发行入口完成并固定 version/hash/signature。私有 Provider、账号池或内部 shim 不计入验收。分别在 shared 与 dedicated HarnessProfile 下验证：凭证不泄露、用户级配置告警可见、Project Workspace/Session 不串线；认证响应和账号限额不得进入 Project 事件流或 Git。

### L. 跨 Harness 自动接力与目标诚实性

同一 Project/Workflow 中，Implementer 与 Auditor 必须分别使用 Codex 和 Grok 的精确 ExecutionTarget；前序 Accepted Result 自动使后继 READY/claim，全程无需人工复制任务、结果或上下文。Attempt/Receipt 显示 requested/selected/observed target。尝试静默切模型、Profile、CredentialHandle、Host 或协议等级必须被拒绝；显式 fallback 产生新 Attempt/target generation 并重新编译 Context。

### M. 两 Project 并发隔离

Project A 与 B 同时各有一个活跃 Attempt。对 workspace、native Session/cache、event cursor、Context candidate、CredentialHandle/ProviderReadiness、budget reservation、CapacityRequest/Permit、Artifact、Attention 和通知 deep link 做双向越界测试。暂停、停止、切换或恢复 A 不得改变 B；测试结束前不能仅靠 UI 过滤声称隔离。

同时验证共享 Host/Runner/Provider/Profile 下的 weighted-fair admission、有界 aging、Pause 不新增 permit、旧 fence/duplicate grant 拒绝、Host 崩溃后 permit reconciliation，以及公平调度不泄露另一 Project 身份。P17 namespace 在该双 Project 场景下必须 fail-closed：任一方不得写入对方 `project_id/assignment_id/attempt_id` worktree。

### N. Local Control Host 与大众产品生命周期

完成首次启动、repository/Harness/Profile/Party 配置和 LoadProof；验证 per-user Host 按需启动、UI close/crash 后 Attempt 继续、UI 重连补读、单实例、Windows restart 后 reconciliation、Pause/Stop/Quit/Force terminate 区分、通知迟到/重复/旧 revision，以及 encrypted backup/restore 损坏/不兼容/Host-bound 缺失/rekey/reauth。卸载默认不删除 Project 数据。

### O. P17 Project/worktree namespace fail-closed

P17 是唯一 namespace owner。两个 active Project 的 source write 必须落在各自已注册 runner root 下的 `project_id/assignment_id/attempt_id`，并绑定当前 Controller epoch/fence 与 Runner/process identity。下列行为全部 zero-write reject，且不得 auto-clean、auto-reuse 或 fallback：

- name collision：用显示名、目录名或另一 Project 的 path 段撞击目标 namespace
- path traversal：`..`、绝对路径、junction/symlink 逃出 registered runner root
- stale Attempt：旧 `attempt_id`、旧 epoch、旧 fence 或过期 worktree lease
- dirty workspace：未授权修改时进入 Attention/human Gate，不把脏树当作可写或可复用 root

P16/P33/P18/P22 若尝试另建 namespace、root 或 source-write 路径，验收失败。

## 7. 非功能验收

- 本地重启后持久任务、claim、命令、回执与 Git 物化关系可恢复。
- UI 关闭或崩溃不终止 Local Control Host/Runner；UI 重连可从持久 offset 补齐时间线。
- Local Control Host 崩溃或 Windows 重启后先 reconciliation，再开放新 claim；无法证明时保持 UNKNOWN/Attention。
- Project scoped 负向测试和 Adapter Conformance Suite 自动运行。
- 核心对象有 versioned Schema 与 migration 测试。
- secret 不出现在仓库、API payload、日志和测试 fixture。
- 状态投影延迟有指标，且证据年龄可见。

## 8. MVP Exit Gate

只有以下条件同时满足才退出 MVP：

1. 两个真实 Adapter 的声明能力有当前版本 Conformance Evidence。
2. A-O 场景全部有可复现证据；未执行项不能记 PASS。
3. 至少一次故障注入证明无双重可写执行。
4. 至少一次 Session 轮换证明上下文连续性。
5. 两个 Project 同时存在活跃 Attempt 的隔离负向测试通过。
6. Open Questions 中影响数据模型或 Runner Protocol 的问题已由产品所有者决定。
7. 同一 Workflow 的 Codex/Grok 自动接力、Local Control Host 生命周期和备份/恢复矩阵均由同一固定 build 通过。
8. 轻量 tag、伪造 40 字符 commit、移动 tag、manifest 不在 peeled commit tree、任一 governed file digest 漂移和无关 `PASS` Gate 谓词均无法开启 dispatch；所有 UI/API/CLI 显示同一 derived authorization。

随后再进入 fan-out 多 Auditor、sidecar/remote Git、WSL/Remote Runner、Chief of Staff、全文检索 UI、结构化 Vote/Decision、独立官方安装工作包和 Web Chat Agent 阶段。

## 9. MVP 负向需求（参考车道，非功能）

下列 ID 进入 MVP 验收的负向不变量，实现状态仍是 `NOT_IMPLEMENTED` 直到对应工作包落地。它们不是产品功能，不得成为生产依赖，也不得改名为 Fake PASS：

- `HS-01`–`HS-06`：无 Receipt 不得投影成功；迟到审批 ACK 拒绝；origin Session 不可解不得 SUCCEEDED；双 Session 存储分叉先 reconciliation；无 scheduler lease 不得派活；transcript/`@名字` 不得创建 Assignment。
- `DSH-01`–`DSH-07`：插件卸载不得抹掉 in-flight 工作；重复/回退 nativeSeq 不得写第二条权威事件；过期 subagent 终态不得成为当前 Receipt；缺 scope 的 API 投影拒绝；Harness worker 重启只报 UNKNOWN；不完整 Windows ACL 不得宣传沙箱；包版本漂移不得套用未解析 pin。
