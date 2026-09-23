# Gogoke S1-R4 修订版：受控实机纵向闭环

**状态：DESIGN_PROPOSAL / NOT_AUTHORIZED。** 设计基线：`taiyun668/gogoke@e79dc718b98eef4a36ec88ee4d7ac43d63e0ca7a`；施工停点：MC-035。本稿只修订本轮范围，不恢复施工、不宣称产品已可用。

## 1. 输入、目标与生效

以 main 已合入的 [事实账本决定](../../governance/gogoke-ledger-decision.md)、[构建与发布治理](../../governance/gogoke-build-and-release.md) 及根 [AGENTS.md](../../../AGENTS.md) 为准，分别包含 PR #14、#15、#16。Owner 本轮要求及其追溯在 `PLAN.json.requirements`；五个施工包与八个验收检查也只在该文件定义。

**目标：Owner 从正式安装的 Gogoke 产品入口，完成一条实际服务可达的受控工作链，并在 Windows 11 智能应用控制强制模式下验证。** 具体样例是：在 Owner 选定的测试项目中，读取指定、无敏感内容的版本化材料，委派一个受控 Worker 生成可核验的报告，呈现结果与变更，形成 Git/GitHub 账本记录，并能在重启后恢复已接受记录。

本轮以已有 rules/fake/replay DecisionProvider、现有 Pi adapter 配受控协议测试进程完成可重复任务，不冒称真实模型的开放目标自治或全部 CLI 资格已经完成。Tauri UI、实际服务和 native-host 不能用测试替身替换。交互面与内核始终是同一个 Gogoke，不是“临时壳”和另一个终局产品。

生效顺序：**计划 PR 合入 main → 另开只修改 `artifacts/s1-r4/intake/PUBLIC_AUTHORIZATION_RECEIPT.json` 的授权 PR → Owner 本人合并 → 核验授权后恢复施工。** 本计划不包含授权文件。现有 MC-035 STOP 不因计划编写、计划自检或计划合并自动解除。

新授权沿用 `gogoke.s1-r4.public-authorization.v1`；保留真实 provenance/carryover，`plan.public_plan_path` 指向 `docs/design/gogoke-s1-r4-plan-v2/`。在计划合入后，用 `git rev-parse origin/main:docs/design/gogoke-s1-r4-plan-v2/MANIFEST.json` 读取**实际 Git blob OID**填入 `public_plan_manifest_blob`，不能填 SHA-256、计划 commit 或本稿中的预估值。核对授权文件引入 PR 的实际 changed files、base=main、merged_by=taiyun668、合并后的文件字节和当前 MANIFEST；不以文件内的 owner 字符串自证。

本 PR 同时将 v1 的 PLAN 和 MANIFEST 改为历史入口。v1 原件在上述固定基线可取回；v1 MANIFEST 字节改变也使旧授权的 blob 绑定失效。旧实现与测试不在本设计 PR 中删除。

## 2. 产品架构：事实账本与活跃协调

**PRODUCT SEMANTICS / PRODUCT ARCHITECTURE**

| 职责 | 唯一归属与最小实现 |
|---|---|
| 已接受的成果、Context/Decision/Outcome/Evaluation/Dream 记录及审阅证据 | Git/GitHub。以仓库、不可变 commit/path/blob 和必要的 PR/CI 身份引用；数据库仅保存引用或可重建缓存。 |
| 草稿、候选和在途任务 | 本地工作状态。可写工作分支；提交草稿不等于采用。实际接受以所指向的账本操作与其授权/合并人为准。 |
| 当前 grant/admission、lease、claim、幂等、投递、Session、NativeBinding、进程托管、消费位点 | 原有唯一协调域。保留 Route B 的 Rust/SQLite 原子协调与原生 custody，Node 仅通过 typed operations 使用；不另建数据库或 scheduler。 |
| UI、T3-derived 服务、native-host | UI 发产品请求并显示事实/状态；T3-derived 服务是唯一产品编排入口；native-host 执行原生句柄与协调事务职责，不再拥有已接受认知事实的独立历史权威。 |

事实引用最少能定位 repository、commit、path、内容 hash；采用/验收还必须能定位 PR、合并 commit 和合并人。复用 Git/GitHub 现有版本、差异、审阅、合并和 CI，不再造一套本地 event/receipt 体系来证明同一件“已接受事实”。当前协调所需的 event/receipt 则保留。

同一记录按状态分工，不按名称一刀切：Decision 的在途选择与容量租约是协调；已接受决定进入 Git。Action completion 的可信运行观察先是协调证据，下游记录以 exact refs 进入账本；写数据库本身不构成采用。Session 运行绑定保留在协调层，已接受的可恢复上下文按 Git 版本引用。

Git 接受记录不自动激活运行权限；实际 admission 仍须来自经认证的 Owner/Controller 控制与版本化授权依据。协调库重建不能据旧记录复活已撤销许可或给遗留进程补造 custody。

授权来源与活跃撤权也分开：持久授权依据引用已授权的产品记录；当前 admission、限额和撤权由同一协调域执行。Controller 默认在自身权限包络内委派，Worker 可收窄不可扩大。不得把上下文专用 `GrantSpec` 重解释成委派权限。入队及真实 I/O 前重查当前 grant、撤权、Task/Manifest、Seat/Runtime/Model 与 binding generation；缓存、pipe 认证或模型文字均不能代替这些检查。

### Git/GitHub 边界不是事务捷径

Git 与本地数据库没有跨系统原子提交。本轮复用现有 operation identity 与幂等协调：保留待提交引用 → 写入指定工作分支/提交 → 从实际 Git/GitHub 读取结果 → 更新协调引用。失败只核对远端 exact ref/PR/commit，不凭超时伪造采用、不强推覆盖并发变更、不在未知情况下盲重做外部动作。并发分支头变化应显示冲突或重新规划。

结果、审阅、采用、Goal Acceptance、Release 保持分开。PR 合并只具有该 PR 明示的业务含义；采用一个结果不自动宣称 Goal Acceptance 或发布。建设项目的“Owner 合并授权文件”也不能变成未来产品所有动作都要 Owner 微审批的规则。

产品账本指向 Owner 为项目指定且授权的仓库，不自动指向公开源码仓。首次验证用无敏感 fixture；真实 GitHub 写入及结果 PR 只能发生在 Owner 授权的测试项目/分支范围。源域读权、目标共享权、选中材料与撤权在提交前重查。私聊、原始日志、账户/路径信息不因成为“证据”而获准公开。已有有效授权可持续使用，不每个普通任务重新索取许可。

### 恢复边界

正常重启从协调记录恢复在途任务，从 Git/GitHub 恢复已接受事实。协调库丢失/损坏时，先保全现场与原生进程 custody，再重建已接受事实的引用/缓存；不能由 Git 推断丢失的在途发送记录，也不能从 lease 过期推断旧写者退出。无法证明完成的已承诺动作保留 `ACCEPTANCE_UNKNOWN`，不盲重发。

`prepare → beginCommitted → completion` 不变；beginCommitted 位于实际最低不可逆 I/O 点，不是 SDK 缓冲 write 的名字。EOF、exit 0、Promise 完成或 Worker 自述不是 completion。不能暴露该边界的 Runtime 路径不能靠降低保证进入本轮闭环。

Context 仍为一级对象，GLOBAL/PROJECT/SESSION 与隐私域不合并；Task 当前版本决定必需 Context。Seat、RuntimeDriver、RuntimeInstance、ModelRef、Account/Profile、NativeBinding、Session 分离，公开 SPI 不封闭为厂商枚举。Jev 是 DecisionProvider，Gemini 是 ModelRef，Antigravity 是 RuntimeDriver。

## 3. 最短施工路径

**CONSTRUCTION PROGRAM GOVERNANCE**

| 包 | 完成后实际可观察的增量 |
|---|---|
| R2-01 实际入口与权威分工 | Tauri 产品请求进入实际 T3-derived 服务的 Gogoke composition，经过已准入调用上下文到 native-host；同一产品入口显示一个测试 Goal 和其账本引用。不以孤立导出函数或单元测试冒充接入。 |
| R2-02 一条任务与账本闭环 | 在授权内选 Context、Seat、Runtime、Recipe，完成受控任务；Result/Objective Outcome 绑定可信观察，生成一次 Evaluation 和隔离 Dream 提案；结果/提案按权限进入指定 Git 工作分支，实际审阅采用后回读接受事实。Dream 不自动激活。 |
| R2-03 负向闭合与一套检查入口 | 对已接入的同一条路径验证拒绝、未知、恢复、隐私和开放类型；复用现有测试与 runner 必要部分。CI 与允许的本机脚本共享测试定义和机器结果，不另建平行资格账本。 |
| R2-04 云端构建与完整随包 | 正式包包含 native-host、实际 T3-derived 服务 bundle 及其所需运行时/资源；使用现有 Tauri/NSIS 与更新链。离开源码工作树也能由产品入口受管启动。 |
| R2-05 冻结候选复核与 Win11 验收 | 独立复核精确候选；Owner 用同一候选的正式安装包在 Win11 强制模式下完成闭环、重启和结果回读。按实际证据收口，不以云端 Windows Server 结论替代。 |

表中的 checks 是责任与最终证据映射：上游包先验证自己的可观察增量，不能要求它预先完成下游完整场景，也不能把部分子断言写成整项 PASS；V01–V08 最后在同一候选完整结算。

顺序默认按表执行。每包都是产品可观察增量，不为每个字段开工作包。入口组合、共享 authority 拼接、候选切割与集成由 Construction Controller 持有；确有独立收益的无交叉写域才委派。Master Control 只写本设计与阶段裁定，不竞争生产写入。

先复用当前 `gogoke/bootstrap/index.ts::constructGogokeService`、`nativeStoreService.ts` 和 `cognitionService.ts` 的正确接口及测试。实际 composition root、Tauri 路由及 bundle 入口由 Controller 在接包时定向确认；必须落入回执的精确文件，不允许只加 import 让静态搜索看起来可达。与 Route B 重复的 Node 权威实现，仅在同职责被实际新路径替代并有回归证据后收掉。

## 4. 随包路径与 Owner 实机

复用 `.github/workflows/gogoke-desktop.yml` 的桌面产物链，绑定同一 candidate、锁定依赖/工具链、目标架构与 hash。当前 Cargo 清单声明的产品目标是 `gogoke-native-host`，另一个 `gogoke-root-lock-probe` 是测试工具，不能拿它冒充产品 host。计划的制品路径为：

`apps/desktop/native-host/Cargo.toml --bin gogoke-native-host --release --target x86_64-pc-windows-msvc` 云端构建/测试 → `apps/desktop/src-tauri/binaries/gogoke-native-host-x86_64-pc-windows-msvc.exe` 暂存 → `tauri.conf.json` 的 `bundle.externalBin` 引用 `binaries/gogoke-native-host` → NSIS 正式安装包 → 安装后以产品解析的安装资源路径受管启动 `gogoke-native-host.exe`。

T3 服务复用 `third_party/t3code/apps/server/package.json` 的 `build:bundle`/`dist/bin.mjs` 产物，而不是新造一个只服务于测试的“主服务”。其 `src/bin.ts` 实际入口应进入 Gogoke 本地构造图，在构造前封住未授权 T3 产品服务器/探针；不能先启动再隐藏。计划将服务 dist、必要动态依赖和 runtime 暂存到 `apps/desktop/src-tauri/resources/gogoke-service/`，通过 Tauri resources 入包。安装后 bundle、runtime 与 native-host 均由安装资源解析，不依赖当前工作目录或开发目录。这些是待施工路径，不声称目前已存在或已验证。

同时交付实际服务 bundle 和运行它所需的 Node 运行时；优先复用现有锁定 runtime 资产，不要求 Owner 装开发依赖、手工复制 exe、启动开发服务器或从工作树启动服务。若现有资产不完整，只补这条闭环所需的 runtime、资源与许可材料。产品入口建立进程托管和私有 IPC 后才准入任务；不能绕 native-host 直接启动无托管工作。

**不做可执行文件代码签名，不新增 OV/EV/SignPath 申请或签名流水线。** 沿用未签名安装包与 Owner 离线签署的 `SHA256SUMS.windows`；私钥不进代码、CI 或制品。Owner 的清单签名证明发布完整性，不证明 Smart App Control 会放行。更新公钥、允许主机、协调器信任根仍为编译期常量。正式安装包验证不自动授权公开 release。

native-host、Node/runtime/helper 都是独立子进程资产，应逐项记录来源、版本/hash 与强制模式下的实际启动结果；主程序通过不能外推给子进程，旧版本通过不能外推给新版本。被拦截时记录 Code Integrity/进程现象与精确制品，交 Owner 决定；不自行签名、关安全设置或改用本机原生编译绕过。

原生编译和测试一律云端；各 CI job 有 timeout，同分支新推送取消旧运行。**不限制原生流水线只能在候选触发，不恢复“云端故障即不重试”的政策。** Controller 依据当前故障处理 CI；未成功执行的检查如实记未完成，不用本机原生结果顶替，不代 Owner 操作付款或账户设置。

## 5. 验收和旧检查去向

本轮仅有 `PLAN.json.checks` 的 V01–V08；全部初始为 NOT_RUN。PLAN.json 的任务/检查状态是设计初值，实际进度写 checkpoint 与运行证据，不为报告进度反复改 MANIFEST。一条检查可以调用多项已有行为测试，但每项必须可追踪实际运行，不能把包装命令 exit 0、跳过或零测试写成 PASS。检验完成条件是必要观察成立，不是达到一个测试数量。

需绑定 candidate/source、构建与安装包 hash、CI run/job、实际执行/失败/跳过、平台及被测产品入口。记录 native-host/服务运行身份，排除旧二进制或 fixture 替代真实服务。独立复核按现有风险轴检查当前可达范围；保留 fresh、修复复核、最终异构审查与 Owner 最终裁决的分工，不把计划自检当产品或审查通过。

旧 v1 的 32 张任务在 `PLAN.json.legacy_task_disposition` 逐 ID 映射为本轮五包，替代其**施工排程**，不因排程退役删除源码；逐类资产判定在 `PLAN.json.assets`。旧 59 项 due check 分别列在 `legacy_due_disposition`，全保留 MC-035 时未晋升的历史状态。ADAPT_CURRENT 仅说明本轮相关保证由何处验证，不代表旧检查全部断言已通过；R4-24 纵向链明确保留并改线到 Git；R4-06/12/16 的接受事实语义改线，必要安全断言保留；真实 Jev、完整统计校准/holdout、跨项目提升与在线策略发布等明确延期，具体逐 ID 写明。旧 157 截止中的其他 124 项仍按固定源文件保留为本轮范围外，不安排未来工作包或伪造日期。

v2 完成表示**修订范围的受控实机纵向闭环**完成，不表示 v1 全 59 检查、所有 Runtime、本产品全部 UI 或 S2/S3 完成。旧 G0 PASS 与 G1/G2/G3/G4 未接受、G5 NOT_RUN、GN NOT_AUTHORIZED 保留在历史停点；不得因修订映射自动改成 PASS。新的收口回执必须明确引用 v2 MANIFEST 和实际 V01–V08 结果。

## 6. 延期与恢复纪律

本轮不新铺全部 provider 原生资格、真实 Jev/Antigravity/Gemini、18 个策略全量启用、跨项目 GLOBAL promotion、在线 Dream 自动优化、remote/mobile/voice、真实生产数据迁移或新插件系统。这些产品能力没有被取消；本轮没有它们也能验证指定闭环，故不建立未来任务/测试矩阵。现有安全隔离与封存入口的回归继续保留。

PR #13 保留 WIP，NB-F1 与 provisional NB-F3 不抹掉；需要复用的协调身份代码先按当前字节重判并修复相关缺陷，不再为“数据库即已接受事实”的退役语义继续堆历史索引。PR #4 只提取本轮所需授权核验、机器结果解析及负例；旧完整 59 项 registry 作为历史，不盲目合入。PR #10 的必要随包 notices 随 R2-04 适配。其他 lineage/native-close WIP 按 assets 判定，不盲目整枝合并。

GPT Construction 窗口一个有价值小闭环就留恢复点；关键定位、修改/测试结果和耗时操作前后及时保全，不套用长回合等待大 checkpoint。控制面先读 HEAD/最新恢复点/索引，证据按具体文件和片段展开，不拉整仓大 diff、完整大日志或几十个历史 MC。恢复点只写事实、分支/HEAD、未完成成果归属、证据位置、未决动作与 NEXT_ACTION，不存隐藏推理；恢复先读 durable 状态，不盲重发。

源码 WIP 在授权内由 Controller 保全并推送，不能只留聊天或临时沙箱；WIP/candidate/accepted 分清。普通 bug、测试和 codec 等留施工方收敛；真正改变冻结产品语义或无法满足保证才回 Master Control/Owner。checkpoint 不构成审批门。

任务结束清理自己开出的工作树、依赖、构建产物与临时副本；确需保留的，在检查点写明理由。公开提交不含秘密或本机身份/绝对路径。遵守当前 AGENTS，不增写额外卫生清单或未经 Owner 决定的频率/重试禁令。

## 7. 自检

`python docs/design/gogoke-s1-r4-plan-v2/verify_plan.py --root docs/design/gogoke-s1-r4-plan-v2 --self-test`

验证 MANIFEST、需求追溯、依赖、旧 due-check 完整去向及证据层级负例。输出只证明计划结构和完整性；不证明代码、云端、Owner 实机或独立复核通过。完整原始记录按固定 Git 坐标读取，不把本稿的归类当作代码删除审计。
