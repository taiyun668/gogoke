# Gogoke S1-R3 连续施工计划

## 目标

在保留 Wave A 有价值成果的前提下，以 T3-derived 单一主运行服务替代旧 S1 的大面积自建，实现：

- P10a remote/voice 保留式封存
- P01 provider-neutral 公共合同
- P02 SQLite 持久权威与恢复
- P03 唯一 host / process / profile custody
- P09a 五家 capability observation
- P04 安全 delivery / continuation / event / delegation 纵向链

## 施工输入

- Gogoke：88ef8e7dfbf5ba5aef58743dc45fa660f946276e
- donor：pingdotgg/t3code@d6f291303ddc0c9a14f570266a4d9eff6d431593
- 旧 S1 任务/检查账本：34b7f891e4e17715b08491570740516d6cf7f49f

旧账本的 19 tasks、68 主测试、157 检查截止、33 S1 当期检查及原证据 ID 必须逐项映射保全；只是实现对象改变，不得通过删除测试义务“完成改线”。

## 分段

### Segment I — G0/G1：资格、边界和持久基础

1. 固定 donor source/lock/license/SBOM；确认未运行其安装/update/postinstall 副作用。
2. 独立 Node/pnpm 工具目录，不升级 Gogoke 全局/desktop 工具链。
3. 最小编译 T3 server/provider/orchestration 所需闭包。
4. 建 provider process/spawn 注入 seam，使 fake executable 可替代真实 CLI。
5. 冻结 remote/mobile/http/update/auto-install 等默认入口。
6. 建 Gogoke public contract ↔ donor contract mapping。
7. 建产品 root、单实例 writer fence、SQLite product migration。
8. 建 action/event/receipt/continuation/custody 最小表和事务。

### Segment II — G2：provider / process / capability

9. 接 Codex driver/instance。
10. 接 Claude driver/instance。
11. 接 Grok driver/instance。
12. 接 OpenCode driver/instance。
13. 接 Antigravity driver/instance。
14. 所有运行路径统一走 process/custody seam；SDK 内部隐藏 spawn 的路径必须显式审计并可注入/监管。
15. Windows 受控 fake CLI 做 suspended→Job→resume、descendant、host crash、registration failure、PID reuse/custody 测试。
16. 建 capability snapshot：binary/version/protocol/profile/auth revision/domain/generation/source。
17. 未验证能力 = unknown；无自动 login/download/update。

### Segment III — G3/G4：delivery、权限与纵向演示

18. action durable intent → reactor dispatch → acceptance observation。
19. ACCEPTANCE_UNKNOWN/restart/no-blind-resend。
20. continuation approval/question：回答/取消竞态、restart stale callback。
21. per-domain event cursor、gap recovery、slow consumer。
22. Owner/private audit/public review/secretary domain 负测。
23. 最小 delegation：主控在 delegable grant 内派 worker，结果不自升 Owner 权限。
24. fake provider 纵向演示：Tauri→private IPC→service→fake native→event/result。
25. duplicate operationId 同内容仅执行一次；同 id 异 payload 拒绝。
26. stop proof 与 residual custody。
27. 独立 fresh review / 高风险最新候选全轴审查。
28. 固定最终回执；不 merge。

## 旧 19 task 映射原则

WP10a、WP01 原 Wave A 保留任务继续作为输入资产。
WP02 从 custom root/journal 改为 SQLite product authority + migration/root lock。
WP03 从自建 daemon/worker 改为 T3-derived host + thin custody seam。
WP09a 改为复用 driver/instance/probe，再投影 Gogoke 三态能力。
WP04 改为复用 orchestration/event/receipt 主体并补 durable dispatch、continuation、privacy policy。

Controller 在开工第一提交生成 MACHINE_TASK_MAP.json，把旧 19 task ID 一一映射到上述施工项；缺任何旧 task ID 为 G0 FAIL。

## 团队路由

沿用 docs/model-routing.md：
- Controller：Sol/medium
- 探索/证据：Luna/medium
- 简单机械施工：Luna/high
- 大批明确施工：construction Luna/max
- 核心 DB/process/permission：Sol/high
- fresh 聚焦复核：Sol/high
- 高风险首次及最终全轴：risk_auditor Astra/xhigh

worker 不 commit/push。共享 manifest、lock、provider registry、root state、最终集成由 Controller 写。

## 连续推进规则

G0 之后依赖满足的任务连续推进，不按 WP 回 Owner。普通 bug、编译和测试失败留 Codex 集中修复。

只有必须改变主内核、事实权威、公开合同、安全/隐私、迁移边界或产品能力时，停止受影响路径回 GPT。

## 结束条件

- 六产品都有最新 SHA 实现证据；
- 33 个 S1 当期检查全部实际 PASS 或准确 BLOCKED（不能 skip→pass）；
- 旧 68/157/84 义务映射完整；
- 一次受控 Windows/fake native 纵向演示；
- 最新候选独立全轴审查；
- 真实未执行轴、失败、残留 custody 和回滚路线明确；
- PR 保持未合并。
