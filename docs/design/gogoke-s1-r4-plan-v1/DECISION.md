# D：主控自主决策、Jev接入与18场景

## D1 受委托的选择权

Gogoke项目主控可在Owner授予的职责/工具/数据/资源预算内复用或创建临时Seat、选ExecutionRecipe、调整依赖就绪任务顺序、请求允许的观察；不是每一步交Owner确认。建设Gogoke的Codex Controller仍只实现GPT冻结方案，不能把产品自主策略当开发代理扩大scope的理由。

DecisionBackend SPI区分Rules/Fake/Replay/Jev/Generative。Jev是typed判断后端，不是RuntimeDriver，不拥有运行事实。Generative后端若使用Gemini，经Antigravity配方执行，不能自动开另一计费通路。任何backend返回WAIT/NONE/NEEDS_EVIDENCE/NEEDS_REASONING均合法，不能强制选差候选。

## D2 生命周期与事务

状态REQUESTED/EVALUATING/VALIDATED/COMMITTED/STALE/REJECTED/ABSTAINED/FAILED/CANCELLED。DecisionRecord包含family、stateViewHash、candidateHash、question/rubric版本、model requested/resolved、source/task/policy/capability revisions、nativeConfidence/probabilities、calibrationRef、选择/规则原因、模式、budget、deadline、actionId。网络请求在事务外。

先由EligibilityBuilder按授权/能力资格/工具/隔离/独立性/容量过滤，再semantic排序；task、runtime、model、account、tool、context组成ExecutionRecipe。代码保留公平和等待，不能三个任务各自argmax后抢同一资源。S1用可解释有界贪心：先hard constraints、priority class、等待时间，再qualified semantic rank和估计成本；数值未知不当0，不能精确预算靠Jev猜。

commit比较taskRevision/policyRevision/candidateHash/capabilityRevision/bindingGeneration，事务内重查余额/容量/写入所有权；DecisionApplied+容量预约+现有action intent原子提交。提交后唯一码回执防双发；迟到回复/取消不复活旧action。Jev超时不能取消确定性stop。停用backend可降低优化质量，不能让safe工作或数据恢复瘫痪。

## D3 可配置，不把18个用例变成18套系统

DECISION_FAMILIES.json登记18个设计方向。主要族RESOURCE_SELECTION/SESSION_LIFECYCLE/CONTEXT_SELECTION/MEMORY_LIFECYCLE/RISK_CLASSIFICATION/ESCALATION/ATTENTION/OPTIMIZATION。序号仅沟通目录；其中下游评价/Dream是闭环机制，不能让Jev自己给自己当客观标签。

每场景定义ViewSchema/Questions/CandidateBuilder/ActionCeiling/Scorer/Calibration/DreamPolicy。新增普通场景改注册定义和测试，不增加另一个调度器。新增新权限或新外部副作用必须走新的授权合同，不以配置绕开。

S1允许在fixture里证明bounded_auto，但真实Jev资格为空、external budget=0、egress disabled；生产不得把合成校准profile带入线上。最低演示场景为配方选择、Session生命周期、上下文相关性和项目内记忆候选。其他场景注册并有契约拒绝/回放接口，不声称全部策略可用。

## D4 Jev传输与输出

参考模型jev-1.13.0，版本改变使原资格失效，记录实际resolved model；不使用latest取得已校准资格。输入state/questions为预定义任务视图，Choice/Score/Noul语义分开。Choice必须含NONE/WAIT出口；Noul没有独立confidence字段。请求key名不被当指令，问题明确引用字段；数学/计数/日期/阈值计算在代码。

使用受控服务HTTP客户端或已审SDK注入fetch。S1 transport注入fake，无真实models/systemone调用；disabled状态在构造和请求前都拒绝，不能读环境Key自动启用。真实端点未来固定allowlist，不跟随到新域的重定向。

返回运行时校验model、问题集合、primitive、合法候选、finite概率/范围/和、Score等级与legend、缺失/额外结果、body/内容类型上限。TypeScript as T不是校验。body原文debug禁用，日志仅hash/耗时/token/错误码；本地保留内容也按域授权。

外层总deadline默认2s（交互）/10s（后台），默认maxRetries=0，单次结果绑定版本；这些是初始工程上限，不是供应商SLA或最终生产阈值。可按预登记预算调整，不允许SDK默认10s×多次重试拖住控制链。网络失败返回ABSTAIN/规则fallback，不转发到未经授权的替代云端。真实API可能计费的超时记录unknown usage，不能当免费。

## D5 Egress与授权

ReadGrant与EgressGrant分离；Jev影子也外发，缓存命中也核权限。每次View绑定destination/model/purpose/materialClasses/domain/retentionPolicy/budget。不存在grant则在序列化body前拒绝。不可把多个私域batch成一次请求，不将秘密或token放state；脱敏过程本身也不得未经许可先发给另一个模型。

规则不让模型扩大职责、取消独立review、修改安全策略、删测试、启用真实费用/账号或发布。未知预算/权限/能力一律不eligible。可按既定policy自主处理普通事项；只有跨该上限才需要Owner。
