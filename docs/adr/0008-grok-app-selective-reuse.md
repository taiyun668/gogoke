# ADR-0008：允许选择性复用 Grok App 源码，禁止运行时接入

- 状态：Accepted
- 日期：2026-08-11

## Context

GOGO PARTY 首发需要面向公众版 Grok CLI。公开的 Grok App 已经实现成熟的 ACP client、Session FSM、多会话宿主、权限交互、stall/watchdog、进程回收和上下文用量。完全重写会重复高风险运行时工程；但 Grok App 是非官方社区产品，不能成为 GOGO PARTY 的组件、兼容依赖或接入对象。

## Decision

1. Grok Adapter 的规范主路径仍是由 GOGO Runner 直接连接公众版 `grok agent stdio`。
2. 允许在 MIT License 边界内，从固定 commit 选择性移植 Grok App 的 Grok 专属运行时模块及其测试，形成 GOGO 内部 `grok-runtime`。上游来源、commit、修改和许可必须可追溯。
3. 禁止检测、启动、嵌入或连接用户安装的 Grok App；禁止连接其 Mirror RPC、Tauri IPC、本地 store 或原生 Session。
4. 不复用 Grok App 的 Automation、Project store 或 journal 作为 GOGO 的 Canonical/Coordination Plane。
5. 派生代码必须由 GOGO 自己构建、发布和治理，不得让 Grok App 进程承担任何 runtime authority。
6. 必须按公众版 Grok CLI、GOGO Adapter 和 OS 版本重跑 capability probe 与 Conformance Suite；Grok App 版本不属于兼容矩阵。

## Consequences

- 首版桌面技术栈优先采用 Tauri 2 + Rust Host + React/TypeScript，降低 Windows 子进程与 stdio 管理风险。
- Grok Adapter 可以更快达到稳定会话与可观测性，但需要维护上游派生补丁和 LICENSE/NOTICE。
- GOGO 不承诺读取、迁移或控制 Grok App 的既有会话。
- 产品 UI、安装器和文档不得把 Grok App 表述为官方或受支持组件。
- GOGO 的 Controller、Project 隔离、任务账本、Context Compiler 和 Gate 语义保持独立。

## Evidence

见 `docs/research/grok-app-reuse-audit.md`。
