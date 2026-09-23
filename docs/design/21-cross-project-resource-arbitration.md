# 跨 Project 资源仲裁与 Capacity Permit

- 状态：`v0.2-p31-d1-product-contract-draft`
- 性质：P31 D1 产品契约草案；wire 由 P32、Transition/Store 由 P33、readiness registry 由 P40、目标选择由 P37、Runner 执行侧硬门由 P41 负责
- 原则：Project 隔离不等于每个 Project 拥有独立的物理容量；所有共享稀缺资源都必须先由 Controller 获得原子、可恢复的许可

## 1. 问题边界

MVP 必须允许两个 Project 同时存在活跃 Attempt，但它们可能共用 Host CPU/内存、Runner 进程槽、HarnessProfile、Provider 限流窗口、模型并发额、网络和磁盘。仅在 Project 内做 BudgetReservation 无法阻止：

- 一个 Project 长期占满共享 Provider/Profile；
- UI 排序或 Adapter 本地限流变成隐式调度权威；
- Host 崩溃后容量永久泄漏或旧 Attempt 继续占用；
- 为做公平调度而泄露其他 Project 的任务、账号或用量细节。

## 2. 权威对象

```text
ResourcePool
  ├─ scope: host | runner | provider | profile | model | project-budget
  ├─ capacity units + evidence/freshness
  ├─ admission/fairness policy version
  └─ current generation/epoch

CapacityRequest
  ├─ project/assignment/attempt candidate identity
  ├─ exact requested units and duration bound
  ├─ priority class + project share policy
  └─ target/universe/policy digests

CapacityPermit
  ├─ exact pool/request/attempt binding
  ├─ lease expiry + fencing token
  ├─ granted units and generation
  └─ release/settlement/reconciliation state
```

ResourceBroker 是 logical Controller 内的决策组件，不是 Adapter、Runner、UI 或 SQLite 的第二调度器。Coordination Store 只对 Controller 形成的 exact permit plan 执行机械 CAS、lease/fence 和原子写入。

## 3. 公平、优先级与抢占

P31 必须冻结以下用户可见语义：

- 默认按 Project 做有界 weighted-fair admission，不以创建任务数量占满队列；
- 人工优先级只影响尚未运行的 request，不默认强杀已产生不可逆副作用的 Attempt；
- 持续等待的 Project 获得有界 aging，但不能越过权限、数据驻留、预算或 target readiness 硬约束；
- Pause Project 停止其新 permit；Stop Attempt 需 fenced cancel 和真实终止证据后才释放不可共享容量；
- 任何抢占、优先级调整和保留额变更都产生 Decision/Event，不得由 UI 拖拽直接改运行态。

MVP 的确定性公平合同必须由 P31 冻结为可测试参数，而不是一句“尽量公平”：

- 每个 Project 默认 weight 为 `1`；非默认 weight、priority class 和 reserve 只能来自版本化 Policy/Owner Decision；
- 同一 eligibility class 内先比较归一化已服务量，再比较有上限的 aging bucket，最后以稳定 `request_id` 排序；不得以到达线程、UI 顺序或哈希容器迭代顺序破坏确定性；
- aging 只提升合格 request 的排序，不可越过 permission、budget、residency、target readiness 或 credential/profile 隔离；
- starvation bound：两个等权且持续合格的 Project，任一方在对方仍合格时最多被连续旁路 **2** 次 grant decision。双 Project 持续负载是机器验收，不是“尽量公平”；
- Provider quota、token 余额和速率预测属于 `SOFT_ESTIMATE`，只能降低 readiness 或触发 Attention；只有具有 scope、generation、freshness 和可证明单位的 `HARD_CAPACITY` 才能签发精确 permit。
- Runner 启动命令必须绑定并验证 permit identity/generation/fence、selected ExecutionTarget digest 与 Controller epoch；失败不得产生进程副作用。后置施工 owner 是 P41。

## 4. 选择与事务边界

TargetSelectionRecord 与 CapacityPermit 必须绑定同一 SelectionUniverse version、ExecutionTarget digest、Project/Assignment 和 Controller epoch。正确顺序是：

1. Controller 读取完整 target/resource universe 与 end witness；
2. 事务外应用硬约束、公平策略和确定性排序；
3. 形成同一 exact ClaimAssignmentPlan 内的 target selection + capacity permit plan；
4. Store 在一个短事务内验证 version/fence 并原子写 Claim/Attempt/Lease/Permit/Command/Event/Outbox；
5. 任一 universe 或 capacity generation 漂移则零写拒绝，Controller 从新 version 重算。

不得先创建 Attempt 再“补扣容量”，也不得把 Adapter 启动失败当作公平调度机制。

一个 Attempt 通常同时需要 Host、Runner、Provider/Profile/Model 等多个 pool。`CapacityRequest` 因此是规范化的多 pool resource vector：

- Store 只接受全有或全无的单事务 grant，不允许先占一个 pool 再等待另一个 pool；
- pool identity 使用稳定的规范顺序参与 canonical bytes、锁/CAS 计划和冲突报告；不得通过运行时锁获取顺序形成死锁；
- 任一分量未知、过期或 generation 漂移，整个 grant 零写拒绝；不得留下部分 reservation；
- 同一 request 的相同幂等键异 resource vector 必须硬冲突；
- permit ACK 丢失时按 request/permit identity 查询并重放同一结果，不能换 ID 再扣一次。

## 5. 隔离与隐私

- Project 只看到自己的 request/permit/queue position class 和等待原因，不看到其他 Project 的任务、Profile、账号、模型用量或精确优先级。
- 共享 Profile 的容量证据必须脱敏；无法证明 scope 时为 `UNKNOWN/UNAVAILABLE`，不得合并不同账号的额度。
- 全局运营投影可显示池级别繁忙度和饥饿告警，但必须使用脱敏聚合，不能反向推导项目内容。

## 6. 恢复与 UNKNOWN

Host/Controller 崩溃后，旧 permit 不得仅因进程不可见就释放。恢复者必须核对 owner process identity、Controller epoch、Attempt fence、Runner/Harness 观察和 terminal receipt：

- 已证明终止：原子 settle/release；
- 活跃且 fence 仍当前：恢复/renew；
- 矛盾或无法证明：进入 `INDETERMINATE/RECONCILE`，保留容量或只开放显式安全余量，不盲目重派。

lease expiry 只表示 permit 不可再用于新启动或新副作用，不表示容量已经安全空闲。到期 permit 在 terminal/process identity 尚未对账时进入 `EXPIRED_RECONCILE_REQUIRED`；只有当前 fence 的终止证据或显式安全余量 Policy 才能允许其他 Attempt 使用对应容量。

## 7. MVP 验收

1. 两 Project 在共享 Host 与 Provider 上持续运行，任一方不能用大量 Assignment 永久饥饿另一方。
2. 同一容量只有一个当前 fence 可写；duplicate/regressive/stale permit 全部拒绝。
3. target readiness 或 capacity version 在 claim 前漂移时零写拒绝，不产生孤儿 Attempt。
4. Pause/priority/aging 不越过 permission、budget、residency 或 Credential/Profile 隔离。
5. Controller/Host 崩溃、permit ACK 丢失、租约过期、旧 Runner 迟到回执和双 Controller 都不会双重占用或双重执行。
6. Project A 的资源事件、队列信息和容量证据不泄露 Project B 的身份或活动。

## 8. 施工归属

- P31：公平、优先级、保留额、Pause/Stop 和用户可见语义。
- P32：ResourcePool/CapacityRequest/CapacityPermit 的 vendor-neutral schema/wire。
- P33：原子 reservation、lease/fence、settle/release 与 recovery authority。
- P40：Registry/readiness evidence ingestion；Adapter 只提供证据。
- P37：Controller/Core 的 deterministic target+capacity 选择与 ResourceBroker。
- P41：Runner 在 ACK/进程副作用前验证 CapacityPermit identity/generation/fence、selected target digest 与 Controller epoch。
- P18/P19/P21：经认证 API、脱敏投影和 operator 交互。
- P27/P34：饥饿、崩溃恢复、跨 Project 泄漏和双占用攻击验证。
