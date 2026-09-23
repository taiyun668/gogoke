# ADR-0005：四平面状态模型与持久协调内核

- 状态：Proposed（v0.2 设计候选，待产品所有者评审）
- 日期：2026-08-11
- 替代：ADR-0003

## Context

Git 适合可审阅、可交换的项目事实，却不提供可靠原子 claim、lease、CAS 和低延迟命令；heartbeat/PTY 又是高频可重建观察；Harness transcript/checkpoint 则受供应商控制。将这些简化为“持久与实时”两层会误把关键协调状态当作缓存。

## Decision

系统分为四个语义平面：

1. Canonical Fact Plane：Accepted Fact/Result、Decision、证据引用和精确 repository revision，以 Control Repository 为交换/恢复载体。
2. Coordination Plane：Task 活跃状态、Assignment、Claim、Lease、Attempt、Command、幂等、offset 和 outbox，保存在持久事务数据库。
3. Live Observation Plane：heartbeat、process/PTY、stream cursor、usage 等可重建观察。
4. Native Session Plane：Harness transcript、checkpoint、tool history 与本地缓存。

Gate 通过的对象先为 `PROMOTION_PENDING`；Repository Projector 物化并返回 commit/object hash 后才成为 Canonical `ACCEPTED`。跨存储使用 outbox + 幂等 saga + reconciliation，不声称分布式单事务。

## Consequences

- Git 不承担锁、租约或实时消息队列职责。
- Coordination Store 不是可随意清空的 Runtime cache，需要持久性与备份。
- UI 必须展示规范状态、现场观察和证据新鲜度，不能合并成无来源的一个绿点。
- Coordination Store 丢失时 in-flight 工作进入 UNKNOWN/reconciliation，不能只从 Git 自动重派。
- 本地 Git commit 可完成本地 canonical materialization；remote push 是否为 Gate 由 Project Policy 决定。
