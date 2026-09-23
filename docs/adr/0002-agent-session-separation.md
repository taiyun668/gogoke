# ADR-0002：Agent 身份与 Session 生命周期分离

- 状态：Proposed（v0.2 设计候选，待产品所有者评审）
- 日期：2026-08-11

## Context

长期任务中，聊天窗口会因阶段变化、上下文漂移、压缩或容量而失去继续执行的质量，但角色、职责和工作历史仍需保持。

## Decision

AgentInstance 是 Project 内持久身份；Session 是某 Harness 中的临时上下文载体。一个 Agent 可以拥有多个历史 Session；默认只有一个可写 Active Session，Workflow 显式允许并行 Assignment 时可以有多个隔离的 Active Session。任一 Session 只能属于一个 Agent 和一个 Project。

## Consequences

- Session Governor 可以在安全点轮换窗口。
- UI 以 Agent 为主对象，Session 作为历史和运行详情。
- Handoff Capsule 与 Context Handshake 成为必要协议。
- 不再把原生 Session ID 当作 Agent ID。
- resume、native fork 和 cold rebuild 是不同连续性模式，UI 必须如实显示。
