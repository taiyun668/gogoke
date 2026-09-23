# GOGO PARTY 正式设计入口

- 版本：`v0.7-p31-d3-p17-namespace-handoff`
- 状态：M0/M1 基线已接受；P31 D1 产品契约草案已经独立 D2 fresh audit，并因缺失 P17 Project/worktree namespace handoff 非 PASS；本目录现含唯一 D3 consolidated repair `P31-D3-P17-01`，等待新的独立 fresh audit 与 D4 Owner freeze
- 范围：CLI Harness；Web Chat Agent 仅保留未来接口边界
- P31 批次：`P31-D3-P17-CR-20260822-01` 修复 `P31-D0D1-20260822-01`；D0 结论 `PRODUCT_CONTRACT_GAPS_INVENTORIED`；D1 结论 `PRODUCT_CONTRACT_DRAFT_CONSOLIDATED`；D3 结论 `P17_NAMESPACE_HANDOFF_CONSOLIDATED`；本目录条文在 Owner `ACCEPTED` 前不得称为已冻结或实现授权

## 1. 文档权威顺序

施工操作系统的当前权威按以下顺序处理：

1. `packages/room` 与 `packages/seat-runtime` 的实际运行时（配套 `START_HERE_CONTROLLER.md` §3–§7）
2. `35-construction-os.md`：产品身份、边界与 PR 设计
3. `GOGO-施工计划.md`：当前施工顺序、修正与每个 PR 的合入门
4. `36-construction-os-ui.md`：界面设计，受施工计划约束

施工计划是当前执行裁决：涉及施工顺序、修正和合入门时，施工计划覆盖 35 与 36 的旧表述；35 仍是产品身份与边界的设计依据。`00-product-charter.md`–`08-mvp-acceptance.md` 是历史设计输入，不是当前施工线的运行时或施工权威。其余文档按各自被 35 和施工计划引用的范围阅读。源码审计报告是设计证据，不是运行时规范；上游项目只提供可借鉴机制，不直接决定 GOGO PARTY 的语义。

## 2. 阅读顺序

进入产品施工前，先阅读 `../plan/00-master-construction-plan.md`；它定义里程碑、并行车道、关键路径和施工授权 Gate。以下文档是其技术输入，不替代总施工计划。

1. `00-product-charter.md`：为什么做、做什么、不做什么
2. `01-domain-model.md`：共享语言和不变量
3. `02-system-architecture.md`：四平面与组件边界
4. `04-workflow-event-protocol.md`：任务、认领、命令、回执和自动接力
5. `03-repository-protocol.md`：Git 中保存什么以及如何进入系统
6. `05-harness-adapter-contract.md`：如何接入不同 CLI Harness
7. `06-context-session-governance.md`：提示词、上下文和换窗
8. `07-isolation-security.md`：项目、Agent、Runner 和 Windows 边界
9. `08-mvp-acceptance.md`：第一个可证明闭环
10. `09-runner-protocol.md`：Control Plane 与本地/远端执行面的可靠边界
11. `10-artifact-evidence-retention.md`：产物、证据、日志和删除生命周期
12. `11-source-audit-corrections.md`：每项源码审计发现落到了哪里
13. `../adr/0009-generic-kernel-implementation-spine.md`：实现主干、唯一 Supervisor、Adapter 证据边界与第一条纵向切片
14. `12-schema-first-protocol.md`：v1alpha1 中立对象、摘要、Extension 与 Schema Gate
15. `13-sqlite-coordination-spec.md`：Assignment CAS、lease/fencing、Receipt、outbox 与 commit reconciliation
16. `14-fake-conformance-vertical-slice.md`：确定性 Fake Runner/Adapter、故障脚本、Oracle 与纵向 Gate
17. `15-v1alpha1-machine-contract-review.md`：机器 Schema、Wire、正反例、摘要向量与本轮总审结论
18. `16-sqlite-machine-contract-review.md`：Project 复合键、Assignment CAS/fence、Receipt/outbox 与 commit reconciliation 的机器总审
19. `17-fake-kernel-executable-subset-review.md`：54/54 内存参考执行、SQLite authority bridge、Oracle 与未证明边界
20. `18-project-context-room.md`：进入 Project 后的卡片式 Context Workspace、Context Ledger、结构化 mention/reply 与纳入上下文边界
21. `19-execution-target-model-routing.md`：Harness、模型、Profile、凭据、Host、Provider readiness 与禁止静默替换
22. `20-local-control-host-lifecycle.md`：Windows 后台 Control Host、首次启动、关闭/退出、通知和备份恢复
23. `21-cross-project-resource-arbitration.md`：跨 Project 共享 Host/Runner/Provider 容量的全局仲裁、公平和 fenced permit
24. `../../spec/draft/v1alpha1/README.md`：机器可读草案、非声明边界与一键只读校验
25. `../research/adapter-spike/README.md`：公众版 Codex + Grok Build 协议与能力实测
26. `../research/grok-app-reuse-audit.md`：Grok App 源码复用与禁止运行时接入边界
27. `../research/reuse-blueprint.md`：所有已审计上游如何组合成 GOGO PARTY 自有实现
28. `../research/source-audit/09-cross-repository-extraction-matrix.md`：全部已看源码按同一判尺得到的复用/拒绝矩阵

## 3. 当前设计基线

系统明确分为四个状态平面：

- Canonical Fact Plane：已接受、可审计、可进入后续上下文的项目事实与成果
- Coordination Plane：任务状态、依赖、认领、租约、命令、幂等和消费位点
- Live Observation Plane：心跳、进程、PTY、流游标、用量估计等可重建观察
- Native Session Plane：Harness 私有会话、transcript、checkpoint 和本地缓存

Git Control Repository 是可审阅的事实与协议账本，但不是低延迟协调数据库。协调状态必须在本地持久存储中事务化；通过 outbox、仓库投影和 reconciliation 与 Git 对齐。

## 4. 评审纪律

- 文档为候选并不等于实现授权。
- ADR 在产品所有者确认前保持 `Proposed`。
- 能力必须由探测证据和 Conformance Test 支撑，不得由适配器名称推断。
- 任何 `PASS`、`RUNNING` 或 `COMPLETED` 都必须可追溯到结构化证据。
- `UNKNOWN` 和 `INCONCLUSIVE` 不得自动降级为成功。

## 5. 已确认的产品决定

- 2026-08-11：MVP 首发 Adapter 组合确定为 **Codex + Grok CLI**。这冻结的是 Spike 对象，不预先宣称两者已经满足能力契约；必须以当前版本实测决定各能力等级。
- 2026-08-11：Grok Adapter 的目标必须是公众可下载的官方 Grok Build CLI。用户私有 Provider、账号池或内部 shim 不得成为产品依赖、默认入口或验收前提。
- 2026-08-11：允许在 MIT License 边界内选择性复用 `RongleCat/grok-app` 源码，但 Grok App 是非官方社区产品，禁止把其 App、进程、RPC、IPC、store 或原生 Session 接入 GOGO PARTY。该决定记录为 ADR-0008。
- 2026-08-11：公众版 Spike 已证明 Codex app-server 与 Grok ACP 可作为首发主通道；两端 headless JSON 只作 one-shot 降级。结论是允许进入 Adapter 实现与 Conformance，不等于生产 PASS。

## 6. P31 D1 产品契约草案与 D3 P17 namespace handoff（非授权）

以下条目是同一产品契约草案，含 D1 十一类收口与唯一 D3 repair `P31-D3-P17-01`。它们必须经 `P31 DESIGN -> fresh product audit -> one consolidated P17 repair -> fresh product audit -> Owner ACCEPTED` 绑定同一 contract-manifest digest 后才能称为已确认或已冻结；在此之前不得作为 P32 或实现授权：

- 全局 Command Center 只做跨 Project 运营总览；用户从 `Projects` 选择 Project 后，Project Overview 默认进入该项目的卡片式 Context Workspace。
- MVP 一个 Project 恰好一个 Source Repository；Control Repository 默认同仓 `.gogo/`。后继多仓的每个 source revision 必须绑定 `repository_binding_id`。
- `ContextEntry` 与 `CollaborationProposal` 是唯一协作权威对象；卡片/mention/reply 只是投影或 Intent。`ReactionSignal` 不进入 MVP 权威面，只允许 `DISPLAY_ONLY`。
- Context Compiler 只消费冻结的 `ContextCandidateSet/SelectionBasis`；MVP 提供 Project 内结构化筛选，不提供全文检索 UI；Reveal Gate 前不得用命中数/摘要泄漏其他 Auditor 内容。
- `operator_id` 不等于 OS principal；Control API 必须本地认证 origin/CSRF/IPC/owner token。危险动作需要 exact scope、actor、expected revision、幂等键、风险预览与显式确认。
- MVP Harness 旅程是 detection + manual official guide；检测回执不是登录成功。一键官方安装器不在 MVP，若未来需要则新开工作包。
- 每个 Attempt 必须绑定精确 ExecutionTarget；Harness、模型、Profile、CredentialHandle、Host 或协议等级不得静默替换。已有副作用时的 fallback 必须新 Attempt 并经人工 Gate。
- Desktop UI 只是 Local Control Host 客户端；`Close window | Pause | Stop | Quit | Force terminate` 互不等价。Host 是当前 OS principal 的 per-user 后台进程，不是 Windows Service。
- portable backup 默认加密并由用户 passphrase/recovery key 托管；secret/登录态/native Session 默认排除。DPAPI-only 包为 Host-bound。卸载默认保留数据。
- 共享 Host/Runner/Provider/Profile 容量必须经 ResourceBroker 原子仲裁。等权双 Project 的 starvation bound 为最多连续旁路 2 次 grant decision。
- Chief of Staff 只作为 post-MVP proposal-only 能力，不得具有隐藏 authority。
- MVP 必须证明同一 Workflow 的 Codex/Grok 自动接力，以及两个 Project 同时存在活跃 Attempt 时的完整隔离。
- P17 是唯一 Project/worktree namespace owner，不得自行推断该产品语义。`Project -> SourceRepository/repository_binding_id` 对 Attempt 不可变；canonical namespace 为已注册 runner root 下的 `project_id/assignment_id/attempt_id`；worktree ownership/lease 绑定当前 Controller epoch/fence 与 Runner/process identity。每次 source write 前必须完成全部 binding/fence 校验；cross-Project、cross-Attempt、stale 与 path-escape 一律 zero-write reject。dirty workspace 只进入 Attention/human Gate，不得 auto-clean、auto-reuse 或 fallback。P16/P33/P18/P22 只消费 P17，不得另建 namespace authority。MVP 负向验收覆盖两个 active Project、name collision/path traversal、stale Attempt 与 dirty workspace 的 fail-closed/no-write。

完整对象合同、旅程、十一类收口与 P17 namespace handoff 见 `docs/plan/evidence/p31-project-context-product-inventory.md`。文档字节摘要见 `docs/plan/evidence/p31-project-context-contract-manifest.json`。

## 7. Owner / audit 仍未完成的事项

M0 的 repository topology、默认自动化上限、Windows 首发拓扑与 ADR-0009 主干已经由 Owner 接受。P31 D1 已对十一类产品决定给出单一草案，D3 已把 P17 Project/worktree namespace 显式交给 P17，但下列事项仍不是本批次权威：

1. 对同一修复后 contract-manifest digest 的新独立 fresh product audit 给出 `PASS | FAIL | INCONCLUSIVE`。D2 已对修复前 digest 给出 `FAIL`（axis 13 / P17 handoff）；本 repair 不是第二次 audit，也不得开启第二 repair。
2. D4 Owner Decision `ACCEPTED | REJECTED | REVISION_REQUIRED`。
3. P32 v1alpha2 wire/Schema 与生成类型。
4. 任何内核、Store、API、UI、Adapter、安装器、真实 Harness 或 P17 实现。

P31 只形成产品契约草案；新增 wire/Schema 和生成类型必须经 P32 的 v1alpha2 兼容 Gate，不能回写 v1alpha1。repair 后再非 PASS 进入产品 architecture replan。
