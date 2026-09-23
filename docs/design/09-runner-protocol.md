# Runner Protocol

- 状态：v0.2 正式设计候选
- 目标：以同一可靠协议连接本地 Windows、WSL 和未来 Remote Linux Runner，不提供隐藏远程控制通道

## 1. 连接模型

Runner 主动向 Control Plane 建立认证出站连接；Control Plane 默认不反向登录 Host。传输可以是长期双向流或带游标的 pull/ack API，但语义必须一致：至少一次投递、持久 command、持久 receipt spool、幂等确认和断线 reconciliation。

协议不是任意 shell/RPC。Controller 发送类型化 Runner Command；Runner 只能执行已注册 Adapter、Workspace 和 Policy 允许的动作。

## 2. Runner 生命周期

```text
ENROLLING -> ACTIVE -> DRAINING -> OFFLINE | REVOKED
                └──────────────> UNKNOWN
```

- `ENROLLING`：机器身份和 execution profiles 等待审批/验证。
- `ACTIVE`：能力证据新鲜且可接受新 claim。
- `DRAINING`：不接新任务，已有 Attempt 按策略收尾。
- `OFFLINE`：已确认无连接；不等于所有子进程已停止。
- `UNKNOWN`：连接/进程事实不足，需要 reconciliation。
- `REVOKED`：身份被撤销，任何新消息拒绝；遗留进程进入 Attention。

## 3. 消息类型

### 3.1 Runner -> Control Plane

- `runner.register`：runner identity、protocol/adapter versions、execution profiles
- `runner.heartbeat`：capacity、active attempt refs、monotonic sequence
- `capability.report`：CapabilityEvidence 与 Conformance references
- `command.ack | command.reject`：是否接受命令及验证原因
- `observation.append`：process/session/tool/usage 等现场证据
- `receipt.submit`：TerminalReceipt
- `artifact.prepare | artifact.finalize`：内容 Hash、大小、分类和上传状态
- `reconcile.report`：重连后的进程、native session、spooled receipts 和最后命令清单

### 3.2 Control Plane -> Runner

- `runner.accept | runner.drain | runner.revoke`
- `command.deliver`：持久 fenced Command
- `receipt.committed`：Controller 已耐久接收 receipt
- `artifact.upload_grant`：限 scope、限大小、限时上传授权
- `reconcile.request`：要求按已知 command/attempt 清单核对
- `capability.reprobe`：版本/证据过期后重新探测

## 4. 注册与身份

每个 Runner 有稳定 `runner_id` 和机器身份。首次 enrollment 需要本地一次性 bootstrap 或人类审批，持久凭据存于 Host-local provider；仓库和聊天中不保存 token。

每条消息绑定 runner identity、protocol version、sequence、timestamp、nonce/message ID 和 payload digest。Remote transport 必须加密；本地 loopback 也必须防止其他本机进程伪装 Runner。身份轮换和撤销不改变历史 Runner ID。

## 5. Command 接受规则

Runner 在产生 ACK 前必须验证：

1. protocol/schema 支持；
2. command 的 `runner_id/project_id` 与本机授权匹配；
3. Controller epoch 不旧；若属于 Assignment/Attempt 命令，其 fencing token 也不旧；
4. idempotency key 未出现异 payload；
5. command 未过期，expected revision/context/policy/capability digest 匹配；
6. Adapter/credential/workspace/execution profile 可用；
7. `capacity_permit_id`、permit generation/fence、selected target digest 与 Controller epoch 完整，且绑定当前 Project/Assignment/Attempt/Runner；
8. permit 尚可用于启动，resource vector 与命令请求一致，未 release/revoke/expire；`EXPIRED_RECONCILE_REQUIRED` 不得当作空闲；
9. capacity 与 draining/revocation 状态允许。

ACK 表示命令已耐久记录并将尝试执行，不表示进程已经启动。`process.started` 需要后续 Observation。

任何启动类 `command.deliver` 缺少当前 CapacityPermit、携带 stale/duplicate/regressive fence、target digest 不一致，或同幂等键出现不同 permit/resource vector 时，Runner 必须在产生进程或 Harness 副作用前 typed reject。Runner 只验证 Controller 已签发的 permit，不自行选择 Project、排队、续租、释放或换用另一个 Provider/Profile/Model。permit settle/release 由 Controller 根据 TerminalReceipt 或 reconciliation evidence 形成 exact plan。

## 6. Runner 本地耐久 Spool

Runner 必须在本地持久保存：已接收未完成 Command、幂等结果、当前 Attempt/process/native session 引用、未确认 Observation/Receipt 和 artifact finalize 状态。

Receipt 在收到 `receipt.committed` 前不得删除。Spool 有 Project/Attempt 命名空间、配额、加密/权限和保留策略；满载时 Runner 进入 DRAINING/Attention，不得丢弃终止回执后继续接任务。

## 7. 重连与 reconciliation

重连后双方交换：当前 controller epoch、Runner 最后接受 command sequence、Controller 已确认 receipt/event 游标、Runner 观察到的活跃 process/native session、spooled receipt/artifact。

逐项结果：

- 双方一致：继续或补传。
- Controller 有 Command、Runner 未见：若未过期则重投同一 Command ID。
- Runner 有 process、Controller 认为未启动：进入 `COMMAND_OUTCOME_UNKNOWN`，核对 process identity，不创建新 Attempt。
- Runner 有 receipt、Controller 未确认：重投同一 receipt ID。
- Controller 认为运行、Runner 无法证明：`UNKNOWN`，不自动宣称失败或安全重派。

## 8. 流控与顺序

消息只保证每个 producer 的 sequence 可检测缺口，不保证跨 Runner 全局顺序。Controller 按对象 revision、causation、epoch 和 fencing 判定，不依赖到达顺序。

Runner 报告 capacity/backpressure；Controller 不得超过授权并发。Observation stream 可以采样/压缩，但安全事件、state boundary、receipt 和 evidence hash 不得丢弃。

## 9. 版本协商

注册时协商 protocol major/minor、支持消息、最大 payload 和 artifact transport。Major 不兼容则拒绝 ACTIVE；minor 能力通过显式 feature set 协商，不依据版本字符串猜测。

Adapter 和 Harness 版本变化会使相关 CapabilityEvidence 失效，但不应影响 Runner 上其他 Adapter。

## 10. 安全禁止项

- UI 不得向 Runner 下发未审计的任意 shell 字符串。
- Controller 不得索取原始 credential、完整环境变量或主目录扫描。
- Runner 不得接受另一个 Project 的 workspace/session/artifact 引用。
- 日志和 Observation 必须脱敏；敏感 raw stream 留在 Host 或加密 artifact 中。
- Remote Runner 的在线状态不证明其 Host 未被攻陷；高风险任务按 Project Policy 选择信任级别。
