# Codex 单次接包：S1 连续施工

**当前：NOT_DISPATCHED / AWAITING_OWNER_AUTHORIZATION。** 本文是待授权的执行指令；现在没有创建Codex会话或生产分支。不能从Owner“继续推进方案”推断其已授权产品修改。

## 接包入口

以发布回执中的本计划完整SHA读取本目录 PLAN.md、CONTRACTS.md、TASKS.json、TEST_MATRIX.json、SOURCE_MANIFEST.json。生产输入固定bc665a852833952b76d9508401193bedd2198436；计划父a06619719bbac73a892d9e7b27b911cac77d8aa8仅为文档基线。D技术就绪已记录，P00接受/实际授权仍需Owner。

Owner授权记录应同时标明：接受P00施工前结论；固定计划SHA；允许的WP（完整S1或明确子集）；允许产品路径；允许临时根/受控假子进程/私有本地IPC/构建与测试；Controller提交推送范围；明确不允许合并、真实账号/费用/用户迁移/外网listener/麦克风/安装发布。授权可以一次覆盖整个S1，不要求每个task再次询问。记录中的真实SHA/工作目录由接包时绑定，不预造未来构建身份。

若只批准WP10a/WP01，完成波次A后停止新生产修改并请求范围授权，后续B/C方案已在包内，不重开研究。未批准任何生产WP则仅报告接包准备，不写生产、不跑可能产生副作用的产品测试。

## 授权后的连续指令

你是Codex施工Controller，不负责重新研究本阶段架构。

1. 核对实际git状态、dirty文件和现有分支；不得reset/clean用户更改。核源码与固定基线是否有语义漂移；文档新增不等于生产漂移。含额外产品改动时先隔离差异，不混入旧基线自称通过。
2. 核对角色和可用工具。使用项目既有Sol/Luna/Astra路由；construction worker不commit/push。未验证实时slots不要创建假想并行席位；容量不足排队，不外建线程绕限。
3. 以批准计划形成一个S1集成分支和必要的隔离worktree，分支名由Controller避免冲突后登记。任务执行依TASKS DAG，只有共享文件所有者做集成。每个产物固定commit/hash并运行当期检查；开发测试可提前编写，不能提前验收未来产物。
4. 一般编译、接线、测试失败在Codex解决；批量缺陷集中修复，fresh Sol聚焦，再对最新候选作所需Astra全轴。固定审计角色只读生产工作区；有效mutation在Owner允许的专属副本由有写权限的测试执行角色运行，审计者核实际字节、仪器和重复结果。不为mutation绕过角色/沙箱限制；无法有效执行则保留缺项，不写PASS。
5. 全阶段保持source/fixture/model/受控真实OS/真实provider证据分层。原P00假件不能替代新实现测试；不存在的新测试命令要先由其任务实现，零用例/skip不能算完成。
6. 按PLAN三波次继续，依赖与授权已满足时不因换WP/PR返回GPT。必须改变合同/权限/owner/root/writer/采用范围时才暂停相关路径，并提交固定证据给GPT，不自己重新设计；安全独立任务可继续。
7. 阶段最后固定六个P产物、33个S1截止检查（原主计划总157个检查全部保留）、独立审查、实际演示、回滚/保管与未执行N1/I/M矩阵。所有数字仅为范围账本，不是默认通过数。
8. 下一工作若是S2研究/方案，整理一次性阶段交接回GPT；若后继施工已有完整方案与明确授权，可继续Codex。不得把阶段研究派给planner角色留在Codex。

## 最小阶段回执

```text
PHASE_ID: S1
PLAN_COMMIT: 实际固定计划SHA
AUTHORIZATION_REF: Owner明确范围记录
SOURCE_BASIS: bc665a852833952b76d9508401193bedd2198436
ACTUAL_HEAD: 实际最终提交
PRODUCTS: 每个P的commit/hash及完成状态
VALIDATION: 每checkId的命令、artifact/build、退出码、fail/skip及原始日志
FAILURES: 已保留的失败、修订和剩余问题
INDEPENDENT_REVIEW: 初始/聚焦/最终候选及fresh审查来源
UNEXECUTED_AXES: 原N1/I/M/真实提供方/平台/迁移和任何被阻断L/H
RESIDUAL_CUSTODY: 未确认身份、writer与恢复责任
DEVIATIONS_FROM_PLAN: 范围/实现差异及是否获准
NEXT_WORK_KIND: RESEARCH | DESIGN | PHASE_DECISION | IMPLEMENTATION | WAIT_AUTHORIZATION
NEXT_PLATFORM: GPT | CODEX | OWNER_DECISION
```

不要把上述实填字段的提示文字抄成执行结果。S1完成不代表五家真实功能完成、全产品上线或PR允许合并；后续授权和验收义务不减少。
