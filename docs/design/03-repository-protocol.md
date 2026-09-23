# Repository Protocol

- 状态：v0.2 正式设计候选
- 目标：让本地 Git 与带 Remote 的 Git 仓库都能承载可审阅事实与成果，而不承担实时队列和锁服务

## 1. 定位

Control Repository 是 Canonical Fact Plane 的交换、审阅和恢复载体。它保存 Task 定义/快照、ResultCapsule、Fact、Evidence 引用、Decision、Party/Role/Workflow/Policy 版本和物化记录。

它不负责：

- Assignment 原子认领
- Lease、heartbeat、retry timer
- Runner Command 投递
- 消费位点和低延迟通知
- 判断进程当前是否存活

这些属于 Coordination Plane 或 Live Observation Plane。不得以创建 lock 文件、抢先 commit、分支命名或 JSONL 合并模拟安全租约。

## 2. 推荐目录

```text
.gogo/
  project.yaml
  party/
    party.yaml
    roles/<role-version>.yaml
    agents/<agent-id>.yaml
  workflows/<workflow-version>.yaml
  policies/<policy-version>.yaml
  ledger/
    tasks/<task-id>/<task-revision>.yaml
    results/<assignment-id>/<attempt-id>.json
    facts/<fact-id>/<fact-revision>.json
    decisions/<decision-id>.json
    events/<yyyy-mm>/<event-id>.json
    manifests/<materialization-id>.json
  proposals/
    <assignment-id>/<attempt-id>/result.json
  contexts/
    <snapshot-id>/manifest.json
  evidence/
    <evidence-id>.json
  publications/<publication-id>.json
```

大型 artifact 不强制进入 Git；仓库只保存 URI、content hash、大小、媒体类型、保留策略和访问范围。

`proposals/` 是受约束的入站区，不是权威区。Agent 可以提交自己 Assignment/Attempt 的 Proposal；只有 Repository Projector 可以写入 `ledger/` 中的已接受对象。

`ledger/tasks/` 保存不可变 Task 定义和里程碑快照，不充当当前 claim/lease 文件。活跃状态仍以 Coordination Plane 为准；数据库丢失时可恢复定义与已物化里程碑，但 in-flight Attempt 必须 reconciliation。

## 3. 协议对象信封

每个对象至少携带：

```yaml
schema_version: gogo/v1alpha1
kind: ResultCapsule
id: stable-id
project_id: project-id
revision: 1
created_at: RFC3339
producer:
  actor_type: agent
  actor_id: agent-id
causation_id: command-or-event-id
source_revisions:
  - repository_id: source-main
    base_commit: full-sha
    observed_commit: full-sha
content_digest: sha256:...
```

分支名、`HEAD` 字样、短 SHA 和“latest”不能代替完整 revision。脏工作区必须记录 base commit、补丁/dirty witness hash 和明确边界；不得把未验证脏状态描述为 commit 事实。

## 4. 写入路径

### 4.1 Runner/API 快速路径

1. Runner 生成 terminal receipt 与 ResultCapsule，保留本地耐久副本。
2. 通过 Runner Protocol 提交；Controller 验证 project、assignment、attempt、lease/fencing、schema、revision 和 producer。
3. Coordination transaction 接受 Proposal 并写入 outbox。
4. Repository Projector 确定性写入 `proposals/` 或直接物化候选对象。
5. Gate PASS 后写入 `ledger/`；获得本地 Git object/commit hash。
6. Controller 记录物化回执，状态从 `PROMOTION_PENDING` 进入 `ACCEPTED`。

### 4.2 Repository 入站路径

Agent、人工或外部工具也可以直接创建 Proposal commit。Reactor 只把变更转换为 Candidate Signal，随后执行与 API 快速路径相同的验证与 Transition。文件存在或 webhook 到达本身不表示接受。

两条路径必须用同一 `idempotency_key` 收敛；同一 Proposal 经 API 与 Git 重复到达只处理一次。

## 5. 原子性与物化

单个文件必须以临时文件 + fsync/flush + atomic rename 的方式写入，再执行 Git add/commit。多对象领域变更由一个 `materialization manifest` 列出精确对象、Hash 和预期前序 revision。

文件系统原子写入不等于 Coordination DB 与 Git 的跨系统事务。因此使用 saga：

```text
DB transition + outbox
  -> deterministic repository materialization
  -> record commit/object hash
  -> finalize canonical acceptance
```

Projector 崩溃后按 `materialization_id` 查询 Git 和 manifest；若 commit outcome 不确定，进入 `COMMIT_INDETERMINATE`，不得直接再 commit 一份语义相同但 ID 不同的对象。

### 5.1 Git writer 隔离

每个 Control Repository 的 Projector 写入必须在 Coordination Store 中串行化，并对预期 control ref 执行 compare-and-swap。Projector 使用专用干净 worktree/clone，只允许 stage manifest 列出的 `.gogo/**` 路径；不得在用户或 Agent 的脏工作区运行 `git add -A`。

RepositoryBinding 必须声明 `control_ref`。在 Co-located 模式中，它可以是专用 GOGO ref，也可以由 Project Policy 显式选择与 source integration ref 共线；无论哪种都不得把 task worktree 的未接受改动混入 control commit。

## 6. Local 与 Remote

- 本地 Control Repository commit 默认足以完成本地 canonical materialization。
- Remote push 是可观察的同步状态：`NOT_CONFIGURED | PENDING | PUSHED | DIVERGED | FAILED | UNKNOWN`。
- Project Policy 可以要求 `PUSHED`、PR、CI 或人工审批作为后续 Gate，但不得把 remote 可用性暗中混入所有项目。
- webhook 只是唤醒提示；Reactor 必须 fetch/verify 精确 ref 后再处理。

## 7. 冲突与分叉

发现非快进、对象同 ID 异内容、expected revision 不匹配或 ledger 被外部改写时：

1. 停止相关 Task 的自动推进。
2. 保留两侧 commit/hash 与差异证据。
3. 创建 Attention Item，状态为 `DIVERGED` 或 `CONFLICT`。
4. 只有显式 Decision 可以选择接受一侧、创建新 revision 或撤销对象。

不得由 Git merge driver 静默解决语义冲突。

## 8. 生成的 Harness 项目文档

生成文件必须包含或伴随：`generated=true`、`context_snapshot_id`、`effective_prompt_digest`、生成器版本和源对象 Hash。它们可以位于隔离 workspace 而不提交到源仓库。

Agent 修改生成文件只形成 drift observation，不会反向修改 Role、Policy 或 Fact。需要永久改变时必须提交独立配置 Proposal。

## 9. 保密与完整性

- 仓库不得包含 API key、OAuth token、cookie、私钥或原始 secret path。
- Evidence 可引用受保护存储，但必须记录 Hash 和访问策略。
- 每个入站对象验证 producer scope；Agent 不能冒充 Controller 写 accepted ledger。
- 可选签名用于跨机器/远端来源证明，但签名不能替代授权与 Gate。
- transcript 默认不入库；只有经过选择、脱敏和来源标注的片段才能作为 Evidence。
