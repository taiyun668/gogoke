# Architecture Decision Records

ADR 记录一旦影响协议、兼容性或安全边界，就不能只存在于聊天中。

状态：

- `Proposed`：待评审
- `Accepted`：当前设计基线
- `Superseded`：被后续 ADR 替代
- `Rejected`：明确不采用

当前 ADR：

- [ADR-0001：仓库保存协作事实而非完整对话](0001-repository-facts-not-transcripts.md)
- [ADR-0002：Agent 身份与 Session 生命周期分离](0002-agent-session-separation.md)
- [ADR-0003：持久状态与实时状态双层模型（已由 ADR-0005 替代）](0003-durable-runtime-dual-state.md)
- [ADR-0004：单一逻辑 Controller 与 fencing](0004-controller-authority.md)
- [ADR-0005：四平面状态模型与持久协调内核](0005-four-plane-coordination-model.md)
- [ADR-0006：Adapter 能力必须以证据和契约测试声明](0006-adapter-capability-evidence.md)
- [ADR-0007：Windows Control Plane 与 Runner 执行能力分层](0007-windows-execution-profiles.md)
- [ADR-0008：允许选择性复用 Grok App 源码，禁止运行时接入](0008-grok-app-selective-reuse.md)
- [ADR-0009：通用内核实现主干与证据边界](0009-generic-kernel-implementation-spine.md)

M0 已于 2026-08-11 接受 ADR-0009 与总施工计划；权威记录见 `docs/plan/decisions/M0-2026-08-11-owner-authorization.md`。其他仍标为 Proposed 的 ADR 不因该决定自动改写状态。

当前评审规则：ADR-0008 已由产品所有者明确接受；ADR-0003 已被替代；其余 ADR 保持 `Proposed`，直到产品所有者明确接受。源码审计或支线程评审完成本身不等于产品决定。
