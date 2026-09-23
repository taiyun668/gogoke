# SQLite Coordination 机器契约总审

- 日期：2026-08-11
- 状态：draft DDL 与 Fake 事务参考模型通过；ADR-0009 仍为 `Proposed`
- 边界：只创建可删除的临时 SQLite 数据库，不接 Runner、Harness、Git、网络或凭据

## 1. 落地裁决

1. 权威根是 Project；领域子表使用 Project 复合主键/外键，不能依赖查询时追加 filter。
2. 调度权位于 Assignment，不位于 Task。相同 Task 可按 `role_slot_key` fan-out 多个 Auditor，但同一 role slot 不能重复。
3. `assignment_revision` 是 CAS 版本，`fence_high_water` 是执行权世代；Attempt 固化 controller epoch 与 fence，两者不得合并。
4. Claim 使用短 `BEGIN IMMEDIATE`，一次事务同时写 Assignment CAS、Attempt、Claim、Lease、Command、idempotency、Accepted Event 与 outbox。
5. Lease 到期只进入 `OUTCOME_UNKNOWN + reclaim_required + reconcile outbox`，绝不直接重新 READY 或创建新 Attempt。
6. Claim、Lease、Command、Artifact 与 Receipt 的数据库外键同时绑定 Project、Assignment、Attempt、controller/runner epoch 与 fence，避免遗漏应用层检查时接受旧执行权。
7. `COMMITTED` Receipt 只接受 official protocol/hook 强度、当前 lease/fence 和已验证 Artifact；Gate 仍在后续事务执行。
8. COMMIT 响应丢失后使用原 operation/key/digest 查询并核对原子行，不创建新 ID、不盲重放外部副作用。

## 2. 当前机器证据

`spec/draft/v1alpha1/sqlite/0001_coordination.sql` 创建 21 张严格表及所需 partial/claim/outbox/sequence 索引。迁移文件摘要由 runner 在应用后记录，避免文件自包含自身摘要的递归问题。

`validate_sqlite_draft.py` 当前通过五组故障导向测试：

- DDL、PRAGMA、migration digest、integrity/foreign-key check、NULL scope 与跨 Project 拒绝；
- 8 路并发 claim 恰一赢家，同 key/digest 返回原 Attempt，同 key/异 digest 冲突；
- current fence heartbeat 成功、旧 fence 失败，lease expiry 不自动重派；
- 第二 writer 在 `BEGIN IMMEDIATE` 前得到有界 BUSY，确认零权威行/零副作用；释放锁后固定 ID 的 claim 可正常执行；
- PTY、旧 fence、未验证 Artifact 均不能 commit；模拟 COMMIT 后响应丢失可查询收敛，并保持单 Receipt/单 outbox。

本轮 PASS 只证明临时 Fake 数据库中的参考语义。没有证明真实磁盘断电、WAL checkpoint starvation、Rust driver、OS crash、Remote Runner ACK、Git materialization 或真实 Harness 行为。

## 3. 下一阶段

Fake trace/fault interpreter 已完成 54/54 条内存参考执行并命中全部 42 个声明式 fault，详见 `17-fake-kernel-executable-subset-review.md`。下一步应让整条 trace 直接驱动本 SQLite 事务模型，而不是把内存模型演变为第二权威；完成后再进行唯一 ProcessSupervisor 的 Windows bake-off。真实 Codex/Grok Adapter 仍排在 Fake Kernel 与 Supervisor Gate 之后。
