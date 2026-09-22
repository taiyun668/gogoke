# E：下游评分、校准与空闲“做梦”

## E1 不再用一个confidence冒充一切

保存raw probabilities/nativeConfidence、校准与样本量/区间、contextCoverage、candidateCoverage、freshness、scorer可靠性、OOD状态分别展示。policyEligible是硬布尔守卫，不是可与模型高分相互抵消的概率分量。禁止简单平均成万能总分；缺关键输入无论confidence多少均拒绝自动提交。

Jev返回分布集中程度不是独立审查。Brier用于带真实标签的概率，ECE只作分桶辅助，Score用等级分布评估；不要把nativeConfidence直接当某次正确率，也不要将多问题概率相乘称联合保证。校准键含decisionFamily+modelVersion+question/view/criteria版本+候选生成版+语言/任务域+runtime环境。

## E2 下游才给判断反馈

OutcomeRecord append-only，链接decision/action/任务/Manifest/实际配方；保存accepted artifact、tests、fresh review、返工、换Session/重解释、Owner override及原因、耗时、费用、permission/privacy事件。迟到的失败或复审可以追加修正revision，不覆盖原记录。

标签类别OBJECTIVE/INDEPENDENT_SEMANTIC/OWNER_OVERRIDE/SELF_REPORT及来源证据。SELF_REPORT永不成为唯一通过标签；独立评估给事实和引用，不索取隐藏思维链。无反馈=PENDING或CENSORED，不计成功也不计失败；基础设施故障单列，不能全部扣到Jev。未选候选没有反事实结果，不能从单臂日志宣称最优。

每决策族保留多维结果（质量、返工、延时、成本、上下文遗漏）；安全与隐私先作为硬排除，不能用成本节省抵消。评分rubric在看最终留出结果前固定，不让Dream优化reward定义来“提高分数”。

## E3 数据与资格

数据按project/time/session-lineage/近重复cluster分组，先划development、calibration、sealed final holdout。候选提出器只能看dev错误；不能把holdout逐行反馈或反复测试的排名传回提示优化器。正式holdout使用预算，耗尽后新独立数据；同项目大量相似条目不虚增样本。

域内先过滤数据，合成与真实标签分开。跨项目/跨Owner归并必须显式授权，不默认共享私聊/源码。离线评分结果可能泄漏敏感存在性，权限同样覆盖数据清单/指标/日志。

S1实现报告和资格守卫，不预设“100例/0.9就是可靠”。缺已审批QualificationProfile（loss定义、风险上限、CI/样本计划、baseline、heldout hash、locale/model）时真实backend最多shadow或提议，不能ACTIVE。真实阈值由GN的事前协议确定；测试阈值明确test_only不可上线。候选集合变化/新模型/新语言失效旧资格。

## E4 Dream是同一调度器的低优先级维护任务

不是额外常驻自主agent服务。已有scheduler队列kind=maintenance，触发条件：无前台关键工作、无紧急待续接、无未决危险custody、预算/维护窗口/数据许可全有、近期样本达到配置门槛。默认真实调用预算0；合成fixture可显式启用。并发最多1，同账号foreground预留优先，不自动借其它账号额度。

工程默认idle120s、每run最多20步/300s、控制面收到foreground后2s内停止新优化派发；已发模型请求取消/状态核对异步收尾，不谎称2s内所有进程已停。前台优先，deadline/费用/token上限和租约写回执。restart后从已提交step恢复，不盲重放unknown外部调用。

Dream使用只读快照或授权派生分析库；不能争用产品writer、在旧snapshot上覆盖新事实。提出新memory索引/摘要、Jev View/Question/Candidate规则、调用批次/缓存/阈值候选，不执行任意生成代码，不改生产代码、原始证据、Grant、审查要求、测试预期或scorer规则。

## E5 提议与发布

DreamProposal记录inputSnapshot、dataset split、optimizer recipe/model、family、before/after schema/hash、收益/风险证据、版本依赖、回滚点。生成席（Antigravity承载Gemini）提出，Jev可参与语义选择；下游scorer与确定性guard评估，不以Gemini/Jev自评落锤，不改变Jev模型权重。

状态DRAFT→DEV_VALIDATED→CALIBRATED→HOLDOUT_VALIDATED→SHADOW→CANARY→ACTIVE；有REJECTED/RETIRED/REVOKED分支。S1真实命名空间止于候选/离线回放，不启动真实Jev shadow、canary或active。fake namespace可演示全状态转移，要求test_only标志不可被删除后迁入prod。

激活必须策略发布权限、独立验证、依赖版本仍匹配、compare-and-swap activeRevision原子提交；在途decision固定旧policy，后续用新policy，禁止解释一半用新rubric。撤权、预算失控、安全事件、已定义质量退化自动停用相应family并回退，不把产品全部停掉。回滚不删除真实历史，只切策略指针。

## E6 S1收敛范围

做到Outcome入库→分族评分→校准报告→快照Dream→候选→独立留出/污染拒绝→fake shadow/回滚演示。18场景元数据完整，但先验证上下文/Session/配方链；不开“夜里随意研究一切”的代理。自动优化不等于自动扩大Owner授权。真实收益、中文质量、长期稳定性均须GN实测。
