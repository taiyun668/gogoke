# ADR-0004：单一逻辑 Controller 与 fencing

- 状态：Proposed（v0.2 设计候选，待产品所有者评审）
- 日期：2026-08-11

## Context

多个 Agent、UI、Repository watcher 或 Controller 实例并发推进 Task/Fact 会产生双重执行、旧 leader 写入和绕过 Gate 的风险。单纯约定“只运行一个进程”又无法覆盖崩溃接管和未来高可用。

## Decision

Controller 是 Party、Task/Assignment 权威状态、Claim/Lease、Command、Canonical Fact promotion、GateEvaluation、Decision 和 Accepted Event 的唯一逻辑写入者。

每个控制域同一时刻只有一个持有 leader lease 的 Controller；接管生成单调递增 `controller_epoch`。每次 Assignment claim 生成单调递增 `fencing_token`。Runner 与 Coordination Store 必须拒绝旧 epoch/token 写入。

Agent 只能提交自己 namespace 的 Result/Evidence Proposal；UI、Reactor、Runner 和 Chief of Staff 只能提交 Intent、Candidate Signal 或 Observation。

## Consequences

- 高可用可以有多个物理实例，但不能多 leader 写入。
- Controller 重启/接管必须先 reconciliation，再决定重派。
- 所有外部副作用通过 transactional outbox 至少一次投递并幂等确认。
- Agent 自报 PASS、文件出现或命令已发送都不会直接推进 Workflow。
- Controller/Coordination Store 成为关键组件，需要备份、迁移、崩溃恢复和故障注入测试。
