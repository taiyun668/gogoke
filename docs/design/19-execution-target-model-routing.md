# Execution Target、模型路由与 Provider Readiness

- 状态：`v0.2-p31-d1-product-contract-draft`
- 性质：P31 D1 产品契约草案；协议形状由 P32 冻结，registry/broker/Runner enforcement 分别由 P40/P37/P41 负责
- 原则：Assignment 可以请求能力和偏好，但每个 Attempt 必须绑定一个精确、可审计且不可静默替换的实际执行目标

## 1. 为什么这是核心对象

GOGO PARTY 的价值来自同时治理多个 Harness、模型、账号/Profile 和执行 Host。只记录 `harness_id` 无法回答“实际是谁、在哪台机器、以哪个模型和哪份凭据边界执行”，也无法诚实解释成本、上下文容量、限流、降级和结果差异。

因此，Harness 名称、模型名称、CredentialHandle 和 Runner 不能作为若干松散字段临时拼接。Controller 必须在 claim 前形成版本化 `ExecutionTarget`，Attempt 和 TerminalReceipt 绑定实际目标。

## 2. 对象边界

```text
ExecutionTarget
  ├─ HarnessInstallationRef      官方发行来源、安装实例、版本与摘要
  ├─ AdapterRef                  Adapter 版本与 Conformance 证据
  ├─ HarnessProfileRef           shared/dedicated 配置与 native session 域
  ├─ ModelRef                    provider/model/version 或不可获得版本的诚实标记
  ├─ CredentialHandleRef         仅 Host-local locator，不含秘密
  ├─ Runner/HostRef              execution profile 与隔离能力
  ├─ CapabilityEvidenceDigest    本次可依赖的能力快照
  ├─ ProviderReadinessDigest     登录/额度/限流/服务状态的脱敏快照
  ├─ PricingSnapshotRef          可选；来源和新鲜度必须可见
  └─ FallbackPolicy              允许候选、禁止替换和人工确认规则
```

`ModelProfile` 描述模型能力、上下文窗口、结构化输出、工具/图像支持、计价来源和证据新鲜度；它不是供应商营销名称的复制。无法验证的字段为 `UNKNOWN`，不得用同系列模型推断。

`ProviderReadinessEvidence` 只暴露调度所需的脱敏状态，例如 `READY | LOGIN_REQUIRED | QUOTA_EXHAUSTED | RATE_LIMITED | UNAVAILABLE | UNKNOWN | STALE`、scope、有效期和证据等级。它不得携带 token、cookie、账号邮箱、完整限额响应或可用于登录的材料。

## 3. 选择与绑定

Assignment 保存：

- `target_requirements`：必需 Harness 能力、模型能力、Host/OS、数据驻留、Profile 隔离和预算上限；
- `target_preferences`：允许的模型/成本/速度偏好和稳定排序规则；
- `allowed_target_set_digest`：Controller 实际评估的完整候选集合；
- `fallback_policy`：是否允许替换、替换范围、确认 Gate 和最大次数。

Controller 基于同一 SelectionUniverse 选择目标，产生 `TargetSelectionRecord`，并通过 ResourceBroker 同时形成绑定该 target 的 `CapacityPermit`。Attempt/Claim/Lease/Permit/Command 原子创建时固定 `selected_execution_target_id + target_digest + capacity_permit_id/fence`；Runner 只接受该目标。Adapter 只能报告 native identity 和能力/容量证据，不能自行换模型、账号、Profile、Host/Harness，也不能选择优先服务哪个 Project。

Receipt 同时记录 requested target、selected target、实际 native model evidence 和任何差异。无法证明实际模型时，结果可以保留，但模型字段必须为 `UNKNOWN`，相关 Conformance/成本声明不得 PASS。

## 4. 禁止静默替换

以下变化都视为新选择，不是透明重试：

- Codex 换成 Grok，或反向；
- 同一 Harness 中切换模型、模型版本或推理档位；
- 切换账号、CredentialHandle、HarnessProfile 或 Provider endpoint；
- 从 dedicated profile 降级到 shared user profile；
- 从 structured protocol 降级到 PTY/transcript；
- 切换 Host、Runner 或数据驻留域。

若 Policy 允许 fallback，Controller 必须创建新 Attempt 或显式 target generation，保存原失败证据，再次执行能力、预算、凭据、上下文容量和权限检查，并重新编译 ContextSnapshot 与 LoadProof。已有工具活动或 workspace 修改时禁止自动替换：必须先 reconciliation，并经人工 Gate 后才能启动新 target generation。Runner 启动命令必须绑定并验证当前 CapacityPermit identity/generation/fence、selected target digest 与 Controller epoch；校验失败不得产生进程副作用。后置施工 owner 是 P41。

## 5. Context、预算与模型的关系

Context Compiler 接收的是固定的 ModelProfile/ExecutionTarget capacity snapshot，而不是在编译中重新探测供应商。若目标变化，ContextSnapshot、预算、projection 和 LoadProof 必须重新计算；不同模型的窗口和 tokenizer 估计不能共用一个“剩余百分比”。

价格和额度只用于策略与显示：

- provider-reported、官方价格快照、本地估计和 unknown 必须分级；
- 本地累计 token 不得伪装成官方剩余额度；
- pricing snapshot 过期时可阻止成本敏感的自动 claim，但不能抹去已发生 Attempt；
- 配额耗尽只冻结有明确归因的目标，不得把网络、认证或全局 Provider 故障归因到账号。

## 6. UI 表达

Project、Task、Agent 和 Attempt 必须显示：Harness、模型、Profile 类型、Host、能力/Readiness 新鲜度、成本证据等级以及是否发生显式 fallback。普通用户可以选择“推荐目标”，但推荐结果仍展开为精确目标，并允许在运行前查看为什么选它。

UI 不得只显示品牌 Logo；发生替换时必须形成时间线事件和 Attention，不能让同一个 Agent 卡片无提示地换了底层执行身份。

## 7. MVP 验收

MVP 至少证明：

1. 同一 Project Workflow 中，Implementer 与 Auditor 分别绑定 Codex 和 Grok 的精确 ExecutionTarget，并完成自动接力。
2. 过期 capability/readiness、模型不可用、login-required 和 quota-exhausted 都会在 claim 前给出不同 typed 结果。
3. Adapter 尝试静默切模型/Profile/账号/Host 时被拒绝。
4. 明确允许的 fallback 产生新 Attempt/target generation、保留原证据并重新编译 Context。
5. Receipt 可追溯 requested/selected/observed target；observed 不可验证时保持 `UNKNOWN`。
6. Project A 的 CredentialHandle、Profile readiness、预算和目标候选不会进入 Project B 的事件或 Context。
7. 两 Project 共享 Host/Provider/Profile 时，CapacityPermit 的公平、aging、fence、崩溃恢复和脱敏符合 `21-cross-project-resource-arbitration.md`。

## 8. 施工归属

- P31：产品语义、用户选择和 fallback 边界。
- P32：vendor-neutral wire/schema 与版本兼容。
- P33：选择记录、Attempt/Receipt target binding 以及 CapacityPermit reservation/release 的 Transition/Store 集成。
- P35：CredentialHandle/HarnessProfile 与 login-required。
- P40：W4 负责 ExecutionTarget Registry/ModelProfile/ProviderReadiness ingestion。
- P37：W2 Controller/Core 负责 target+capacity 的确定性选择。
- P41：W3 Runner 负责在 ACK/进程副作用前验证 CapacityPermit identity/generation/fence、selected target digest 与 Controller epoch，并硬拒绝不匹配命令；不拥有选择或释放权威。
- P18/P21：API 和诚实投影。
- P23/P24/P25：公众版 Adapter 证据与跨 Adapter 集成。
