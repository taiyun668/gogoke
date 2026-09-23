# GOGO PARTY 参考负向测试矩阵

- 日期：2026-08-20
- 性质：设计输入。每条场景都是 `REFERENCE_REQUIREMENT`，实现状态都是 `NOT_IMPLEMENTED`。
- 来源：Hermes Studio 参考车道与 DeepSeek Harness 参考车道。
- 明确不是：可执行 Fake 套件、Adapter Conformance PASS、上游回归、生产准入。

本矩阵把上游产品缝翻译成 GOGO PARTY 必须自行守住的负向不变量。它**不是**对 `EKKOLearnAI/hermes-studio@d6bed4cc550b8e887e69389d00840ac97407fdd1` 或 `deepseek-ai/deepseek-harness@141eb6fef83422698aef7a981029e843e8161534` 当前失败的指控。上游测试文件只是 `TEST-SOURCE` / `NOT-RUN`。

禁止：把下列 ID 写进可执行 Fake conformance、宣称测试已通过、或用本表证明 `PRODUCT-PATTERN` / `SEMANTIC-PORT` 已准入。

## 1. 状态词汇

| 标记 | 含义 |
|---|---|
| `REFERENCE_REQUIREMENT` | GOGO PARTY 需要这条负向不变量 |
| `NOT_IMPLEMENTED` | 本仓库尚未实现对应测试或产品路径 |
| `NOT-RUN` | 本轮未执行上游测试、Fake 套件或任何新测试 |
| `TEST-SOURCE` | 上游存在测试 seam，但未在本机执行 |

## 2. Hermes Studio 参考场景

固定上游：`EKKOLearnAI/hermes-studio` commit `d6bed4cc550b8e887e69389d00840ac97407fdd1`。产品壳为 `PRODUCT-PATTERN`，coding-agent 传输思想为 `SEMANTIC-PORT`，商业源码复制与 runtime 直连为 `REJECT`。

| ID | 场景 | GOGO PARTY 权威对象 | 负向不变量 | 状态 |
|---|---|---|---|---|
| HS-01 | 底层失败而审批 UI 显示成功 | `GateEvaluation`、`ReceiptRecord`、Attention Inbox | 无已提交 `ReceiptRecord` 时 UI 不得投影成功；run 失败或未知不得被审批卡片盖成 PASS | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| HS-02 | 审批超时后迟到 ACK | fencing token、`controller_epoch`、approval generation、Command 幂等 | 超时消耗该 generation；迟到 ACK 必须拒绝；不得把已关闭 Attempt 改为成功 | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| HS-03 | 任务完成无法解析 origin Session | `Attempt`、`Session`、native session ref、`TerminalReceipt` | origin Session 不可解则不得 `SUCCEEDED`，不得组装 Receipt，不得进 Gate；记 `OUTCOME_UNKNOWN` | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| HS-04 | 两套 Session 存储分叉 | Coordination Plane Session 身份、Native Session Plane 投影 | 只允许一个权威 Session 身份；分叉先 reconciliation，UI 不得选边当事实 | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| HS-05 | 重启丢失调度器所有权 | scheduler lease、`controller_epoch`、fencing、WorkflowRun | 无有效 lease 不得派活；重启后先续租/接管；双主零写拒绝 | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| HS-06 | transcript 推断路由到错误收件人 | Party/Agent 引用、Assignment、`LoadProof`、mention 路由 | 只接受结构化收件人；transcript/`@名字` 不得创建错误 Assignment，不得写入 LoadProof | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |

## 3. DeepSeek Harness 参考场景

固定上游：`deepseek-ai/deepseek-harness` commit `141eb6fef83422698aef7a981029e843e8161534`（`dsh@0.1.0-rc.8`）。DSH 只当 Harness/Adapter 目标。DSH 序号只进 `AdapterEvent.nativeSeq`；subagent finish 只进 `NativeTerminalEvidence`。

| ID | 场景 | GOGO PARTY 权威对象 | 负向不变量 | 状态 |
|---|---|---|---|---|
| DSH-01 | 插件卸载与 in-flight 所有权 | Runner 所有权、spool、Attempt、`RUNTIME_LOST` | effect 撤回不得抹掉 in-flight 工作；必须 drain、移交或标丢失；无主工作不得算成功取消 | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| DSH-02 | 重复或回退 SessionEvent | `AdapterEvent.nativeSeq`、Observation 幂等、GOGO PARTY sequence | 重复/回退/replay 不得写第二条权威事件，不得回退账本序号；GOGO PARTY sequence 不采用 DSH seq | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| DSH-03 | 过期 subagent 谱系 | `NativeTerminalEvidence`、Attempt generation/fence | 旧 child finish 不得成为当前 Attempt 的 `TerminalReceipt` / `ReceiptRecord` / Gate | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| DSH-04 | API 投影丢失 scope | `project_id`、session/attempt 绑定、Coordination Store | 缺 scope 的 Typert/API 或 JSON-RPC 投影必须拒绝，不得入账本、spool 或 Context | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| DSH-05 | workflow worker / host 重启 | Transition Engine、durable ledger、lease/fencing、Runner spool | DSH goal/plan/schedule/worker 不是耐久权威；重启后只报 `UNKNOWN`，禁止无新 Command 续跑 | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| DSH-06 | Windows ACL 不支持或不完整隔离 | `CapabilityEvidence`、Project 隔离、execution profile | partial/unsupported/unknown ACL 不得满足强隔离 Assignment，不得宣传完整沙箱 | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |
| DSH-07 | 源码 pin 与 npm 包版本漂移 | Adapter 版本证据、`verified_version`、调度准入 | 若实际部署解析 `latest=0.1.0-rc.7` 或版本未知，不得套用 commit/`dsh@0.1.0-rc.8` 证据；只有显式解析并验证 `rc.8` 后才继续该版本 Gate | `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED` |

## 4. 权威层对照（防止测错对象）

若未来实现这些场景，断言必须打在 GOGO PARTY 对象上，而不是上游内部状态。

| 错误测法 | 正确测法 |
|---|---|
| 断言 Hermes Studio 审批 API 返回成功/失败 | 断言 GOGO PARTY UI 与 `GateEvaluation` 一致，且无 `ReceiptRecord` 不能成功 |
| 断言 DSH SessionEvent 连续 | 断言 `AdapterEvent.nativeSeq` 可重复/可回退时 GOGO PARTY 账本仍单调且幂等 |
| 把 subagent 结束当任务完成 | 只产生 `NativeTerminalEvidence`；无 spool+Controller 校验则无 Receipt/Gate |
| 把 DSH worker 重启当 Control Plane 恢复 | Runner `UNKNOWN` + Controller reconciliation；Transition Engine 不自动前移 |
| 把 Windows ACL README 当隔离证明 | `CapabilityEvidence` 为 partial；强隔离需求 fail-closed |
| 执行上游 `tests/e2e` 或 DSH testing 套件并宣称 GOGO PARTY PASS | 上游保持 `NOT-RUN`；GOGO PARTY 场景保持 `NOT_IMPLEMENTED` |

不得被这些场景替换的对象：GOGO PARTY 耐久账本、lease/fencing、Runner spool、`TerminalReceipt` / `ReceiptRecord` / Gate、Project 隔离、Context `LoadProof`、Transition Engine。

## 5. 与可执行 Fake 套件的边界

现有 `gogo-fake-conformance/v1alpha1` 与本矩阵隔离：

- 本文件不增加、不修改、不引用为已实现的 Fake fixture。
- Fake Suite 的 PASS 仍只能是 `FAKE_CONFORMANCE_ONLY`，不能因为本矩阵存在就扩张其范围。
- HS-* / DSH-* 在有独立设计与实现计划前，不得改名成 Fake ID。
- 本轮未运行 Fake 套件。`NOT_IMPLEMENTED` 与 `NOT-RUN` 同时成立。

## 6. NOT-RUN 清单

- Hermes Studio：`tests/client`、`tests/server`、`tests/desktop`、`tests/e2e` 均为 `TEST-SOURCE` / `NOT-RUN`。
- DeepSeek Harness：`docs/testing.md` 及任何 package 测试均为 `TEST-SOURCE` 定位 / `NOT-RUN`。
- 未执行 `npm test`、`pnpm test`、`cargo test`、上游构建或本仓库 Fake conformance。
- 未安装上游依赖，未加入 GOGO PARTY 依赖。
- 本工作区未落地这两份上游副本，也未运行 shell 命令核验 npm registry（npm 漂移采用控制器 2026-08-20 证据）。

## 7. 残留风险

1. Hermes Studio 实现细节可能比核验锚点更复杂；双 Session 模块是参考缝，不是已复现的生产事故。
2. DSH 本轮几乎是 `DOC` 边界；Windows ACL、插件卸载和 API 投影的真实强度需要以后的实现级审计。
3. DSH 为 developer preview，pin 之后仍可能不兼容；DSH-07 在包漂移期间持续有效。
4. BSL 1.1 使 Hermes Studio 源码在无商业许可时保持 `REJECT`；产品参考不得滑向复制。
5. 在 Fake/Conformance 真正落地前，本矩阵不能减少 P05/CT3 或其他生产权威工作的范围。
