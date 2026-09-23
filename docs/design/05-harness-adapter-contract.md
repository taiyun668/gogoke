# Harness Adapter 契约

- 状态：v0.2 正式设计候选
- 目标：用一个严格、可测试的内核接入快速变化的 CLI Harness，同时保留能力差异

## 1. 边界

Adapter 只负责 Harness transport、生命周期调用、项目文档投影、原生事件归一化和 receipt 收集。它不得决定 Workflow、Gate、重试、Agent 角色、权限放宽或结果是否成为事实。

核心层不得解析各 Harness 的随意 stdout 来推断成功；Generic PTY Adapter 可以提供观察与人工 attach，但没有 terminal receipt 时不能宣称结构化完成能力。

## 2. AdapterDescriptor

```yaml
adapter_id: codex-cli
adapter_version: 0.1.0
harness_id: codex
supported_versions: ">=x <y"
  integration_modes: [native_rpc, headless_json]
platforms: [windows-native, linux, wsl]
projector_ids: [agents-md]
conformance_suite: gogo-adapter/v1alpha1
```

版本范围只是声明；每台 Host 的实际可用性由 CapabilityEvidence 决定。

## 3. 标准方法

```text
discover(host) -> installations[]
probe(installation, execution_profile) -> CapabilityEvidence
prepare_project_context(workspace, ContextSnapshot, EffectivePromptManifest)
  -> ContextProjectionReceipt
create_session(SessionSpec) -> NativeSessionRef
send(SessionRef, AssignmentEnvelope) -> NativeCommandRef
steer(SessionRef, SteeringEnvelope) -> NativeCommandRef | Unsupported
interrupt(SessionRef, reason) -> Observation
stop(SessionRef, reason, deadline) -> Observation
observe(SessionRef, cursor) -> NormalizedObservation[]
collect_terminal_evidence(SessionRef, AttemptRef)
  -> NativeTerminalEvidence | Pending | Unsupported
attach(SessionRef) -> AttachDescriptor | Unsupported
archive(SessionRef) -> ArchiveReceipt
```

每个改变状态的方法都接收 GOGO `command_id`、controller epoch、fencing token、deadline 和 cancellation signal，并返回可关联的 native reference。

## 4. 能力模型

能力不是简单 true/false。每项至少记录：

- `support_level`: `SUPPORTED | DEGRADED | UNSUPPORTED | UNKNOWN`
- `integration_mode`: `SDK | ACP | NATIVE_RPC | HEADLESS_JSON | PTY | READ_ONLY`
- `evidence_quality`: `EXACT | NATIVE_OBSERVED | HEURISTIC | UNKNOWN`
- `enforcement_strength`: `PREVENTIVE | APPROVAL | POST_HOC | NONE`
- `verified_version`、`verified_at`、`expires_at`
- 限制和 Conformance Test 引用

标准能力维度：

- session create/resume/fork/archive
- structured input/output/terminal receipt
- streaming events与 event cursor
- mid-run steering/interrupt/cancel
- native usage/context-window observation
- tool permission enforcement
- filesystem/network/sandbox enforcement
- project document support与 precedence
- headless/background execution
- interactive attach
- Windows Native / WSL / Linux support

调度按能力要求匹配；`DEGRADED` 只有在 Project Policy 明确允许时才可使用，`UNKNOWN` 默认不可满足强制要求。

## 5. 规范化事件与证据强度

Adapter 可产生以下 Observation 类型：

- `session.created | resumed | waiting | archived | unknown`
- `process.started | exited | lost`
- `assistant.message.delta | completed`
- `tool.requested | started | completed | denied`
- `permission.requested`
- `usage.observed`
- `context.compacted | context.pressure_observed`
- `receipt.available`
- `adapter.error`

所有事件携带 native event ID/cursor（若有）、GOGO correlation、原始时间和 payload digest。原始事件可以作为受限 artifact 保留，但归一化不得丢失安全相关含义。

事件必须携带 `source_strength`：`OFFICIAL_PROTOCOL | OFFICIAL_HOOK | OWNED_PROCESS | PTY_PARSER | TRANSCRIPT | HEURISTIC`。低等级来源可以补充活动与诊断，不能覆盖新鲜的高等级终态；Hook 断开或过期后的降级必须形成可审计事件。PTY idle、最后一行、assistant 自述和进程 exit 0 均不能单独形成成功证据。

## 6. Native Terminal Evidence 与 TerminalReceipt Fence

Adapter 只返回与当前 native session generation/turn fence 绑定的 `NativeTerminalEvidence`，不签发 GOGO TerminalReceipt。Runner 只有在以下条件同时满足时才能组装 TerminalReceipt：

1. native execution 已进入可证明终态；
2. 结构化 ResultCapsule 通过本地 Schema 校验；
3. source revision/artifact/evidence 已固定并可读取；
4. receipt 已写入 Runner 的耐久 spool；
5. stop/exit 原因没有被 stdout 文本冒充。

Runner 必须先把 TerminalReceipt 写入耐久 spool，再向 Controller 提交。Controller 验证 project、assignment、attempt、epoch、fencing、幂等键、终态证据和 payload digest 后提交 ReceiptRecord；随后才执行 Policy/Gate。Runner 必须等待 `receipt_committed`，网络不确定时按 receipt ID 查询，不重新运行。Adapter 原生终态、Runner 回执、Controller ReceiptRecord 和 GateEvaluation 是四个不同层级，不得合并。

## 7. Project Context Projector

每个 Harness-specific Projector 把同一个 ContextSnapshot/EffectivePromptManifest 编译为 Harness 能识别的文件或启动参数。投影必须：

- 标明 generated、snapshot ID、prompt digest 和 projector version
- 在隔离 workspace 内生成
- 校验 Harness 实际加载路径与 precedence
- 记录写入文件 Hash
- 不把 Harness 私有缓存提升为项目事实
- 检测生成文件漂移，不静默反向同步

Context Compiler 与 overlay 冲突裁决属于 Control Plane；Projector 只负责 vendor 目标路径/协议字段和加载证明。写入成功不是加载证明。mandatory 片段缺失、冲突、hash 不匹配或缺少策略要求的 load proof 时，`create_session/send` 必须 fail-closed。

## 8. 错误分类

统一错误：`NOT_INSTALLED`、`VERSION_UNSUPPORTED`、`AUTH_NOT_READY`、`CAPABILITY_MISSING`、`CONTEXT_PROJECTION_FAILED`、`SESSION_CREATE_FAILED`、`DISPATCH_REJECTED`、`RUNTIME_LOST`、`RECEIPT_UNAVAILABLE`、`PROTOCOL_VIOLATION`、`CANCEL_UNCONFIRMED`、`ADAPTER_BUG`。

错误必须声明 `retryability = SAFE | RECONCILE_FIRST | NEVER | UNKNOWN`。Adapter 不自行重试会产生副作用的方法。

## 9. Conformance Suite

任何 Adapter 在 UI 标为可调度前必须通过与所声明能力对应的测试：

1. 安装/版本/认证探测不泄露 secret。
2. Context 投影可复现且加载位置得到验证。
3. start 重复投递不会双启进程。
4. 旧 epoch/fencing token 被拒绝。
5. structured receipt 与 evidence/revision 一致。
6. Runner 断线、Controller 重启后可 reconciliation。
7. cancel/interrupt 的实际语义与声明一致。
8. resume/fork 不会串 Project、Agent 或 Session。
9. unsupported 能力明确失败，不做启发式伪装。
10. Windows/WSL/Linux 分别出具独立结果。

CapabilityEvidence 到期、Harness 版本变化或 Adapter 更新后，相关测试必须重新执行；UI 显示最后验证版本和时间。

## 10. Generic CLI Adapter

Generic Adapter 提供最低公共能力：探测 binary、设置 cwd/env allowlist、启动进程、捕获 stdio、超时/取消、人工 attach 和 artifact 收集。除非目标 CLI 有经过验证的机器协议或 wrapper receipt contract，否则其能力必须标为 PTY/heuristic，不得用于要求精确 terminal receipt、可靠 resume、blind audit 或强权限执行的 Workflow。

## 11. 公众版 Codex / Grok Build transport 基线

2026-08-11 的公众发行版 Spike 固定如下，不允许实现静默换成私有组件：

- Codex：主通道为 app-server stdio `NATIVE_RPC`；`exec --json --ephemeral` 仅作 one-shot `HEADLESS_JSON` 降级。
- Grok Build：主通道为 `ACP` stdio；`-p --output-format streaming-json` 仅作 one-shot `HEADLESS_JSON` 降级。

Codex 只有 `turn/completed` 可以形成 native terminal fence；Grok ACP 只有 prompt response 的 stop reason、headless 只有最终 `end` 可以形成 native terminal fence。PTY 文本、assistant 自述、进程 exit 0 和最后一行 stdout 都不能单独形成成功回执。

Adapter 在归一化前必须执行 host-local Privacy Firewall：认证/账号/限额信息、reasoning、完整 transcript 和用户级插件配置默认不得进入 Project event stream 或 Git。可恢复的 Harness 配置问题归一化为 `adapter.warning`，不得静默忽略，也不得未经分类直接把 Task 判为失败。
