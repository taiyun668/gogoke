# ADR-0009：通用内核实现主干与证据边界

- 状态：Accepted（M0 Owner Authorization，2026-08-11）
- 决策引用：`docs/plan/decisions/M0-2026-08-11-owner-authorization.md`
- 日期：2026-08-11

## Context

GOGO PARTY 已确定四平面、单一逻辑 Controller、项目隔离和能力证据原则，但这些规范仍允许实现走向多套进程监督器、进程内专有 Adapter、由 PTY 推断完成，或让桌面 UI 持有协调权威。源码审计同时发现：xAI Grok Build、OpenAI Codex、Claudexor、Beads 等项目各自在局部机制上更强，不能把任一完整产品直接作为 GOGO 内核。

本 ADR 固定实现主干、组件责任和进入纵向原型前的选择 Gate；它不授权复制任何候选上游源码。

## Decision

### 1. 实现主干

1. Control Plane、Coordination Store、Runner 和内置 Adapter 的首选实现语言为 Rust；首版持久协调存储为 SQLite。SQLite 是需迁移、备份和故障恢复的权威协调数据库，不是可删除缓存。
2. Desktop/Web UI 使用 React/TypeScript，只通过版本化 Control API 和事件投影工作，不链接协调数据库、Harness SDK 或 Native Session store。
3. Adapter 的规范边界是版本化、可生成 JSON Schema 的线协议，不是 Rust trait ABI。内置 Codex/Grok Adapter 可以与 Runner 同进程部署，但必须通过与未来外部 Adapter 相同的消息模型和 Conformance Suite，不能泄漏 vendor 类型。
4. Rust/SQLite/React 是 v0.2 的实现候选主干；只有本 ADR 被接受且以下 Gate 通过后，才可冻结依赖版本和建立生产 workspace。

### 2. 权威归属

- Controller 拥有 Project、Agent、Task、Assignment、Attempt、Workflow、Policy、Gate、ContextSnapshot、Session rotation 决策和 Accepted Event。
- Coordination Store 原子持有 Assignment claim、lease、单调 fencing token、Command、idempotency、outbox、ReceiptRecord 和消费位点。
- Runner 拥有 Host-local process scope、耐久 spool、workspace、credential handle 解析和现场证据收集，但不能推进 Workflow。
- Adapter 只负责 detect/doctor、经批准的 install、start/resume/send/cancel、vendor 事件翻译、原生终态证据和目标 Harness 的上下文投影。它不得 claim、重试、判定 Gate、提升 Fact 或决定换窗。
- UI、Repository watcher、Hook、PTY parser、transcript 和 Chief of Staff 只能提交 Intent、Candidate Signal、Observation 或 Proposal。

Claim/lease/fencing 的执行作用域是 `project_id + assignment_id + attempt_id`，不是只有 Task。一个 Task 可以合法 fan-out 多个独立 Assignment；每个 Assignment 的 fencing token 独立单调递增。

### 3. 唯一公共 Process Supervisor

GOGO 只允许一套公共 Supervisor 抽象。固定最小接口语义：

```text
spawn(SpawnSpec, ScopePolicy, OutputPolicy, SecurityProfile) -> RunHandle
RunHandle.write / close_stdin / resize
RunHandle.observe(from_sequence)
RunHandle.interrupt / terminate(grace) / kill / close
RunHandle.wait -> RunnerExit
recover(scope_id, runner_epoch) -> RecoveryReport
capabilities -> CapabilityEvidence
```

`RunHandle` 使用不可复用的 scope identity/epoch，不把 raw PID 暴露为长期身份。关闭与 spawn 必须线性化；关闭幂等；Windows 必须在子进程可运行前完成 Job containment，不能把普通 pipe 的 post-spawn attach 伪装为 tree-safe。输出通道可以有界，但任何丢失、截断或落盘降级都必须产生 sequence/loss evidence；进程退出码不是任务回执。

xAI `ProcessScope` 的 weak-owner、PID 复用防护和 close/spawn 竞态测试，与 Codex suspended-spawn Windows Job、macOS fallback 和 Unix hardening 一起进入有限 bake-off。实验前不同时引入两套实现，也不把任何候选升级为 `SOURCE-DERIVED`。

### 4. Adapter 证据与回执链

证据强度按以下顺序降级，降级必须形成可见事件：

```text
official structured protocol
  > fresh official hook
  > owned process lifecycle
  > PTY parser
  > transcript / idle / heuristic
```

低等级证据可以补充活动和诊断，不能覆盖高等级终态，也不能自行提升完成状态。Generic PTY Adapter 没有机器协议或 wrapper receipt contract 时，只能提供退化的启动、观察、人工 attach 和 best-effort cancel，不能用于自动权威接力。

回执链固定为：

```text
Adapter NativeTerminalEvidence
  -> Runner validates current session generation/turn fence
  -> Runner assembles TerminalReceipt + ResultCapsule/artifact/revision digests
  -> Runner durable spool
  -> Controller validates project/assignment/attempt/epoch/fencing/idempotency
  -> ReceiptRecord committed + receipt.committed ACK
  -> Policy/Gate evaluation
  -> PROMOTION_PENDING -> repository materialization -> ACCEPTED
```

Adapter 不签发 GOGO TerminalReceipt；Controller 也不在 Gate 后倒造运行回执。重复终态、迟到 generation、旧 fencing token、相同幂等键不同 payload 都进入冲突或 reconciliation，不采用最后写入获胜。

### 5. Context Compiler 与 Projector

Control Plane 以确定性 overlay 编译不可变 ContextSnapshot/EffectivePromptManifest：

```text
platform invariant
  -> project default
  -> role
  -> project-role override
  -> agent/member override
  -> assignment/task instruction
```

每个字段使用显式 `append | replace | set_union | exclusive` 合并规则；mandatory 片段不能被更具体层删除或遮蔽。Compiler 必须输出 canonical hash 以及完整 `included / omitted / conflict` manifest。

Adapter-local Projector 只选择 vendor 协议字段或原生文件路径，执行路径逃逸/敏感文件/用户文件覆盖保护，并返回 projection hash 与 load proof。写入成功不等于 Harness 已加载；缺 mandatory、冲突、hash 漂移或缺少所需 load proof 时 fail-closed。Windows 必须支持经过 doctor 验证的 copy 路径，symlink 不能是唯一实现。

Session Governor 根据 task boundary、revision/instruction drift、stall、runtime health、原生 context evidence、交接准备度和安全点决定轮换；Adapter 只上报证据，不自行换窗。token 百分比或 idle 不得单独触发轮换。

### 6. 第一条纵向切片

实现顺序被冻结为：

1. Schema-first 的领域 ID、Command/Event、Adapter wire、Context manifest 和 Receipt schema。
2. SQLite migration + Assignment CAS/lease/fencing/idempotency/outbox；先用 fake Runner。
3. 唯一 Process Supervisor bake-off 与故障注入；选择后再形成独立来源/许可 ADR 或更新本 ADR。
4. Fake Codex/Fake Grok 通过同一 Adapter Conformance，证明 native 类型不越界、重复/乱序不产生双回执。
5. Context Compiler + 两个 fake Projector，通过 mandatory/load-proof/Windows copy 负测。
6. Fake Implementer -> fake Auditor -> human Gate 的端到端闭环，证明人工、一键、运行到 Gate 共用同一 Transition Engine。
7. 最后才接公众版 Codex app-server 和公众版 Grok ACP；完整 Dashboard 在纵向闭环之后。

## Required Gates

- N 个并发 claim 只有一个 Assignment/Attempt/fence 获胜；旧 fence 的 heartbeat、artifact、receipt 和完成全部拒绝。
- 注入数据库 COMMIT、Runner ACK、Git materialization 结果未知；系统只查询/reconcile，不盲重放外部副作用。
- Windows root/child/grandchild、nested Job、spawn/close race、PID reuse、重复 close、grace-to-kill 和 host crash 测试诚实报告退化项。
- 慢消费者和大输出不会静默丢失 terminal boundary；任何截断带 sequence/loss evidence。
- official protocol/hook 断开、PTY 输出“成功”、进程 exit 0、迟到 terminal event 都不能伪造 TerminalReceipt。
- 四层项目 overlay、mandatory 缺失、冲突、path/symlink escape、用户文件碰撞、hash 漂移和 load-proof 过期全部 fail-closed。
- Project B 不能通过数据库键、cache、event cursor、workspace、session、artifact 或 credential handle 引用 Project A。

## Rejected Alternatives

- 让 Tauri/React UI、Git watcher 或 Harness Session 成为协调权威。
- 为每个 Harness 维护一套 process supervisor、任务状态机或自动接力逻辑。
- 把 Rust trait ABI 作为第三方 Adapter 的唯一扩展面。
- 使用 Task 级单一 lease 阻止合法的多 Assignment fan-out。
- 让 Adapter 自报最终 GOGO Receipt，或让 Controller 在 Gate 后倒造运行回执。
- 把 PTY idle、终端最后一行、assistant 自述、exit 0 或 Prompt 当作完成/权限证据。
- 违反 ADR-0008，检测、启动、连接或依赖 Grok App 的进程、RPC、IPC、store 或 Native Session；Grok 首发路径只能直连公众版官方 Grok CLI。
- 在模块级 provenance、依赖、NOTICE、平台和测试 Gate 前复制候选源码。

## Consequences

- 新 Harness 主要增加 Adapter、Projector 和 Conformance Evidence，不修改 Controller 领域语义。
- 首版会先出现可测试的无 UI 纵向内核；Dashboard 是同一 API 的投影，不形成第二套业务逻辑。
- Rust 核心降低 Windows 进程治理和官方 Rust 协议候选的整合成本，但第三方扩展不会被 Rust ABI 锁死。
- SQLite 满足本地优先单逻辑 Controller；未来更换存储必须保持 CAS、fencing、outbox 和 reconciliation 语义，而不是只迁移表结构。
- 源码候选的最终采用需要单独可追溯证据；本 ADR 接受也不等于上游测试已通过。

## Evidence

- `docs/research/source-audit/06-runtime-protocol-and-adapter-sources.md`
- `docs/research/source-audit/07-ledger-workflow-context-sources.md`
- `docs/research/source-audit/08-observability-dashboard-and-shell-sources.md`
- `docs/research/source-audit/09-cross-repository-extraction-matrix.md`
- `docs/design/11-source-audit-corrections.md`
- `docs/design/12-schema-first-protocol.md`
- `docs/design/13-sqlite-coordination-spec.md`
- `docs/design/14-fake-conformance-vertical-slice.md`
- `docs/design/15-v1alpha1-machine-contract-review.md`
- `docs/design/16-sqlite-machine-contract-review.md`
- `docs/design/17-fake-kernel-executable-subset-review.md`
- `spec/draft/v1alpha1/README.md`
