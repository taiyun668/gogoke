# Schema-first 协议规格

- 状态：v0.2 实施前设计候选
- 目标：在选择具体依赖和编写生产代码前，冻结跨 Control Plane、Runner、Adapter、Repository 与测试夹具的中立对象边界
- 前置：ADR-0009 仍为 `Proposed`；本文件不构成实现或源码复制授权

## 1. Schema 基线

v1alpha1 使用 JSON Schema Draft 2020-12。每个顶层 schema 声明固定 `$id`、`$schema`、`schema_version` 和 `kind`；组合对象使用 `unevaluatedProperties: false`，不能靠忽略未知权威字段获得“兼容”。`format` 只能作为注解时，生成的校验器仍须显式执行 date-time、URI、digest 和 ID 语义校验。

规范依据：JSON Schema 2020-12 引入动态引用、bundling，并把 `format` 分为 annotation/assertion vocabulary；实现必须固定所用 vocabulary，而不是只写一个版本字符串。参见 [JSON Schema Draft 2020-12](https://json-schema.org/draft/2020-12)。

机器可读候选已经生成在 `spec/draft/v1alpha1/`。它仍受本规范和 Proposed ADR 约束，只是可审查草案，不是已发布协议：

```text
spec/draft/v1alpha1/
  schemas/
  defs/{identifier,scope,envelope,digest,actor,artifact-ref,problem,extension}.schema.json
  commands/{intent-command,runner-command}.schema.json
  adapter/{operation,event,native-terminal-evidence}.schema.json
  receipt/{terminal-receipt,receipt-record}.schema.json
  context/{context-pack,projection-manifest,load-proof}.schema.json
  result/result-capsule.schema.json
  wire-message.schema.json
  bundle.schema.json
  schema-tests/{cases,manifest.json,digest-vectors.json}
  fake/ + fixtures/
```

每个 per-kind 文件验证一个不可变语义对象；跨进程时必须作为 `WireMessage.payload` 投递。`bundle.schema.json` 用封闭 `oneOf` 联合所有合法语义对象与 `WireMessage`。持久化入口、Repository Reactor 和 Conformance 使用同一 bundle，不维护 UI 专用宽松版本。

## 2. 标量与身份规则

### 2.1 GOGO ID

内部生成 ID 是带类型前缀的 128-bit 不透明标识：

```text
<prefix>_<32 lowercase hex>
```

例如 `prj_...`、`tsk_...`、`asg_...`、`att_...`、`cmd_...`、`rcp_...`。前缀用于诊断，不能代替 `kind` 或外键校验；ID 不从 cwd、Git remote、窗口标题、PID 或 Harness session 推导。外部 native ID 只能进入 `NativeRef`，不得冒充 GOGO ID。

### 2.2 计数器

`revision`、`controller_epoch`、`runner_epoch`、`fencing_token`、producer/event sequence 和 `size_bytes` 在 JSON 中统一为无前导零的十进制字符串，在 SQLite 中使用经过范围检查的整数。这样避免 JavaScript/不同 JSON runtime 对大于 2^53 的整数产生精度分叉。

`revision` 是资源乐观并发版本；`controller_epoch` 是逻辑 leader 世代；`runner_epoch` 是 Runner 本机恢复世代；`fencing_token` 是 Assignment 执行权世代。四者不得比较或互相替代。

### 2.3 时间与摘要

业务时间使用带 `Z` 的 RFC3339 UTC 字符串；lease 数据库字段另存统一单位的整数时间。未知时间不能写当前时间冒充观察时间。

所有 digest 使用 `sha256:<64 lowercase hex>`。JSON 摘要采用 RFC 8785 JCS 的 I-JSON 约束、确定性属性排序和 primitive serialization；重复属性、NaN/Infinity、不能安全表达的 JSON 数字和无效 Unicode 必须拒绝。参见 [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)。

## 3. 语义对象、WireMessage 与投递元数据

per-kind 语义对象自身带 `schema_version + kind + semantic_digest`；其摘要是移除自身 `semantic_digest` 后的完整对象 JCS。跨进程边界时再进入统一的 `WireMessage`，外层结构如下：

```yaml
schema_version: gogo/wire-message/v1alpha1
kind: WireMessage
message_id: msg_...
producer:
  actor_type: controller | runner | adapter | human | agent | reactor
  actor_id: ...
  runner_epoch: "7"       # 仅 Runner/Adapter 来源
producer_sequence: "42"  # 事件/观察必填
occurred_at: 2026-08-11T12:00:00Z
correlation_id: cor_...
causation_id: msg_...     # 根 Intent 可省略
scope: {...}
payload: {...}
semantic_digest: sha256:...
extensions: {...}         # 可省略
```

WireMessage 的 `semantic_digest` 对以下 JCS 投影计算：

```text
{schema_version, kind, producer, producer_sequence?, occurred_at, scope, payload}
```

它不包含 `message_id`、`semantic_digest` 自身，也不包含接收方生成的 `received_at`、transport attempt、network nonce/signature、SSE cursor 或本地数据库行号。同一消息重投必须保留相同语义字段与 digest；同一 idempotency key 对应不同 digest 是安全冲突。

Schema 只能验证形状，不能证明跨对象关系。接收方必须另验外层/内层 Project 一致、scope/fence 一致、引用对象 scope/digest 一致、producer sequence 幂等以及当前 generation/turn/fence。Host 级 `DISCOVER/PROBE` 不得携带 Assignment 权威；其余 Adapter 状态操作必须完整绑定 Assignment scope 与 fence。

`scope` 至少包含 `project_id`。Assignment/Attempt 操作必须包含 `workflow_run_id + task_id + assignment_id + attempt_id + agent_id + runner_id`；已创建 Session 后相关事件再带 `session_id`。`AssignmentFence` 固定为 `assignment_id + attempt_id + fencing_token + controller_epoch`，不能用 lease ID 替代。

## 4. 顶层对象

| 对象 | 生产者 | 必需语义 | 明确禁止 |
|---|---|---|---|
| `IntentCommand` | human/UI/CLI/Chief of Staff | intent、actor、目标 scope、idempotency、可选 expected revision、automation limit | Controller fence、任意 shell、Runner 参数 |
| `RunnerCommand` | Controller | command、完整 Attempt scope、expected revision、AssignmentFence、deadline/expiry、policy/capability/context digest | vendor session/type、凭据内容 |
| `AdapterOperation` | Runner | operation、adapter/version、deadline；状态操作绑定 command/idempotency/fence | Adapter 自选 retry、policy 或 project crawl |
| `AdapterEvent` | Adapter | source strength、native ref、generation/turn fence、native/adapter sequence、normalized data | 直接推进 Task、完整 raw transcript/reasoning |
| `NativeTerminalEvidence` | Adapter | current session generation、turn fence、terminal basis/status/reason、evidence refs | GOGO Receipt、PTY/idle 伪装 authoritative success |
| `TerminalReceipt` | Runner | current scope/fence、command、Runner epoch、terminal evidence/result/artifact/revision/context digest、spool proof | Gate verdict、Accepted Fact 声明 |
| `ReceiptRecord` | Controller | receipt ref/digest、`COMMITTED \| REJECTED \| CONFLICT \| RECONCILIATION_REQUIRED`、验证 evidence、commit epoch/time | 倒造 TerminalReceipt、包含 Gate 结论 |
| `ContextPack` | Controller | purpose、Snapshot/Prompt/Policy/revision refs、ordered fragments、included/omitted/conflict、compiled digest | transcript、secret、隐式 overlay |
| `ProjectionManifest` | Adapter Projector | pack/projector/harness/workspace ref、target hash、included/omitted/conflict、rollback ref | 未解释 omission、绝对逃逸路径、覆盖用户文件 |
| `LoadProof` | Adapter Projector/doctor | projection digest、method、strength、observed hash/time、outcome | 用 write success 冒充 loaded |
| `ResultCapsule` | Agent，经 Adapter/Runner 收集 | producer/attempt/context/revision、verdict/summary、artifact/evidence、required changes、unresolved/boundary/next proposal | 自称 Gate PASS/ACCEPTED 或直接启动后继 |

`NativeTerminalEvidence -> TerminalReceipt -> ReceiptRecord -> GateEvaluation` 是四个独立对象。只有 Runner 组装 TerminalReceipt；只有 Controller 提交 ReceiptRecord；Gate 只消费已提交记录。

## 5. Context 对象细则

`ContextPack.fragments[]` 固定记录 layer、ordinal、required、merge key/mode、source revision 和 content hash。合并模式只有 `append | replace | set_union | exclusive`；同 precedence 的 exclusive 冲突、未声明 merge 或 mandatory 被删除时 compilation 失败。

`ProjectionManifest.targets[]` 记录 `protocol_field | relative_path`、delivery mode、content hash 和 disposition。Harness 会原生自动加载的文件不得重复注入，但必须记录 `native_auto_load_excluded` 和验证方法。

`LoadProof.outcome=LOADED` 时必须绑定 `pack digest + projection digest + adapter/harness/projector version + target + observed hash`。`UNKNOWN | NOT_LOADED | STALE` 不能满足强制加载 Gate。Windows copy 是首个必测路径；junction/symlink 是独立能力，不是默认前提。

## 6. Extension 与 vendor 边界

唯一开放字段为显式 `extensions`。键使用反向域名命名空间，值包含 `schema_uri + schema_digest + data`。未知 extension 可以保留或展示，但 Controller、Policy、Gate、Scheduler 和事实提升不得依据未协商 extension 改变权威状态。

Vendor raw JSON、stdout、reasoning、认证/限额信息、完整 transcript 和用户级配置只可成为 Host-local 受限 artifact。领域对象只保存脱敏 normalized data、`NativeRef`、cursor/time、raw artifact digest 和证据强度。

版本规则：不支持 major/schema family 立即拒绝；alpha/minor feature 必须显式协商。不能通过“忽略未知字段”猜测兼容。

## 7. 错误对象

`Problem` 至少包含：

```text
code
retryability = SAFE | RECONCILE_FIRST | NEVER | UNKNOWN
safe_summary
evidence_refs[]
```

核心 code 合并为：`PRECONDITION_FAILED`、`POLICY_DENIED`、`DISPATCH_REJECTED`、`PROCESS_START_FAILED`、`RUNTIME_LOST`、`COMMAND_OUTCOME_UNKNOWN`、`RECEIPT_INVALID`、`RECEIPT_UNAVAILABLE`、`COMMIT_INDETERMINATE`、`GATE_INCONCLUSIVE`、`NOT_INSTALLED`、`VERSION_UNSUPPORTED`、`AUTH_NOT_READY`、`CAPABILITY_MISSING`、`CONTEXT_PROJECTION_FAILED`、`SESSION_CREATE_FAILED`、`PROTOCOL_VIOLATION`、`CANCEL_UNCONFIRMED`、`ADAPTER_BUG`。

UNKNOWN 或 outcome unknown 默认 `RECONCILE_FIRST`；权限拒绝、旧 fence、异 digest 默认 `NEVER`。错误对象不得携带 secret、完整环境变量或隐藏 reasoning。

## 8. Schema 不能表达的跨对象不变量

1. 所有关联对象和 artifact/workspace/session/cursor 必须属于同一 Project。
2. AssignmentFence 必须等于当前 claim；旧 fence 不能 heartbeat、写 artifact、提交 receipt 或完成。
3. 一个 Attempt 只接受一个 canonical TerminalReceipt digest；同 digest 幂等，异 digest 冲突。
4. Native terminal generation/turn fence 必须与当前 Session 关联一致。
5. Receipt 引用的 ResultCapsule、Context、revision、artifact/evidence digest 必须完全匹配不可变对象。
6. 一个 Task 可以 fan-out 多个 Assignment；fence 不能提升到 Task 级。
7. Generic PTY 没有结构化 terminal fence 时不能产生自动成功 Receipt。
8. Context mandatory/load proof 不满足时 start/send fail-closed。

这些不变量由 Coordination Store、Transition Engine 和 Conformance 联合验证，不能写进 Prompt 代替。

## 9. Schema Gate

生成真实 schema 前必须先固定：

- JCS/SHA-256 跨 Rust/TypeScript golden vectors，包括 key reorder、Unicode、极值与非法数字；
- 每种顶层对象至少一个 golden fixture；
- 跨 Project ref、空/错误 scope、旧 fence、同 key 异 digest、双终态、raw vendor 泄露、unknown field、major mismatch 等负测；
- fake Codex/Grok 使用不同 native vocabulary，但输出同一中立 contract；
- bundle 对全部 fixture 严格验证，Schema 校验器差异不得被忽略。

Fake Schema PASS 只证明协议形状和不变量测试通过，不证明真实 Harness Conformance。
