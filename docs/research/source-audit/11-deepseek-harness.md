# deepseek-ai/deepseek-harness 参考源码审计

- 审计日期：2026-08-20
- 仓库：`deepseek-ai/deepseek-harness`
- 上游：[官方介绍](https://deepseek.com/harness/)；[GitHub 仓库](https://github.com/deepseek-ai/deepseek-harness)；[固定 commit](https://github.com/deepseek-ai/deepseek-harness/commit/141eb6fef83422698aef7a981029e843e8161534)
- 固定源码 commit：`141eb6fef83422698aef7a981029e843e8161534`（全文证据边界）
- 该 commit 的发行标注：`dsh@0.1.0-rc.8`
- 许可证：MIT
- 官方定位：developer preview；官方站点与仓库均警告兼容性破坏变更
- npm 证据（2026-08-20，控制器提供）：`latest=0.1.0-rc.7`，`next=0.1.0-rc.8`
- 结论性质：GOGO PARTY 参考-only 设计输入。DSH 是候选 **Harness 执行层 / Adapter 目标**，永远不是 GOGO PARTY Control Plane。`SEMANTIC-PORT` 不是生产准入。

本轮未复制源码、未加入依赖、未把 DSH 接入生产权威、未执行上游测试，也未改动可执行 Fake 套件。下列负向场景一律 `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`，不是对上游当前失败的指控。

## 1. 角色边界（硬）

```text
GOGO PARTY Control Plane
  Project / Party / Workflow / Gate / Policy
  durable ledger / lease / fencing / spool
  TerminalReceipt → ReceiptRecord → Gate
  Context LoadProof / Transition Engine
          │
          │ fenced Runner Command
          ▼
     Host Runner
          │
          ▼
  Harness Adapter Kernel
          │
          ▼
  DeepSeek Harness（候选执行层或 Adapter 目标）
```

允许：

- 把 DSH 当作一个未来 Adapter 的 native runtime / SDK / stdio JSON-RPC 对端。
- 把 DSH `SessionEvent` 序号写入 `AdapterEvent.nativeSeq`。
- 把 subagent 结束写入 `NativeTerminalEvidence`。
- 把 Cordis 可逆插件、resume/fork/replay、测试缝和 Windows ACL 能力作为语义与能力证据。

禁止：

- DSH 本地 job / goal / plan / schedule / worker 替换 GOGO PARTY 耐久账本。
- DSH 插件容器替换 lease、fencing、spool。
- DSH 终态替换 `TerminalReceipt` / `ReceiptRecord` / Gate。
- DSH workspace 替换 Project 隔离。
- DSH 上下文替换 `ContextSnapshot` / `LoadProof`。
- DSH workflow worker 替换 Transition Engine。
- 把 Windows ACL 包宣传成完整 filesystem/network/credential 隔离。

## 2. 许可、预览与版本漂移

| 项 | 固定事实 |
|---|---|
| `LICENSE` | MIT |
| 源码 pin | `141eb6fef83422698aef7a981029e843e8161534` |
| 发行标注 | `dsh@0.1.0-rc.8` |
| npm `latest`（2026-08-20） | `0.1.0-rc.7` |
| npm `next`（2026-08-20） | `0.1.0-rc.8` |
| 官方口径 | developer preview；可能不兼容升级 |

MIT 只解决许可阅读和未来派生的法律入口，不解决预览稳定性和包漂移。源码 pin 指向 `rc.8`，npm `latest` 仍是 `rc.7`、`next` 才是 `rc.8`；这说明不能把 dist-tag 当作已审计版本。负向场景 **DSH-07** 要求记录并验证实际解析的精确包版本：若部署仍解析到 `latest=rc.7`，或解析版本未知，就不能使用 `rc.8` 源码证据准入；显式解析并验证 `rc.8` 后才可继续该版本的 Adapter Gate。

在依赖、NOTICE、平台、测试和兼容性闭包完成前，DSH 不得升级为 `SOURCE-DERIVED`。本轮最高只到 Adapter 目标的 `SEMANTIC-PORT`；小型测试工具也仍不是生产准入。

## 3. 证据边界与等级

本轮锚点以官方文档和 package README 为主，没有把 DSH 实现 `.ts` 列入控制器核验清单。因此架构能力和上游自我定位多为 `DOC`；许可证文本是许可边界的第一方 `SOURCE`。未运行测试。

| 锚点 | 等级 | 用途 |
|---|---|---|
| `README.md` | `DOC` | 预览定位、包入口、破坏性变更警告 |
| `LICENSE` | `SOURCE` | MIT |
| `docs/architecture.md` | `DOC` | Cordis 插件/effect、整体分层 |
| `docs/subsystems/session.md` | `DOC` | SessionEvent / trajectory / resume / fork / replay |
| `docs/subsystems/subagent.md` | `DOC` | subagent 谱系与 provider |
| `docs/api-gateway.md` | `DOC` | Typert/API 投影与网关 scope |
| `docs/testing.md` | `DOC` + `TEST-SOURCE` 定位 | 测试支持模式；`NOT-RUN` |
| `packages/subagent/subagent-codex/README.md` | `DOC` | Codex subagent provider |
| `packages/subagent/subagent-claude-code/README.md` | `DOC` | Claude Code subagent provider |
| `packages/subagent/subagent-acp/README.md` | `DOC` | ACP subagent provider |
| `packages/workflow/workflow-worker-thread/README.md` | `DOC` | worker-thread 工作流执行 |
| `packages/sandbox/sandbox-windows-acl/README.md` | `DOC` | Windows ACL 沙箱能力声明 |
| `packages/sdk/server/README.md` | `DOC` | DSH SDK server 表面 |
| `python/sdk-runtime/README.md` | `DOC` | Python SDK runtime 表面 |

`DOC` 不能证明实现已完成。Windows ACL、worker 隔离和 API 投影在未读实现与未跑测试前，只作为能力候选和负向需求来源。

## 4. 映射到 GOGO PARTY 对象

| DSH 概念 | 只允许进入 | 禁止进入 |
|---|---|---|
| SessionEvent / trajectory 序号 | `AdapterEvent.nativeSeq`、原生 cursor artifact | GOGO PARTY 账本 sequence、outbox offset、Controller epoch |
| resume / fork / replay | Adapter capability：`session.resumed` 等 Observation | 直接推进 Task/Assignment/Attempt 权威状态 |
| subagent finish | `NativeTerminalEvidence` | `TerminalReceipt`、`ReceiptRecord`、Gate PASS/FAIL |
| Cordis plugin/effect | Adapter 进程内资源生命周期思想 | Control Plane 插件热卸载权威 |
| Typert/API、stdio JSON-RPC | Adapter transport / probe 表面 | GOGO PARTY Runner Protocol 或 Coordination API |
| goal / plan / schedule / workflow worker | Harness 内部执行计划 | durable ledger、lease/fencing、Transition Engine |
| Windows ACL sandbox | 按 Host/profile 的 `CapabilityEvidence`，且标 partial | Project 隔离证明、PREVENTIVE sandbox 宣传 |
| SDK server / Python runtime | 未来 Adapter 集成模式 | 第二套 Control Plane |

Adapter 原生终态、Runner `TerminalReceipt`、Controller `ReceiptRecord` 和 `GateEvaluation` 仍是四层，不得因 DSH 有 SessionEvent 或 subagent 结束而合并。

## 5. 可提取语义（不是可复制内核）

### 5.1 Cordis 可逆插件 / effect

`docs/architecture.md` 把 DSH 描述为 Cordis 插件容器：插件注册服务与可逆 effect，卸载时应撤回 effect。对 Adapter 宿主有参考价值：加载进去的传输、沙箱和 subagent provider 必须能声明资源所有权。

这不是 GOGO PARTY 插件系统。Control Plane 不按 DSH 插件卸载语义推进状态。负向需求 **DSH-01**：插件卸载时若仍有 in-flight Session/subagent/worker，所有权必须显式转移或 fail-closed，不能让效果回滚把进行中的 Attempt 变成“从未发生”。

### 5.2 append-only SessionEvent / trajectory

`docs/subsystems/session.md` 描述 append-only SessionEvent 与 trajectory，并支持 resume / fork / replay。这对应 Adapter 侧原生游标，而不是 GOGO PARTY event-store。

映射规则：DSH 序号只进 `AdapterEvent.nativeSeq`。重复、回退或 replay 产生的事件不得生成第二条 GOGO PARTY Observation 权威行，除非带新的 correlation 且标明 replay。负向需求 **DSH-02**。

resume/fork/replay 可作为 Adapter capability 词汇（`SUPPORTED | DEGRADED | UNSUPPORTED | UNKNOWN`），证据过期或预览版漂移后撤销。

### 5.3 subagent providers

package README 给出四类 provider：Codex、Claude Code、ACP、DSH SDK。DSH 可以把这些当作自己的子执行器；GOGO PARTY 仍只通过 Adapter Kernel 看见它们。

规则：

- DSH 调用 Codex/Claude Code/ACP，对 GOGO PARTY 仍是该 DSH Adapter 的内部 native 细节。
- subagent 谱系可以出现在 Native Session Plane 的引用里，不能成为 Party/AgentInstance 身份。
- subagent finish 只进 `NativeTerminalEvidence`。没有 Runner spool 与 Controller 校验前，不得有 Receipt。
- 负向需求 **DSH-03**：fork/resume/replay 之后若 parent/child 谱系过期，不得把旧 subagent 终态算成当前 Attempt。

不得把 DSH 的“能驱动 Codex 和 Claude Code”写成 GOGO PARTY 已有 Codex Adapter 或 Claude Adapter。那些官方协议仍以各自上游为准。

### 5.4 Typert/API 与 stdio JSON-RPC

`docs/api-gateway.md` 与 SDK README 给出类型化 HTTP 表面和 stdio JSON-RPC。适合作为未来 DSH Adapter 的 probe/transport 候选，类似 ACP 或 app-server 对端。

负向需求 **DSH-04**：API 投影若丢掉 project/session/attempt scope，响应必须拒绝，不能把无 scope 的对象送进 Coordination Plane。stdio JSON-RPC 也不得成为隐藏控制通道；只执行已注册 Adapter 方法。

### 5.5 goal / plan / schedule / workflow 组合

DSH 把 goal、plan、schedule 和 `workflow-worker-thread` 组合成 Harness 内部工作流。这对“执行层可以有自己的计划循环”有参考价值，但层次不能上移。

GOGO PARTY 的 WorkflowDefinition、WorkflowRun、Task、Assignment、Transition Engine 仍是 Control Plane 对象。DSH worker 重启、线程退出或 host 崩溃，只造成 Adapter/Runner `UNKNOWN` / `RUNTIME_LOST`，不能自己提交下一阶段。负向需求 **DSH-05**。

### 5.6 测试支持

`docs/testing.md` 定位了测试支持模式。等级：`TEST-SOURCE` / `NOT-RUN`。以后若做 DSH Adapter conformance，可以借鉴其夹具/重放思想，但：

- 不得把 DSH 测试套件复制进可执行 Fake 套件；
- 不得把“文档里有 testing.md”写成测试已通过；
- 本轮所有参考场景保持 `NOT_IMPLEMENTED`。

### 5.7 Windows ACL 沙箱：能力范围且部分

`packages/sandbox/sandbox-windows-acl/README.md` 只证明存在以 Windows ACL 为机制的沙箱包。按 ADR-0007 与隔离设计：

- Windows Job Object 也只是进程治理，不是完整沙箱；
- ACL 更只覆盖 ACL 能表达的文件权限切片；
- 没有本轮实现级证据证明网络、进程树、凭据、symlink/junction 逃逸均被预防。

因此 Windows ACL 声明保持 **capability-scoped 且 partial**：`support_level` 可为 `DEGRADED` 或 `UNKNOWN`，`enforcement_strength` 不得写成全面 `PREVENTIVE`，UI 不得使用未限定的“沙箱”一词。负向需求 **DSH-06**：ACL 不支持、不完整或探测失败时 fail-closed，不得调度需要强隔离的 Assignment。

## 6. 七条 GOGO PARTY 参考需求

每条均为 `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`。完整矩阵见 [12-reference-negative-test-matrix.md](12-reference-negative-test-matrix.md)。

### DSH-01 插件卸载与 in-flight 所有权

插件/effect 撤回不得抹掉进行中 Attempt 的 process、native session 或 spool。卸载时必须先 drain、移交 Runner 所有权或标 `RUNTIME_LOST`。无所有者的 in-flight 工作不得被当成成功取消。

### DSH-02 重复或回退的 SessionEvent

`nativeSeq` 重复、回退或 replay 不得生成第二条权威 GOGO PARTY 事件，也不得回退已提交 Observation。只允许幂等忽略或标为 replay artifact。账本序号永远由 GOGO PARTY 分配。

### DSH-03 过期 subagent 谱系

parent resume/fork/replay 之后，旧 child 的 finish 只能作为过期 `NativeTerminalEvidence` 候选，且必须因 generation/fence 不匹配被拒绝。不得写入当前 Attempt 的 `TerminalReceipt`。

### DSH-04 API 投影丢失 scope

Typert/API 或网关投影若缺少 `project_id` / session / attempt 绑定，必须拒绝。无 scope 对象不得进入 ledger、spool 或 Context 投影。

### DSH-05 workflow worker / host 重启

worker-thread 或 host 重启后，DSH 内部 goal/plan/schedule 不是 GOGO PARTY 耐久状态。Runner 报告 `UNKNOWN`，Controller 先 reconciliation。禁止 DSH worker 在无新 fenced Command 时自动续跑下一阶段。

### DSH-06 Windows ACL 不支持或不完整隔离

ACL 包缺失、探测失败、无法覆盖 Assignment 所需路径/进程/网络/凭据时，CapabilityEvidence 为 `UNSUPPORTED`、`DEGRADED` 或 `UNKNOWN`。不得调度强隔离任务，不得把 partial ACL 写成 Project 隔离已满足。

### DSH-07 源码 pin 与 npm 包版本漂移

Adapter 必须同时记录源码 commit、发行标注和实际解析的包版本。若部署解析 `latest=0.1.0-rc.7` 或版本未知，就不得套用 `dsh@0.1.0-rc.8` / commit `141eb6fef83422698aef7a981029e843e8161534` 的证据；显式解析并验证 `0.1.0-rc.8` 后才可继续该版本的 Gate。developer preview 的破坏性变更使过期证据立即失效。

## 7. 结论

DeepSeek Harness 是 MIT 许可的 developer preview 执行层。Cordis 可逆插件、append-only trajectory、resume/fork/replay、多 provider subagent、类型化 API/stdio 和测试缝，适合作为未来 DSH Adapter 的语义来源。它不能承担 GOGO PARTY Control Plane，也不能用本地 job/goal/schedule/worker 或 Windows ACL 包替换账本、lease、spool、Receipt/Gate、Project 隔离、LoadProof 或 Transition Engine。

下一步若评估 DSH Adapter，先做版本钉死与 probe，再把 nativeSeq / NativeTerminalEvidence 边界写成 conformance；在此之前全部场景保持 `NOT_IMPLEMENTED`。
