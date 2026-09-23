# ADR-0001：仓库保存协作事实而非完整对话

- 状态：Proposed（v0.2 设计候选，待产品所有者评审）
- 日期：2026-08-11

## Context

不同 Harness 的完整对话格式、隐私、体积和可恢复能力差异很大。把所有 Transcript 复制到统一仓库会放大上下文污染、存储成本和供应商耦合。

## Decision

Control Repository 的对象限定为 Task 定义/快照、Accepted Fact/Result、Evidence 引用、Decision、Accepted Event、Role、Workflow 和 ContextSnapshot Manifest。Transcript 留在 Harness 私有存储，仅在用户显式查看时作为非权威诊断资料。活跃 claim、lease 和 command 不进入 Git 锁模型，而由持久 Coordination Store 管理。

## Consequences

- Session 可以替换而不丢失正式工作状态。
- GOGO PARTY 不需要解析所有供应商的完整历史才能运行。
- Agent 的有用发现必须经过 Result/Fact promotion。
- 某些调试仍需跳转到 Harness 原生历史。
- “repository-backed” 不等于“只使用 Git 作为数据库”。
