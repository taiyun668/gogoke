> **历史稿，非现行规则。**本文件 2026-09-19 起草，曾在私有仓库 gogo-party 的施工线（S1 起）上使用，未进入其 main。2026-09-23 按 Owner 决定随补迁移原样保留作参考；现行的 GPT 主控与施工协作以 `docs/governance/GOGOKE_GPT_MASTER_CONTROL_HANDOFF_STANDARD.md` 为准。

# gogoke 连续施工阶段包模板 V1

**TEMPLATE_ONLY / NOT_DISPATCHABLE**。本文件没有赋予任何生产、测试副作用、commit/push 或合并权限；不得以文件存在标记阶段已就绪。

使用规则：`gogoke-gpt-codex-stage-routing-v1.md`。实际方案由 GPT 完成并固定提交，Codex 只执行已批准范围。一个阶段包可以包含多个 WP、PR 和任务卡，不要求每个子任务后切回 GPT。

## A. 身份与授权

- PHASE_ID / 可交付阶段目标 / 包状态：待 GPT 填写。
- 仓库、生产输入 commit、证据 commit、计划 commit：待固定；生成文件不得预填自己的未知 SHA。
- Owner 目标与范围授权的准确引用：待明确。
- 允许工作包 / 允许路径 / 禁止路径 / 非目标：必须逐项填写。
- 允许测试 / 进程 / 文件根 / 网络 / 凭据 / 迁移 / 安装等副作用：各自列明；未明确即未授权。
- Controller 集成 / commit / push 权限，以及 PR merge 权限：分别登记，不由 worker 权限推导。

## B. 已定方案与依赖

- 公共对象、接口、版本与兼容、错误 / unknown / 部分成功语义。
- owner/root/writer、隐私域、授权、进程身份 / 后代 / 旧 writer、日志与数据保全合同。
- 采用 / 内化 / 替换 / 保留 / 不采用的具体文件、符号及传递副作用。
- 所有纳入 WP 的依赖 DAG、可提前准备的任务与禁止提前集成的路径。
- 允许的实现弹性和预先裁定的 fallback；改变合同必须回 GPT。
- 阻断决策清单必须为空才能就绪；非阻断未知须有 owner、期限和不能作出的假设。

## C. 团队与任务卡

每张任务卡必须填满以下字段：

| 字段 | 要求 |
|---|---|
| TASK_ID / WP / 所属阶段 | 一个稳定标识，不把任务卡当平台切换点 |
| Executor | Luna/high、construction Luna/max、Sol/high 等已核配置；研究类任务留 GPT |
| Reviewer | fresh Sol/high；高风险首次 / 最终全轴 Astra/xhigh；不得同一实现者自验 |
| Input | 实际存在的固定依赖产物与源码对象 |
| Allowed changes | 准确文件 / 符号 / 写入所有权；热点文件唯一集成人 |
| Acceptance | 行为断言、实际命令、输出证据和失败 / skip 分类，不只给“测试通过” |
| Dependencies / parallel group | 写入不重叠、运行依赖可满足才允许并行 |
| Stop / escalate | 合同内错误先回 Codex Controller；需研究或越界时回 GPT |
| Deliverables | 实际 diff、受控产物、命令回执与未执行轴；worker 不自行 commit/push |

容量表登记：配置声明的路由、实际会话确认、工具可用性、有效限制、可用槽位、串行降级顺序。未知容量不虚构数量；Grok 没有符合任务的实际工具与授权时不是关键路径施工者，不通过外建会话绕上限。

## D. 测试、验收与恢复

- 保留原计划 C/T/WP 双向映射、首验截止和 L/N1/I/M 等层次；不能以阶段分组延迟检查。
- 区分静态检查、固定源码假依赖、未来合同模型、真实受控行为、真实产品 / 平台验证。
- 固定正例、负例、回归、适用的 production mutation；不删现有测试或改预期消除错误。
- 高风险轴一次收集同候选的完整问题批次；修复后聚焦复核及最新候选最终全轴审查。
- 回滚条件、保留原件、退出 / 残留 custody、未授权操作的停止点与恢复条件。
- 独立复核者、阶段结束条件、最终 Owner / 外部 Claude 义务；技术认可与执行授权分开。

## E. 移交准入

GPT 在派发前核对：方案已定、对象固定、边界完整、团队可执行、热点有所有者、验证可运行、风险预案明确、授权覆盖、无阻断决策。任一不满足即 NOT_READY，不把空模板交给 Codex 边做边研究。

Codex 接收时核对：实际工作区 / dirty tree / 输入 SHA、模型与角色、权限与容量、测试依赖和允许文件。仅核接包，不重做总体方案。普通环境失败先由 Controller 处理，不能自动安装、升级或扩大权限。

## F. 连续执行与回到 GPT

- 连续施工：剩余任务已有计划、依赖满足、权限覆盖，Codex 继续，直到完整阶段或批准停点。
- 完成后下一项仍是获准施工：可继续 Codex，不强制平台往返。
- 完成后下一项是研究 / 方案 / 阶段决策：固定交接后转 GPT，不在 Codex 私自开始研究。
- 重大边界阻断：暂停相关路径，立即提交可复现证据及待决选择；安全独立任务可继续。
- 一般实现问题 / 非阻断疑问：集中修复或登记，不按每个 bug 切平台。

最终交接保留原项目回执字段，并包含：

```text
PHASE_ID:
PLAN_COMMIT:
AUTHORIZATION_REF:
ACTUAL_HEAD:
COMPLETED_AND_REMAINING:
VALIDATION_AND_FAILURES:
INDEPENDENT_REVIEW:
UNEXECUTED_AXES:
ROLLBACK_AND_PRESERVATION:
DECISIONS_REQUIRED:
NEXT_WORK_KIND: IMPLEMENTATION | RESEARCH | DESIGN | PHASE_DECISION | WAIT_AUTHORIZATION
NEXT_PLATFORM: CODEX | GPT | OWNER_DECISION
```

不要把模板中的备选状态当作实际状态；由真实结果填写。平台移交不宣称自动开启另一会话，Owner 不应承担各 worker 之间的微任务搬运。
