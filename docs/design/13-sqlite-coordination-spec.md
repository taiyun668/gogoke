# SQLite Coordination Store 规格

- 状态：v0.3 A0 authority convergence candidate（待fresh A1审计）
- 目标：把 Assignment claim、lease/fencing、Command、Receipt、outbox 和 reconciliation 落成可迁移、可故障注入的 SQLite 语义
- 边界：不选择具体 Rust crate，不创建持久产品数据库，不把本规格视为 ADR-0009 已接受；机器校验仅创建并删除临时 Fake 数据库

机器可读候选位于 `spec/draft/v1alpha1/sqlite/`，事务参考验证位于 `spec/draft/v1alpha1/tools/validate_sqlite_draft.py`。两者是本规格的可执行审查件，不是生产 migration/runtime。

## 1. 运行约束

SQLite 只位于可靠的本地文件系统。Remote Runner 通过 Runner Protocol 工作，绝不能打开或同步数据库文件。WAL 允许 reader 与 writer 并行，但 SQLite 同时仍只有一个 write transaction；所有权威写入使用短 `BEGIN IMMEDIATE` 事务。候选全集只能通过selection-version keyset API在独立短read transaction中读取，绝不能在writer reservation内扫描、过滤、排名或选择。事务外执行Controller决策及Runner/Git/文件/网络副作用。参见 [SQLite transaction 文档](https://www.sqlite.org/lang_transaction.html)和 [WAL 文档](https://www.sqlite.org/wal.html)。

每个连接必须自检：

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA busy_timeout = <bounded>;
```

`foreign_keys` 是逐连接设置，不能假定默认开启；复合父键必须对应精确 PRIMARY KEY/UNIQUE，所有 Project scope 列 `NOT NULL`。SQLite 官方文档也明确：含 NULL 的复合 child key 不要求父行，且 parent composite key 必须匹配同一 UNIQUE 定义，参见 [SQLite Foreign Keys](https://www.sqlite.org/foreignkeys.html)。

WAL 数据库及 `-wal/-shm` 必须作为同一持久状态管理，不放网络盘、同步盘或拆开复制。备份、checkpoint、integrity check 和迁移在维护流程内执行；发现损坏立即停止自动写入并 reconciliation。

## 2. 表组与主键纪律

所有领域子表使用 `(project_id, local_id)` 主键/唯一键和复合外键，不只依赖 API filter。

| 表 | 关键约束 | 责任 |
|---|---|---|
| `projects` | PK `project_id` | 隔离根、状态和 policy ref |
| `project_sequences` | PK `project_id` | project event sequence，不用 `MAX+1` |
| `project_controller_leases` | PK `project_id`，epoch > 0 | 当前 leader/epoch/expiry/revision |
| `agents` | PK `(project_id, agent_id)` | 持久 AgentInstance |
| `workflow_runs` | PK `(project_id, workflow_run_id)` | 版本化 workflow 实例 |
| `tasks` | PK `(project_id, task_id)` | DAG 节点和 Gate 状态 |
| `task_dependencies` | PK `(project_id, task_id, depends_on_task_id)` | 同 Project 依赖边 |
| `assignments` | PK `(project_id, assignment_id)` | fan-out 调度单位、revision、fence high-water |
| `attempts` | PK `(project_id, attempt_id)`；unique assignment+attempt_no/fence | 不可复用执行尝试 |
| `claims` | PK `(project_id, claim_id)`；每 Assignment 至多一个 ACTIVE | claim 历史 |
| `leases` | PK `(project_id, lease_id)`；每 Assignment 至多一个 ACTIVE | holder/expiry/fence |
| `reclaim_authorizations` | unique assignment+superseded attempt | 停止/隔离/人工风险证明 |
| `commands` | PK `(project_id, command_id)` | fenced Runner Command |
| `idempotency_records` | PK `(project_id, operation_scope, idempotency_key)` | digest 与稳定结果引用 |
| `outbox` | PK `(project_id, outbox_id)`；unique delivery key | 至少一次投递状态 |
| `receipt_records` | PK `(project_id, receipt_id)`；每 Attempt 至多一个 COMMITTED | 验证后的 ReceiptRecord |
| `accepted_events` | PK `(project_id, event_id)`；unique project event_seq/producer seq | 权威审计流 |
| `consumer_offsets` | PK `(project_id, consumer_id, stream_name)` | 持久消费位置 |
| `artifact_descriptors` | PK `(project_id, artifact_id)` | metadata/digest/state/locator，不存 blob |
| `selection_change_batches`（G2目标合同） | PK `(project_id, selection_revision)`；unique digest/operation | append-only predecessor + ≤256 selection old/new deltas + checked total count；单row canonical payload |
| `business_projection_roots`（G2目标合同） | PK `(project_id, business_revision)` | append-only bounded business-axis delta input + ≤64 KiB current summary |
| `execution_authority_roots`（G2目标合同） | PK `(project_id, execution_revision)` | append-only bounded execution-axis delta input + ≤64 KiB current summary |
| `controller_authority_roots`（G2目标合同） | PK `(project_id, controller_revision)` | append-only bounded controller-axis delta input + ≤16 KiB current summary |
| `project_snapshot_compositions`（G2目标合同） | PK `(project_id, composition_revision)` | exact三根refs + selection version/count + predecessor + Core snapshot digest |
| `authority_operation_outcomes`（G2目标合同） | PK `(project_id, operation_kind, operation_key)` | leader/heartbeat/offset/bootstrap等原identity durable result与完整组refs |

所有历史/证据关系默认 `ON DELETE RESTRICT`；禁止用 cascade 清掉审计链。`assignments` 不得对 `(project_id, task_id)` 唯一，因为一个 Task 可以并行存在多个 auditor role slot。可以使用：

```sql
UNIQUE(project_id, task_id, role_slot_key)
```

### 必需 Assignment 字段

```text
state
assignment_revision
fence_high_water
active_attempt_id nullable
ready_at / priority
task_id / agent_id / role_slot_key
reclaim_required
context_snapshot_digest / policy_digest
```

`assignment_revision` 是 CAS 版本；`fence_high_water` 是执行权世代，两者不能合并。Attempt 固化 claim 时的 revision、controller epoch 和 fence。

## 3. 关键索引

```sql
CREATE INDEX assignments_claimable_idx
  ON assignments(project_id, state, reclaim_required, ready_at, priority, assignment_id);

CREATE UNIQUE INDEX active_claim_per_assignment_idx
  ON claims(project_id, assignment_id) WHERE state = 'ACTIVE';

CREATE UNIQUE INDEX active_lease_per_assignment_idx
  ON leases(project_id, assignment_id) WHERE state = 'ACTIVE';

CREATE INDEX active_lease_expiry_idx
  ON leases(project_id, state, expires_at, assignment_id) WHERE state = 'ACTIVE';

CREATE INDEX command_pull_idx
  ON commands(project_id, runner_id, state, expires_at, command_id);

CREATE INDEX outbox_ready_idx
  ON outbox(project_id, state, available_at, outbox_id);

CREATE UNIQUE INDEX one_committed_receipt_per_attempt_idx
  ON receipt_records(project_id, attempt_id) WHERE state = 'COMMITTED';

CREATE UNIQUE INDEX producer_sequence_idx
  ON accepted_events(project_id, producer_id, producer_sequence);

CREATE UNIQUE INDEX project_event_sequence_idx
  ON accepted_events(project_id, event_seq);
```

所有复合 FK child 列另建查询索引。Schema migration 必须执行 `foreign_key_check` 和代表性 DML，不能只验证 `CREATE TABLE` 成功。

## 4. 时间、锁和 retry

一次 write transaction 在开始后只读取一次 SQLite UTC time，并绑定为 `:now`。v1alpha1 统一使用整数毫秒；实现必须通过 SQLite/version probe 和 golden test 证明生成表达式精度，不能混用秒/毫秒，也不能接受 UI/Runner 自报时间判定 lease。

系统记录最近成功 DB time；检测显著回退时暂停 lease reclaim、进入 Attention。测试用相同抽象注入 deterministic clock，但生产不能由 fixture clock 控制。

`SQLITE_BUSY` 只允许在尚未发生外部副作用时，以同一 operation ID/idempotency key 做有界退避。预算耗尽进入 Attention。事务中禁止等待 Runner、Git、Artifact store、filesystem、人工输入或长时间 hash。

## 5. Candidate stream 与 `applyClaimPlan`

### 5.1 Store只返回完整versioned facts

候选API白名单只有`project_id + opaque selection-version token + keyset cursor + page ordinal + page_size(1..=256)`。`SelectionUniverse`包含该Project全部Assignment及dependency/gate/policy/context、runner/host/capacity/locality/credential/budget/isolation/residency/capability、current controller/execution与project/event revision facts；Store不得接收state/ready/eligibility filter、rank、业务limit、owner/winner hint。

每页在独立短`BEGIN DEFERRED`中读取current composition/selection version、按canonical key取下一页、再确认同一version。token绑定version、cursor、ordinal、page chain、page size、base refs、process token epoch和expiry。expiry且same version时只续签；version/epoch变化才从新`Begin`。唯一end witness绑定selection version/count、observed count、最终key/ordinal/page chain、canonical byte count与stream root。Controller/Core收到完整witness后才在事务外决定winner。

### 5.2 Controller生成exact plan，Store只机械CAS

所有ID/digest在事务前固定；`ApplyClaimRequest`携带Core plan、selection algorithm version/digest、完整end witness、selected Assignment/runner/host、selected/relevant exact source refs、base component/composition refs、current Controller epoch/lease和全部next IDs。Store执行：

```text
BEGIN IMMEDIATE
  bind DB now and bounded deadline
  verify current controller lease/epoch + exact composition/selection version
  read original idempotency identity
    same key + same digest -> verify/return prior complete group
    same key + different digest -> CONFLICT
  reread selected row + every declared relevant source ref
  validate exact relation/revision/digest; do not scan/reselect candidates
  execute Assignment CAS and insert Attempt/Claim/Lease/Command
  append history/project/event/Event/outbox rows in frozen order
  build closed selection deltas and append SelectionChangeBatch if nonempty
  build bounded Business/Execution component-delta root rows
  append one ProjectSnapshotComposition
  append durable idempotency outcome binding the entire group
  scope-local readback; COMMIT
```

每个authority domain mutation必须由closed effect map映射到exactly one component axis；selection-bearing subset还必须与selection batch old/new refs逐字节相同。每axis≤256 deltas，三个component inputs + selection batch + plan/manifest canonical bytes合计≤1 MiB；business/execution/controller current summary上限分别64/64/16 KiB。root digest只hash predecessor + causing operation + bounded deltas + bounded summary，不hash全Project物化aggregate。

selection version、selected/relevant ref、CAS或effect-map任一漂移均在COMMIT前零写失败；不得在Store中“找下一个”。CAS零行则整个事务回滚，不能继续创建Attempt。`BEGIN IMMEDIATE`单writer不能替代expected revision/epoch/fence/version条件。

## 6. Heartbeat 与 lease expiry

Heartbeat 只有同时匹配 `project + assignment + attempt + lease + holder runner + current fence + active state`，且 `expires_at > now` 时才能续租。零行是 `STALE_OR_EXPIRED_LEASE`；可保留为 Observation，但不能复活 lease。

到期扫描只执行：

```text
ACTIVE Lease -> EXPIRED
Attempt -> OUTCOME_UNKNOWN
Assignment -> EXPIRED + reclaim_required
append attempt.lease_expired
outbox runner.reconcile.request
```

它绝不直接执行 `Assignment -> READY -> new Attempt`。重新 READY 必须先有 `reclaim_authorization`，证明下列之一：旧 process/session 已停止、已有强制终止证据、或新 Attempt 使用可验证的不交叉写命名空间；也可由审计化人类 Decision 明确承担风险。随后新 claim 提升该 Assignment fence。

### 6.1 Controller、component roots 与 restart

first Controller row只能由typed `BootstrapProjectOperation`在empty-project complete group中创建。正常Store lifecycle封闭为renew、expired takeover、approved step-down；Store不搜索contender、不自动takeover、不删除row重置epoch，已有Project缺Controller row是`CORRUPTION{ControllerRowMissing}`。

bootstrap同一transaction创建initial selection batch、business/execution/controller三个typed genesis root inputs（execution显式`None`）、composition与outcome。此后三根都是append-only predecessor chain；heartbeat推进selection + execution root + composition，leader lifecycle推进selection + controller root + composition，claim等按closed effect map推进受影响axis。每次operation只有一个composition successor。

restart/takeover后在一个短read snapshot内读取current composition、三根current/immediate predecessor、current summaries的exact refs、selection batch与operation outcomes，重算bounded digest后才交给Controller/Core。正常restart不遍历全历史或全Project facts；paged doctor/recovery从genesis重放delta并与materialized rows/selection stream对账。doctor checkpoint只是观测证据，不是current authority，也不由普通writer推进。

## 7. TerminalReceipt commit

正常 commit 事务必须验证：

- receipt ID 重投同 digest 幂等，异 digest 冲突；
- 当前 Project/Assignment/Attempt/Controller epoch/fence/active lease；
- Runner identity/epoch、Command、Session generation/turn fence；
- NativeTerminalEvidence source strength；
- ResultCapsule、Context/Prompt、source revision；
- 所有必需 Artifact 已 `VERIFIED` 且 digest/scope/producer 匹配；
- Runner durable spool proof。

成功事务原子写入 `receipt_records(COMMITTED)`、Attempt `RECEIPT_COMMITTED`、Assignment `RECEIPT_SUBMITTED`、Accepted Event 和 `runner.receipt.committed` outbox。Gate 在后续事务消费 ReceiptRecord，不放进 receipt commit 事务。

若 lease 已过期或当前执行事实不足，Receipt payload/evidence 可以以 `RECONCILIATION_REQUIRED` 保留，但不能成为 COMMITTED。显式 reconciliation 证明没有新 fence 污染且旧写入安全后，才能形成新的 Controller Decision；迟到成功不能覆盖新 Attempt。

## 8. Outbox 与 ACK

Dispatcher 在短事务中租用 outbox row，写入不可复用 `dispatch_token` 后提交；事务外发送同一 Command ID。发送结果只把 row 变为 `AWAITING_ACK`，不等于 Runner 语义 ACK。

Command semantic ACK必须验证project/runner/command/payload/epoch/fence/current Attempt，并使用唯一**15个有序 logical stages**的complete group，顺序固定为：1 `ACK.InsertPlan`；2 `ACK.InsertPlanOperation[0=COMMAND_ACK]`；3 `ACK.InsertPlanOperation[1=OUTBOX_ACK]`；4 `ACK.CasCommandAck`；5 `ACK.CasOutboxAckAndClearDelivery`；6 `ACK.InsertAggregateHistory[0=COMMAND]`；7 `ACK.InsertAggregateHistory[1=OUTBOX]`；8 `ACK.CasProjectRevision`；9 `ACK.CasEventSequence`；10 `ACK.InsertAcceptedEvent`；11 `SEL.InsertChangeBatch`；12 `ACK.InsertBusinessRoot`；13 `ACK.InsertExecutionRoot`；14 `ACK.InsertComposition`；15 `ACK.InsertIdempotencyOutcome`。stage 11不是单一physical row：它展开为1个`selection_change_batches` row、N个按canonical delta order排序的`selection_fact_deltas` rows、以及N个逐项对应且按canonical key order排序的`selection_fact_records` rows。complete-group admission只接受`1 <= N <= 256`；`N=0`和`N=257`（及任何越界值）必须在写入前拒绝。因此actual durable-row manifest恰为`15 + 2*N`行，而不是固定15行。manifest中每个actual row必须记录连续且唯一的row ordinal、所属logical-stage ordinal、table、identity和digest；missing、extra、reordered或duplicate row一律拒绝。它不写mechanical `DeliveryAckOutcome`。Dispatcher发送前后崩溃允许重投同一Command；Runner以`command_id + fencing_token + semantic_digest`幂等，不能创建新Attempt确认旧命令。

其他topic的authenticated delivery ACK是mechanical receipt，只原子写`Outbox ACKED + DeliveryAckOutcome`，不得伪装业务Event。retry exhaustion使用不同的`Outbox FAILED + DeliveryFailureOutcome`，attention只从最终失败事实派生；ACK与FAILED outcome不可复用。

Outbox 状态：

```text
PENDING -> IN_FLIGHT -> AWAITING_ACK -> ACKED
       ^        |             |
       |        +-> PENDING    +-> IN_FLIGHT (ack deadline expired)
       +-----------------------+
any retryable state -> FAILED -> derived Attention
```

过期dispatch lease或ACK deadline可以用新不可复用token重投，但`delivery_key`、业务Command和原identity不变。terminal ACKED/FAILED必须清空token、dispatch expiry与ack deadline。

## 9. COMMIT outcome unknown

Claim、Receipt、ACK 和 outbox 状态都使用同一规则：

1. 事务前固定 operation ID、idempotency key、semantic digest 和下游对象 ID。
2. COMMIT 返回 I/O/连接不确定时，不在原连接盲重试，也不产生外部副作用。
3. 新连接查询原operation outcome/idempotency identity，并交叉检查manifest声明的domain/history/Event/Outbox、selection batch、component root inputs/summaries、composition与outcome完整组。
4. 同 key/digest 且全套行存在：返回原结果；同 key 异 digest：冻结并告警。
5. 完全不存在、old composition/version仍current且能证明未向外投递：才可用相同ID/key/digest重做本地事务。
6. 分类固定为`ABSENT | COMPLETE | PARTIAL_CORRUPTION | INDETERMINATE`；半套、delta/summary/root/composition不对称一律`PARTIAL_CORRUPTION`并停止该Project自动写，不能改标UNKNOWN后盲重放。

SQLite 保证数据库事务原子性，不保证调用方一定收到 COMMIT 成功响应；reconciliation 是正常协议，不是可省略异常分支。

## 10. 禁止实现

- `UNIQUE(project_id, task_id)` 或 Task 级单 lease/fence；
- 全局 ID 主键配不含 project_id 的子表/FK/cache/offset；
- `INSERT OR REPLACE` 破坏历史，或 `OR IGNORE` 吞掉异 digest 冲突；
- select READY 后离开事务，再无 CAS update；
- Store SQL过滤/排名/选择candidate，或在writer transaction全量扫描`SelectionUniverse`；
- 对component root重新hash全Project materialized aggregate，或把delta规范化为未进入fault manifest的隐式子写；
- 公开raw writer connection/transaction/callback，或绕过closed effect map直接改authority row；
- 只按 lease ID heartbeat，或过期 heartbeat 复活 lease；
- lease expiry 定时器直接重派；
- receipt 不检查 epoch/fence/session/digest/artifact；
- PTY/exit 0/assistant 文本直接写 COMMITTED；
- 事务内发网络/Git/Runner，或 send success 当 ACK；
- shared/sync/network directory 中的 WAL 数据库；
- Coordination Store 丢失后从 Git 猜测 in-flight claim 并重派。

## 11. SQLite Gate

必须故障注入：0/1/256/257+ candidate分页、same-version token续签与version漂移、并发N个exact claim plan恰一CAS赢家、closed domain→component/selection effect-map全variant、Command semantic ACK的15个有序logical stages及stage 11的`1 + N + N` actual-row expansion（complete-group admission仅`1 <= N <= 256`；`N=0/257`拒绝；每actual row的ordinal/stage/table/identity/digest以及missing/extra/reordered/duplicate拒绝）、component/selection/composition/outcome每actual write前后、旧fence全写路径、同key异digest、COMMIT response loss、lease/heartbeat/terminal乱序、`SQLITE_BUSY`、含大量inactive facts时writer visited-work不随Project cardinality增长、FK/NULL/cross-Project、artifact未验证、Store/backup恢复后的UNKNOWN。

任何双 claim、双 spawn、双 Receipt、漏 outbox、旧 fence 写入或跨项目命中都是 suite 级 FAIL。无法证明运行/commit outcome 时正确结果是 `UNKNOWN/RECONCILIATION_REQUIRED`，不是自动 retry。
