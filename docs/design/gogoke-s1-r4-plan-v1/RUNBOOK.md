# R：施工团队、命令、回滚和恢复

## R1 接包

只用Owner给出的固定PLAN_COMMIT读取本目录，先运行python verify_plan.py --root . --self-test。它是计划校验，不是G0。缺文件或摘要不符阻断，不能凭旧聊天补计划。parent/source/donor/Pi对象固定；确认实际workspace HEAD/dirty/untracked/未推送/活进程，保全现有修改，禁止reset/clean/按名称kill。不要把旧PR正文授权当新开工许可。

Owner发送本方案CODEX_START意味着仅限当前整段源码/文档/测试和Controller提交推送的启动授权。先从PLAN_COMMIT另建codex/gogoke-s1-r4-execution-r1（若已存在先核所有权/HEAD，不覆盖）。新增AUTHORIZATION_RECEIPT和INTAKE_RECEIPT；不改旧PR#6/#7及冻结分支。本方案分支不作为release合并目标。

## R2 团队与所有权

路由按固定docs/model-routing.md现有配置核实际可用：Sol/medium Controller；Luna/medium只读取证；Luna/high机械接线；construction Luna/max批量实现；Sol/high核心；fresh Sol/high聚焦复核；fresh Astra/xhigh首次/最终高风险全轴。模型名是期望路由，不证明runtime已加载；缺角色先排队/向Controller报告，不偷偷降级审查或另建Codex会话绕上限。Grok原生产品adapter与开发worker通道分开，未有工具资格不派Grok acceptance。

worker不commit/push、不改目标/合同/权限/验收，不自我验收。shared_write_scopes仅Controller改；初始donor全树导入只允许固定字节，不是授权全树任意改。tasks里范围不重叠且依赖就绪才并行；同文件同窗口唯一writer。标准报告保留原Task Capsule字段，追加plan/auth/source/build/test hashes、domain/custody和NEXT_ACTION。

需要复杂技术诊断但不改合同可在Codex做；同失败两次交Controller重排，不自动打断Owner。真正设计反证回GPT；环境缺失可BLOCKED，不伪装架构失败，也不靠反复运行耗尽资源。

## R3 必须实现的运行仪器

R4-Q0与S1-01-V实现tools/gogoke-s1-r4/run_checks.py，不是本包已有产品脚本。接受--plan-root、--group、--out；根据CHECKS的稳定ID及单个registry内精确selector，调用实际tests，不可只输出JSON表示已测。结果含候选SHA、构建产物摘要、fixture/plan/auth、OS、argv/cwd/exit、discovered/executed/pass/fail/skip、观察事实、日志和checks映射。0测试或test目标缺失为FAIL_INSTRUMENT；mock只满足其层次。checker实现先用一项真实失败反例证明没有空跑。

qualification只接受registry中真实已提交的selector、非零framework-owned机器结果、候选SHA、工具路径/hash、依赖根和平台状态。缺selector、0 test、skip或缺依赖为FAIL_INSTRUMENT；AppControl或缺真实平台为BLOCKED_PLATFORM。日志或exit 0本身不是PASS，必须解析框架机器结果；同一blocker identity去重，记录首个失败和触发条件后继续安全不依赖工作。

integrated candidate还必须证明正式protocol/session/TS入口实际到达同一native Product Authority，`prepare -> beginCommitted -> completion`无旁路，重启可由durable identity恢复，Outcome/Evaluation/Dream消费authoritative refs。primitive存在、单元测试绿或process-local cache都不能替代production reachability。结论前须机械核对源码、构建产物、运行进程与候选SHA为同一对象；运行host应报告实际版本、文件/hash或等价身份。

未来命令（创建并验明目标后执行）：
python tools/gogoke-s1-r4/run_checks.py --plan-root docs/design/gogoke-s1-r4-plan-v1 --group <group> --out artifacts/s1-r4/<group>.json

固定groups：qualification,sealing,codec,boundary,store,root,host,process,adapters,capabilities,delivery,continuation,events,policy,context,decision,evaluation,dream,vertical,upgrade。

donor目录：隔离Node24.13.1与pnpm11.10.0；pnpm install --frozen-lockfile --ignore-scripts后逐个审定必要构建生命周期再构建，使用锁内vp命令，不远程install pipe。参考执行pnpm exec vp run --filter t3 typecheck、pnpm exec vp test run <实际selector>；以donor实际脚本与审定的package边界为准，不把未存在命令记执行过。

native-host的构建、测试与平台证据均按长期治理文件[《gogoke 构建与发布架构》](../../governance/gogoke-build-and-release.md)执行；本计划不另定义本机原生执行路径。desktop现有npm run typecheck / lint / test保留，不全仓运行未知副作用suite；selected fixture重定向临时根和fake工具。一般测试能力不等于允许真实登录或产品安装。

## R4 九个高风险轴

身份/资格；根与持久化；进程/custody；投递/事件/continuation；权限/私域/egress；Context来源/lineage；Decision资源/幂等；Outcome/calibration/heldout；Dream抢占/发布/回滚。初次按同候选全轴读，尽可能一次收齐相互独立发现；修复后fresh聚焦+最新候选全轴。有效mutation只撤销真实实现修复且仍可编译，不改测试/注入语法错。外部Claude异构终验另由Owner通路，GPT/Codex不冒称已派。

## R5 旧资产与测试变化

KEEP：Wave A封存意图/布局/模型资产、产品unknown/custody语义、codec负例与失败日志。ADAPT：RPC/TS映射、声明式设置、来源检查、fake夹具、逐ID任务和能力映射。REPLACE_AS_IMPLEMENTATION：未获得对应新验收的custom journal、平行完整host/delivery、强制TS worker。OBSOLETE_AS_PLAN：旧计划必须自建的要求。实际文件处置必须在当前源字节上列keep/adapt/replace及等价检查，不能据标签批删。

唯一既有预期裁定：sealed模式SettingsView的Connect & add不得调用listWorkspaces或更新为已连接；保留配置显示/封存提示与原legacy mock模式回归。其它测试不能为新架构方便随意删除。T60仍证明运行资产/Node/许可/隔离，而非要求不存在的旧worker。

## R6 回滚与升级

产品运行数据使用新版本根，旧根只读保留。停止时先封新派发，drain/保管未决动作和native写者，保存receipt；不能直接跑旧程序读新库。donor mirror和每项patch独立提交，来源/修改理由/相关测试/维护者记patch ledger。先对固定donor及其直接父52d08a14b9e475371658fabbb12c056c3d3dad67演练patch重放/构建（只证明该次，不保证未来）。数据库backup/export/旧schema拒读用实验副本，不做用户迁移。

策略回滚切active pointer，数据/结论不抹去；Context所有derived历史保留受权追溯。出现真实网络/凭据/安装需求立即隔离该步骤，不把构建资格扩大成live许可。

## R7 高频恢复点

每完成一个有价值小批、3至5个小项或目标约3分钟可保存工作落MC；重大事实/反证即时落账，耗时命令前写intent，返回后写result。时间目标不是系统实时定时保证。只存事实、结果、文件和NEXT_ACTION，不保存模型隐藏推理。

每MC包含HEAD/diff/dirty归属、source/plan/auth/build hashes、命令/exit、失败/skip、活进程/custody、待派发/已提交/unknown动作。追加manifest和内容地址历史对象；latest用原子替换；大里程碑创建不可变archive。定期在临时副本还原并测试缺对象/篡改/路径越界拒绝。哈希不等于可信时间戳，sandbox也非永久备份。

证据层级保持分离：requirement、源码存在、测试存在、本次执行、fresh review、acceptance互不替代；WIP、candidate、accepted互不等同，review、adoption、Goal Acceptance、release互不等同。0 test、skip、BLOCKED都不是PASS；fresh review绑定最新exact SHA，候选源码变化后旧PASS不沿用。

checkpoint或中断不得留下无法定位的本地工作：源码优先按具名路径落`wip:`并push，且一律标注no tests/no review/not candidate；不可提交生成物登记绝对工作树、绝对路径和原因。worker不commit/push，Controller统一核对未推送、进程和生成物custody；禁止reset/clean或按名称杀进程恢复现场。ACCEPTANCE_UNKNOWN、ownerless WAL或存在写入活动时禁止blind resend。

完整stdout/stderr、机器结果和必要hash持久保存；对话只回命令、exit code、执行测试数、首个失败、候选SHA和日志路径，后续按片段读取，避免反复载入完整warning日志。

恢复先核远端/本地已提交和仍运行进程，再继续NEXT_ACTION。未知外部动作不重发。checkpoint不是Owner审批或平台切换。最终阶段结果含所有必需check状态、全部失败记录、独立审查、未执行轴和下一阶段；进入S2研究再回GPT。

施工中每60秒做一次活性核对，但不机械中断：存在活跃命令、具名diff、具体定位或可验证等待对象即可继续；连续3分钟无命令、无diff、无新结论且无可验证等待对象时回收。相同状态下同一命令最多原样重试一次，再次执行必须有代码、环境、签名或策略变化。完整日志落盘，对话只报命令、exit、测试数、首个失败和日志路径。正常checkpoint约每60至90分钟或一个有价值批次一次；中断、反证、权限变化或未提交custody即时记录。
