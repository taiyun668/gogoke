# C：Session、项目、全局上下文权威

## C1 对象与真实暴露边界

ContextObject不可变版本：id/version、scope(GLOBAL/PROJECT/SESSION)、project/owner/domain、kind、contentRef/hash、sourceRef/hash、sourceAuthority、derivedFrom、validity、supersedes、accessPolicyRevision。Scope不是许可；GLOBAL也可为Owner-private，绝不意味着每个项目能读。

ContextManifest不可变：task/seat/session/binding generation、policyRevision、sourceSnapshot、requiredConstraints、included版本/hash/选择理由、excluded reason codes、redactions、generation/decision refs、manifestHash。跨域exclude明细本身可能泄密，只给有权审计者看；普通调用者只获通用拒绝原因。

ExposureReceipt记录HOST_PREPARED/HOST_DELIVERED/NATIVE_ACKED/INHERITED/POSSIBLE/UNKNOWN。不把宿主发送的Manifest说成模型的完整脑内上下文；原生自动读文件/工具输出/插件/历史超出观测范围时保unknown。清洁资格要求对完整可达来源有实际控制证据；Jev不能把unknown洗成clean。

SessionLineage记录productSession/nativeBinding关系与resume/fork/rebuild/handoff父边。Fork继承所有暴露标签；NEW_CLEAN无父原生history，显式选择材料重新装配。跨域重建不是让模型忘记旧内容；封旧sourceEpoch、旧会话不得输出到新公开域。

## C2 装配与失效

本地policy先过滤，再metadata/FTS检索候选，mandatoryConstraints强制加入且不可被模型筛掉；来源不可用/必需材料超预算则NEEDS_EVIDENCE/NEEDS_BUDGET，不默默截断。S1用SQLite FTS/元数据索引，不要求新embedding服务；索引只保存可见引用并在检索前分域。

生成席将授权GenerationView生成ContextDraft；Jev只用独立获权DecisionView评价相关/充分/复用等。每一步外发重新核EgressGrant；两个模型不直接互传完整私密材料。引用必须可解析到不可变源对象，摘要是DERIVED_UNVERIFIED直到逐项核对，不把推断升级事实。hash核对证明身份，不证明摘要语义正确。

提交Manifest前比较关联task/policy/source/binding版本；不因无关UI变动失效，但受影响材料变化必须重装配。装配与派发串接同一个ActionAdmission；网络生成不持DB事务。撤权后缓存、索引、派发、replay都重查；旧证据保留受控审计访问，不继续给已撤权会话发资料。

Context生命周期ACTIVE/SUPERSEDED/CONFLICTED/STALE/REVOKED/ARCHIVED，状态是单独版本化索引，不修改原内容。源码SHA/hash/时间比较由代码做；语义冲突只能生成待核事实，不凭Jev定事实真伪。内容库可按域salt/密钥隔离，跨域去重不暴露存在性。

## C3 Session策略

可行动作REUSE/RESUME/NATIVE_FORK/NEW_CLEAN/REBUILD/HANDOFF/ARCHIVE/WAIT。合法候选先由域、独立性、原生能力、custody、已暴露标签过滤；Jev选择适配度不能恢复被排除项。公开委托审查不得复用Owner私聊；盲审不得复用作者会话。native idle不足以结束未决action。

ARCHIVE仅停止默认索引/新任务，不表示进程退出、不自动删除。需要关闭进程走A4独立StopProof。新建席位与新建原生会话不等价；席位身份长期，绑定按域和generation替换。

## C4 项目/全局知识提升

项目事实提升为GLOBAL创建新版本候选，有原始来源、适用条件、反例、confidence components及promotion grant。Gemini可以概括，Jev可以建议PROJECT_ONLY/GLOBAL_LESSON/PREFERENCE/RULE/NEEDS_EVIDENCE/REJECT；它们都不能单独赋予跨域分享权。

默认只生成候选；在已明确scope/kind/destination的可委托promotion policy下才可自动采用低风险项目内派生知识。Owner偏好、治理规则、跨项目私密经验不从单次讨论自动提升；需Owner或既定强授权。跨项目借用也核目标获得许可；脱敏不等于自动公开。

过期/推翻关联沿derivedFrom传播到派生索引，保留历史；manifest引用旧版本时标状态并按任务要求拒绝/补证。数据删除与保留冲突独立治理；Dream无删除原件权。

## C5 Antigravity/Gemini + Jev

ContextSteward/DreamResearcher是职责，可用同一已准入Antigravity实例的不同受控Session；Gemini为实际可用ModelRef。上下文生成席不自行验收自己的摘要。独立语义评估使用新材料最小化Session或异构评估；同账号不同Session不等于异构，也不自动独立。主要标签来自真实下游检查，语义评分有来源可靠性，不当唯一真值。

S1只注册disabled配方和fake生成适配，走真实服务调度及schema校验。后台模型不可用时使用固定来源Manifest/规则装配或WAIT；停止、持久化、项目打开不依赖模型。不是改走Gemini API或Gemini CLI；真实认证形态与商业政策由GN取证，不沿用此前未经核实的账户资格断言。
