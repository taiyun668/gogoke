# GOGO PARTY 上游源码审计

- 审计日期：2026-08-11；参考车道补充：2026-08-20
- 审计方式：固定上游 commit，直接读取接口、状态机、持久化、隔离及测试源码；2026-08-20 两条车道为控制器核验只读锚点，reference-only
- 当前结论性质：设计输入，不是选型采购结论，也不是对任一上游的整体质量评价。`PRODUCT-PATTERN` / `SEMANTIC-PORT` 不是生产准入

## 报告索引

- [范围、固定版本与验证限制](00-scope-and-pins.md)
- [Gas City、Gas Town、Beads](01-gascity-gastown-beads.md)
- [Omnigent、Claudexor](02-omnigent-claudexor.md)
- [Mission Control](03-mission-control.md)
- [横向结论与设计校正矩阵](04-cross-project-findings.md)
- [扩展范围、固定版本与统一判尺](05-expanded-scope-pins-and-method.md)
- [Runtime、协议与 Adapter 源码候选](06-runtime-protocol-and-adapter-sources.md)
- [账本、工作流、角色与上下文源码](07-ledger-workflow-context-sources.md)
- [状态观测、面板与产品壳源码](08-observability-dashboard-and-shell-sources.md)
- [全部已看源码的提取矩阵](09-cross-repository-extraction-matrix.md)
- [EKKOLearnAI/hermes-studio 参考源码审计](10-hermes-studio.md)
- [deepseek-ai/deepseek-harness 参考源码审计](11-deepseek-harness.md)
- [参考负向测试矩阵](12-reference-negative-test-matrix.md)
- [补充审计：Grok App 源码复用与禁止运行时接入边界](../grok-app-reuse-audit.md)
- [综合结论：GOGO PARTY 上游复用蓝图](../reuse-blueprint.md)

## 一句话结论

没有一个现成项目已经实现 GOGO PARTY 的完整目标。最合理的借鉴方式不是把两个平台上下叠加，而是按能力拆取：

- Gas City：会话/任务分离、状态投影、协调循环、失败恢复；
- Beads：任务账本、依赖图、原子认领、租约与 CAS；
- Omnigent：真实多 Harness 接入、能力声明、Session/Fork/Policy；
- Claudexor：严格 Adapter 契约、类型化事件、上下文包和终态提交；
- Mission Control：产品面板、工作区视图、任务派发体验，但不能把它当前的 Framework Adapter 深度估计过高；
- Gas Town：角色、handoff 和 fleet UI 交互概念，运行状态实现仅适合作为反例和迁移参考；
- xAI Grok Build：host-agnostic lifecycle、ACP typed channel 和 ProcessScope；
- OpenAI Codex：官方 app-server 协议、跨平台 PTY 和 Windows Job；
- agtx、Squad、Open Harness、Clay：项目/角色/阶段/指令投影的实现样本；
- AgentPulse、AgentDeck、CliDeck：多主机、多 Session 与注意力面板，同时提供了不能进入权威层的并发和启发式反例；
- Hermes Studio：Dashboard、Workflow UX、Group Chat 与 Electron 打包为 `PRODUCT-PATTERN`，coding-agent 传输思想为 `SEMANTIC-PORT`；BSL 1.1 下无单独授权不得复制源码或直连 runtime；
- DeepSeek Harness：MIT developer preview，只作为未来 Harness/Adapter 目标；序号只进 `AdapterEvent.nativeSeq`，subagent finish 只进 `NativeTerminalEvidence`；不得替换 GOGO PARTY Control Plane、账本、lease、spool、Receipt/Gate、LoadProof 或 Transition Engine。

GOGO PARTY 的核心差异仍成立：仓库保存可审阅的成果、事实和任务投影；完整对话留在各 Harness；高频运行态和并发协调由独立控制平面承担。
