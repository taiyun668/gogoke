# Fake Runner/Adapter Conformance 与第一条纵向切片

- Suite：`gogo-fake-conformance/v1alpha1`
- 状态：v0.2 实施前设计候选
- 目标：在真实 Harness 和 Dashboard 之前，证明 GOGO 自有协议、状态机、隔离与恢复语义

## 1. 证据边界

Fake Suite 的 PASS 只能标记为 `FAKE_CONFORMANCE_ONLY`。它不验证真实 Codex/Grok 发行物、认证、权限、OS process containment 或协议兼容，不能写成 `Adapter Conformance PASS`、`Production Ready` 或 MVP PASS。

测试运行时：

```text
Deterministic Test Driver
  -> Controller + SQLite Coordination Store
  -> Fake Runner + durable fake spool
  -> Fake Codex Adapter / Fake Grok Adapter
  -> Fake Process Supervisor + virtual clock
  -> isolated Git fixture + content-addressed fake artifact store
  -> State/Authority/SideEffect/Evidence/Isolation Oracles
```

禁止真实 CLI、网络、GitHub、用户 Profile、凭据、wall-clock sleep 和未固定随机数。ID、时钟、输出、事件、文件与故障点全部由 fixture/seed 决定。

## 2. 异词同义 Adapter

两个 fake 必须故意使用不同 native vocabulary 和 cursor 格式，防止内核暗中绑定某一家：

| 能力 | Fake Codex | Fake Grok | GOGO 归一化 |
|---|---|---|---|
| Session | `thread/started` | `session/new` | `session.created` |
| Turn | `turn/started` | `prompt/accepted` | attempt activity Observation |
| 内容/工具 | `item/completed`、usage update | `session/update`、`tool.update` | message/tool/usage Observation |
| 原生终态 | `turn/completed` | `prompt.response.stopReason` | `NativeTerminalEvidence` |
| Context | `thread/compacted` | `session/update.totalTokens` | pressure/compaction Observation |
| 错误 | JSON-RPC error | ACP error/notification | `adapter.error` |

Fake Codex 只有 `turn/completed`，Fake Grok 只有带 stop reason 的 prompt response 可以构成其原生 terminal fence。exit 0、stdout `done`、idle、最后消息和 PTY parser 永远只是低强度 Observation。

## 3. 基础 fixture

`FX-BASE-IAH-001` 包含两个 Project：

```text
Project A
  Implementer Task/Assignment
    -> accepted/materialized Result
  Auditor Task/Assignment
    -> blind review
  Human Gate

Project B
  independent repository/workspace/cache/spool/session/artifact/credential namespace
```

Implementer ResultCapsule 固定 source/base/observed revision、artifact hash、Evidence refs、task/assignment/attempt/fence/context/prompt digest。Auditor 只看到已允许并物化的 Result、revision、Evidence 和自己的 ContextSnapshot；看不到 transcript、未接受 proposal 或其他 Auditor verdict。

`FX-CONTEXT-001` 固定 platform invariant、project policy、role、project-role override、agent override、assignment instruction、fact/evidence refs、Harness glue，并保存 ContextSnapshot、EffectivePromptManifest、ProjectionManifest、LoadProof、source revision、dirty witness 和 Handoff Capsule。

`FX-RUNNER-001` 固定 Runner/controller epoch、各 Assignment 独立 fence、Command idempotency/digest、process side-effect ledger，以及 Project-scoped spool/artifact/session/cursor/credential stub。

## 4. Fault Script

故障由声明式 JSON/YAML 控制，不在测试实现中散落专用分支：

```yaml
seed: fixed-001
faults:
  - id: drop-runner-ack
    when: runner.command_ack.before_send
    action: drop
  - id: late-old-terminal
    when: adapter.native_terminal.after_new_attempt_claimed
    action: {delay_until: attempt-2.process.started}
  - id: db-commit-unknown
    when: controller.receipt_commit.after_transaction
    action: commit_then_lose_response
  - id: output-loss
    when: process.stdout.sequence=17
    action: {truncate_output: {retained_through: 16}}
```

最小 action：`drop`、`duplicate`、`delay_until`、`reorder`、`disconnect/restart/crash`、`timeout`、`advance_clock/expire_lease`、`reject`、`mutate_payload/corrupt_hash`、`commit_then_lose_response`、`emit_stale_epoch/fence/generation`、`truncate_output/fill_spool`、`artifact_unavailable`、`project_reference_swap/workspace_escape`、`inject_second_terminal`。

每个 fault 都记录触发前/后状态和实际 action；配置但未命中的 fault 使测试 FAIL，不能形成伪覆盖率。

## 5. Oracle

1. `StateOracle`：从 Accepted Event 重放 Task/Assignment/Attempt/Gate 状态。
2. `AuthorityOracle`：验证 project、claim、lease、epoch、fence、revision、idempotency；被拒消息不得改变权威状态。
3. `SideEffectOracle`：验证 spawn、Git materialization、artifact finalize、后继 Assignment 至多一次。
4. `EvidenceOracle`：验证 Receipt、Result、Artifact、Projection、LoadProof、source revision 与 digest 链。
5. `IsolationOracle`：扫描数据库查询、cursor、cache、workspace、spool、artifact、native session、credential 的 A/B 边界。

Manual、one-click、run-to-human-gate 对同一 fixture 必须得到等价的权威因果图、状态和 side-effect ledger；只允许 Intent actor、automation limit 和是否自动提交下一 Intent 不同。

## 6. 纵向 Gate

| Gate | PASS | FAIL | INCONCLUSIVE |
|---|---|---|---|
| `G0 ContextReady` | mandatory/manifest/projection/load proof/workspace 全匹配 | missing、conflict、hash/path/precedence/cross-project | 无法提供所需加载证明 |
| `G1 AttemptDispatch` | 唯一 claim/fence，ACK 耐久，process identity 可证 | epoch/fence/revision/capability/context 不符或异 payload | ACK/process outcome 未知，进入 reconciliation |
| `G2 ImplementerResult` | 原生终态、schema-valid Result、verified artifact、Receipt committed | 假终态、digest/revision/scope/双终态冲突 | Runner/DB/commit/evidence outcome 未知 |
| `G3 AuditorReview` | 独立 Snapshot/visibility proof、Receipt/verdict 可证 | reveal 泄露、跨 Project/Agent、Receipt 无效 | 独立性或 Evidence 不可证明 |
| `G4 HumanDecision` | 授权人对精确 Gate snapshot 作审计化 Decision | actor 未授权、过期/错绑定 | Gate input/materialization 未 reconcile |

只有业务 Gate PASS 才推进；FAIL 停止或生成 correction/replan；INCONCLUSIVE 进入 Attention/reconciliation。测试预期系统停在 INCONCLUSIVE 时，测试本身可以 PASS，但报告必须同时展示 `test=PASS, business_gate=INCONCLUSIVE`，不得混为绿色业务成功。

## 7. Test catalog

### 7.1 端到端

- `E2E-001` manual Implementer -> Auditor -> human accept。
- `E2E-002` one-click 与 E2E-001 权威 trace/副作用等价。
- `E2E-003` run-to-gate 在 Human Gate 前停止。
- `E2E-004` human reject 创建 correction READY，不改写旧 Receipt。
- `E2E-005` pause 只阻止新 claim，运行 Attempt 按真实证据收尾。

### 7.2 Adapter 与证据

- `ADP-001/002` Fake Codex/Grok 全生命周期归一化。
- `ADP-003` 异词同义输入形成相同 GOGO contract。
- `ADP-004/005` exit 0、stdout success、idle 不能形成 Receipt。
- `ADP-006/007` 协议/Hook 断开显式降级；PTY/transcript 不覆盖高强度终态。
- `ADP-008` producer seq 同 ID 同 digest 幂等、异 digest protocol violation。
- `ADP-009` 乱序 Observation 不倒退权威状态。
- `ADP-010` 迟到 generation/turn fence terminal 拒绝。

### 7.3 Claim、Runner 与恢复

- `RUN-001` N 个并发 claim 只有一个 Assignment Attempt/fence 获胜。
- `RUN-002/003` 同 command/key/digest 不双启；同 key 异 digest 零新副作用。
- `RUN-004/005` ACK 丢失或已启动但 Controller 未知时只 reconcile，不创建新 Attempt。
- `RUN-006` disconnect/heartbeat loss -> UNKNOWN；lease 到期不自动重派。
- `RUN-007/008` 旧 fence 的所有写和旧 controller epoch command 拒绝。
- `RUN-009` Runner restart 从 spool 重传同一 Receipt。
- `RUN-010` spool full -> DRAINING/Attention，不丢 terminal 后继续 claim。

### 7.4 Receipt 与 commit

- `REC-001` Receipt submit 前存在 durable spool proof。
- `REC-002/003` DB commit 已落盘但响应丢失或 outcome unknown 时查询收敛，不重放副作用。
- `REC-004` 相同双终态幂等，只产生一个 Receipt/Gate。
- `REC-005` 不同双终态进入 CONFLICT/Attention，不 last-write-wins。
- `REC-006` 新 fence 后旧 terminal 不可覆盖。
- `REC-007` terminal 先到但 Result/Artifact 未验证时 G2 不能 PASS。

### 7.5 Artifact、输出和 Git

- `ART-001/002` Artifact 只有 scope/revision/attempt/hash/size 正确且 VERIFIED 才进 Gate。
- `ART-003/004` 输出截断带 sequence/loss evidence；大输出不丢 terminal boundary，否则 INCONCLUSIVE。
- `ART-005` Git commit-then-lose-response 按 manifest/object reconcile，不重复物化。
- `ART-006` 必需 Evidence 不可读后显示 `EVIDENCE_UNAVAILABLE`，不维持无条件 PASS。

### 7.6 Context 与轮换

- `CTX-001/002` overlay/hash 稳定，mandatory 不可被 override 弱化。
- `CTX-003/004` missing/conflict/drift fail-closed；write success 无 LoadProof 为 INCONCLUSIVE。
- `CTX-005` precedence/path/symlink escape/用户文件碰撞 FAIL。
- `CTX-006` 运行中重大 revision/policy/instruction drift -> BLOCKED/Attention。
- `CTX-007/008` 只在 Receipt committed、无副作用、revision 固定、后继未 claim 的安全点轮换；写入/物化/权限/未提交 Receipt 时延迟。
- `CTX-009` continuity handshake scope/revision/prompt/workspace 错误 FAIL。
- `CTX-010` token/idle 单信号最多建议，不自动硬轮换。

### 7.7 Project 与盲审隔离

- `ISO-001` Project B 引用 A 的 Command/Receipt/Artifact/Session/Context 全拒绝。
- `ISO-002/003` cursor/cache/spool/workspace/artifact/credential collision 负测。
- `ISO-004/005` Reveal Gate 前不可读未许可材料/他人 verdict；visibility proof 缺失为 INCONCLUSIVE。
- `ISO-006` 旧 Attempt 不能写新 Attempt namespace。

## 8. 最小完成定义

Fake 纵向闭环完成需要：

1. `E2E-001..003` 通过且三模式 authority trace 等价。
2. `ADP-001..010` 证明 native 类型未越过 Adapter 边界。
3. `RUN-001..010`、`REC-001..007`、`ART-001..006`、`CTX-001..010`、`ISO-001..006` 全部有确定结果和 fault hit proof。
4. 任一双 claim/spawn/materialization/Receipt、旧 fence 写入或跨项目读取立即 suite FAIL。
5. G0-G4 每个 PASS 可反查精确 Context、Receipt、Artifact、Decision 和 Accepted Event。
6. 报告显式包含 `FAKE_CONFORMANCE_ONLY`，并列出真实 Codex/Grok 尚待执行的同名 test IDs。

当前已生成 machine-readable schemas、fixtures、54 条测试库存，并由确定性内存参考内核执行全部 54 条、命中全部 42 个已声明 fault；详见 `17-fake-kernel-executable-subset-review.md`。但只有 9 个高风险 bridge case 进入 SQLite 参考事务，durable spool 与 isolated Git fixture 仍是模拟语义，因此当前仍不是完整 Fake Conformance PASS。下一阶段必须把整条 trace 接入共享 SQLite/Runner/Git fake 拓扑；完整 Dashboard 与真实 Harness 接入仍在其后。
