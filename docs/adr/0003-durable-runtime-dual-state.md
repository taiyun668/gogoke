# ADR-0003：持久状态与实时状态双层模型

- 状态：Superseded（草案由 ADR-0005 替代）
- 日期：2026-08-11

## Context

Git 适合保存低频、可审计事实，但不适合 Token stream、heartbeat、PTY cursor 和高频进程状态。只用 SQLite 又会失去离线可读、版本化和仓库接力能力。

## Decision

Control Repository 保存持久协议对象；Runtime Store 默认使用 SQLite 保存实时、可重建投影。大型产物进入 Artifact Store，凭据留在 Host-local Secret Provider。

## Consequences

- UI 可以实时更新而不制造大量 Git commit。
- Control Plane 重启后必须执行 reconciliation。
- Repository 与 Runner 观察冲突时使用 UNKNOWN/RECONCILING，不能猜测。
- 需要明确每种对象的权威存储位置。

## Supersession

源码审计证明“Git 持久 + SQLite 可重建实时”仍把原子 claim、lease、command 和 consumer offset 混入所谓可重建状态。ADR-0005 将其替换为 Canonical Fact、Coordination、Live Observation、Native Session 四平面，并把 Coordination Store 定义为不可随意丢弃的持久数据库。
