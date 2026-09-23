# 工作流、命令与事件协议

- 状态：v0.2 正式设计候选
- 目标：让人工启动、一键接力、全自动接力、重复投递和故障恢复共享同一套语义

## 1. 协议分类

- Intent Command：人类或上层 Agent 请求系统采取动作。
- Runner Command：Controller 持久化后要求 Runner 执行的 fenced 动作。
- RuntimeObservation：Runner/Harness 观察到的现场证据。
- TerminalReceipt：一次 Attempt 的结构化终止回执。
- Candidate Signal：Repository/API/webhook 等入口发现的、尚未信任的信号。
- Accepted Event：Controller 验证并在 Coordination Store 中接受的事实。

Event 流是至少一次投递；所有消费者必须幂等。SSE/WebSocket 仅用于 UI 通知，断线后从持久 offset 补读。

## 2. 状态机

Task 与 Assignment 使用领域模型中的状态机。Attempt 另有运行状态：

```text
CREATED -> COMMAND_PENDING -> ACKNOWLEDGED -> PROCESS_STARTED
        -> ACTIVE | WAITING | OUTCOME_UNKNOWN
        -> RECEIPT_COMMITTED -> SUCCEEDED | FAILED | CANCELLED
```

`PROCESS_STARTED` 需要 native session/process identity 证据。模型输出一句“完成了”不能推进任何终态。

## 3. Command 信封

```yaml
schema_version: gogo/runner-command/v1alpha1
command_id: cmd-...
command_type: assignment.start
project_id: project-...
workflow_run_id: run-...
task_id: task-...
assignment_id: assignment-...
attempt_id: attempt-...
agent_id: agent-...
runner_id: runner-...
idempotency_key: assignment:start:attempt-...
expected_revision: 7
controller_epoch: 12
fencing_token: 3
issued_at: RFC3339
expires_at: RFC3339
context_snapshot_digest: sha256:...
effective_prompt_digest: sha256:...
capability_requirement_digest: sha256:...
policy_decision_id: decision-...
```

同一幂等键与同一 payload 重复到达必须返回原结果；同一幂等键对应不同 payload 必须拒绝并产生安全事件。

## 4. Observation、Receipt 与 Event 信封

公共字段：

```yaml
id: stable-id
project_id: project-id
correlation_id: correlation-id
causation_id: preceding-command-or-event-id
producer_id: runner-or-controller-id
producer_sequence: 42
observed_at: RFC3339
received_at: RFC3339
payload_digest: sha256:...
```

TerminalReceipt 还必须包含 command/attempt/session、terminal status、exit/stop reason、ResultCapsule ID、artifact/evidence refs、source revision、adapter/native session refs、usage（若可验证）和边界声明。

Runner 在收到 Controller 的 `receipt_committed` 确认前必须保留耐久副本。提交超时后先按 `receipt_id` 查询，不得重新运行 Attempt 来“确认”。

## 5. 原子认领与租约

Scheduler 的 `claimNext` 必须在单个事务中完成：

1. 选择依赖满足、未被有效 claim、capability/policy 合法的 READY Assignment。
2. compare-and-swap `assignment_revision`。
3. 创建 Claim、Lease、Attempt 和单调递增 fencing token。
4. 写 `assignment.claimed` Accepted Event 与 dispatch outbox。

Heartbeat 只能续当前 holder + fencing token 的 Lease。过期 Lease 不自动触发重派；先确定旧进程不可再写，或给新 Attempt 使用全新只写命名空间并隔离旧写入。

## 6. 自动接力

```text
Agent A terminal receipt / repository proposal
  -> ingest + dedupe
  -> validate project, producer, attempt, revision, lease/fencing
  -> ResultCapsule submitted
  -> schema/evidence/policy/task Gate
  -> PROMOTION_PENDING
  -> repository materialization + commit hash
  -> Result ACCEPTED / Task transition
  -> dependency evaluation
  -> Assignment B READY
  -> atomic claimNext
  -> fenced Runner Command
  -> Runner ACK + process evidence
  -> Agent B RUNNING
```

人工“开始下一步”、一键“运行到下一个人工 Gate”和自动模式都从依赖评估或 claimNext 进入，不得直接启动进程。自动模式只改变是否自动提交下一 Intent Command。

## 7. Gate 与 fan-out/fan-in

Gate 输入必须固定到精确 ResultCapsule、Evidence、ContextSnapshot、Policy 和 source revision。fan-out 为每个 Auditor 创建独立 Assignment/ContextSnapshot/结果命名空间；盲审在 Reveal Gate 前不得向 Auditor 暴露其他结论。

fan-in 可声明：

- `all_required`
- `quorum(n)`
- `any_pass`
- `human_decision`

缺失结果、证据不可读或 auditor 独立性无法证明时为 `INCONCLUSIVE`，不是 FAIL 或 PASS。

## 8. 失败分类与安全动作

| 分类 | 示例 | 默认动作 |
|---|---|---|
| `PRECONDITION_FAILED` | revision/policy/capability 不匹配 | 不启动，重新规划或人工处理 |
| `DISPATCH_REJECTED` | 旧 epoch/fencing、命令过期 | 刷新状态，不盲重试 |
| `PROCESS_START_FAILED` | binary 缺失、workspace 失败 | 结束 Attempt，可按策略新建 Attempt |
| `RUNTIME_LOST` | Runner 断线 | `UNKNOWN` + reconciliation |
| `RECEIPT_INVALID` | schema/producer/evidence 错 | 拒绝 Proposal，保留证据 |
| `COMMAND_OUTCOME_UNKNOWN` | ACK 丢失 | 查询 command/process identity |
| `COMMIT_INDETERMINATE` | Git commit 返回不确定 | 查询 manifest/object，不重复物化 |
| `POLICY_DENIED` | 越权 | 停止并记录 Decision |
| `GATE_INCONCLUSIVE` | 证据不足 | Attention 或补证据任务 |

只有明确列入 retry policy 且已证明无双写风险的失败才可自动重试。每次重试生成新 Attempt 和 fencing token，保留原失败链。

## 9. 暂停、取消与接管

- `pause automation`：不创建新的 claim；已运行 Attempt 按 Project Policy 继续或安全停点暂停。
- `cancel assignment`：持久化 Intent，发送 fenced cancel，等待真实终止证据。
- `human takeover`：形成 Decision；可以 steer、终止或创建替代 Assignment，但不能改写历史回执。
- `emergency stop`：阻止新命令并尽力终止进程；未确认终止的一律保持 `UNKNOWN`。

## 10. 消费者与审计

Accepted Event 必须可按 `project_id + event_id` 去重并按 producer sequence 检测缺口。审计时间线能从任意 Task 追到 Intent、Decision、Command、ACK、Observation、Receipt、Repository commit、Gate 和后继 Assignment。
