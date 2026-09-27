# R2-06a / e1379bb5 fresh 最终风险复核

TASK：按 Controller 的限定任务，对生产源码 `e1379bb518a7012d90da970e4407f95f5e98efaa`、检查点头 `a4eb4b9dcc4a2c5939e64c389e8212dec57e66f4` 做 fresh、独立、只读 R1–R8 审计。只新增本报告；不替代外部 Claude 或 Owner acceptance。

RESULT：**本次覆盖范围内未发现新的已证实 P1/P2。R2-06a 候选构建、用途绑定签名、云端实装、一次 Win11 受控产品调用及卸载证据成立；可供 Controller 收口本包并继续已授权的 R2-06b。完整 R2-06 PASS、正式发行、R2-05 验收均不成立。** 本机产品外层 v2 脚本仍是 FAIL；它之前生成的真实产品回执单独成立，不能改称整个脚本通过。

CURRENT_STATE：审计开始及写报告前工作区均干净；`e1379bb5..a4eb4b9d` 只有 MC-154 至 MC-158 五个检查点，没有源码、测试或 workflow 差异。有效规则为 `origin/main@1ba8f6092a7e86ab2ca2c5d2e58259c57f1d2d31:AGENTS.md`（PR #51），并阅读模型路由、风险角色、构建发布规则、施工规则、账本决定、R2-06 设计、MC-151..158、b025 最终审计及后续聚焦审计。检查点中的旧观察不自动升级成当前事实。

FILES_CHANGED：仅 `artifacts/s1-r4/reviews/R2-06a-e137-final-risk-audit.md`。本席位未提交、push、运行产品、修改登记、清理对象、进行本机原生构建或触碰秘密。Controller 允许的补充下载保存在独立的本机 `r206a-e137-final-audit` 证据目录，等待随交接保全；原始 `r206a-win11-e137` 文件未改。

IMPORTANT_DIFF：实际审查 `b025ba44..e1379bb5`。生产变化是安装/卸载的包与重定向视图拒绝、非 quiet 交互指引，以及 `58deb795` 的卸载登记锁 Windows 错误传递。父进程只接受 nonce 精确匹配、长度受限的忙码或 u32 错误码；未知输入仍拒绝。`58deb795..e1379bb5` 撤掉目录重解析诊断文件及 workflow job，保留错误码修复和两条实装断言。有效 Windows 配置覆盖基础配置，固定 shell 版本为 `0.0.0`、前端为 `bootstrap-error`、NSIS 为 `currentUser`，模板据此使用 `RequestExecutionLevel user`；产品版本和前端来自已验资源。

## 证据身份与仪器核验

四条运行均由审计席位直接只读查询 GitHub API/run/jobs，均为 attempt 1、completed/success：

| 对象 | 本次确认的精确来源 |
| --- | --- |
| [桌面 CI](https://github.com/taiyun668/gogoke/actions/runs/36346338781) | `e1379bb5`，五个 job 成功；独立 frozen/repro Windows 车道及逐字节比较。 |
| [源码卫生](https://github.com/taiyun668/gogoke/actions/runs/36350513194) | `a4eb4b9d`，11 个检查器测试；24,584 个已提交文件，0 leaks。尚不覆盖本报告提交后的新树。 |
| [受信候选签名](https://github.com/taiyun668/gogoke/actions/runs/36348752709) | `workflow_run`，`main@1ba8f609`，实际下载源 run `36346338781` 的 frozen artifact。 |
| [候选实装烟测](https://github.com/taiyun668/gogoke/actions/runs/36348848011) | 执行 workflow revision `3b72065404571b14a8b8114feafc79fd6007422b`；源码/测试/workflow 与 `e1379bb5` 相同，正负回执均绑定 exact source/run/artifact。 |

以下九个原始 ZIP 的长度及 SHA-256 均由本席位重算，并与直接读取的 GitHub artifact metadata 相同。额外下载只增加本机审计副本，不改变远端。

| Artifact ID | 作用 | ZIP 字节数 | ZIP SHA-256 |
| --- | --- | ---: | --- |
| 10941447930 | frozen | 122249255 | `dbc536a49b2b2184c8ff6c751284e37fdb4fcadec11ac3cd58c2d4b34c14d39c` |
| 10941681179 | repro | 122243858 | `e2d631fcd5ecb934d96b9954cef2a2e7c975227af0b52280463162fde769f6c4` |
| 10941572157 | 六类字节比较 | 850 | `9f106c7e95ae5bc494b639edc00cfee71958e964b7b38bfab89c9f1f0761f666` |
| 10941686363 | 已签候选资源集合 | 113174259 | `c67679fbf6d0e46d72f788290ca7b77cedf5a0f467dcfabf28ae1a7fd283ddea` |
| 10941696190 | CI 专用负测安装器 | 55728999 | `75e45bd7dc12e11718cca865f9272ed5919e4ef88ecd27d0a0dcc09ce5cd1e96` |
| 10940507731 | frozen Job custody | 472 | `e1c2104b7d01d49632bb046a857c987c4c8acce986ed64e21ef8fac1f6cb33fc` |
| 10941111922 | repro Job custody | 471 | `9ced10546e34d8c0d3e3105f6515d025661037eb9617ca347b0e67695fadda77` |
| 10941699053 | 实装正向回执 | 1319 | `be6ee971213b5031bfdf21a98653691b5c1ecf9546c2db1597c0d2f2675576c4` |
| 10941708876 | 安装并发/拒绝回执 | 1183 | `10a1ac25df24e6b3cef6880ba3f5d75b9b25091f502d5cf6181fae8839feaf82` |

独立核算 frozen 清单的八个文件、signed 集合的七个共用文件，以及 frozen/repro 六类长度和 hash，均相同。比较器源码实际比较两份完整字节；setup 不属于六类可复现声明。冻结 setup 为 `9fef81a010dd2a51f994512e9fde2351fd5a6362f997e014177d70b513d5dd3d`，安装版 shell 为 `2b5ef742bfcae5a3f686545da0edf4039a4e159594e5d1c01573c4f6d888faad`，portable shell 为 `e723e919f4705a69ab8301f975d8a01bd2aebcfee74cf0ee4a8e8ed8cf4fde21`；没有用 portable 代替安装版身份。

候选 P-256 签名由独立 .NET API 验证成功；相同签名去掉用途前缀、或在内存中改单字节，均被拒绝。这是验签负控，不是 production mutation。清单绑定 `e1379bb5`、run `36346338781`、attempt 1、artifact `10941447930`、版本 `0.1.3`、index `135e442aa0b5138d0425bcc0e9bffade010d268f34a21d85529603283c8632ce`、pack/generation `6cba8e86062c65a85b52ae84bd954c3286f2dc45b2c114d695df6c78347e9fc5`。pack 的 1,234 条目的名字、唯一性、长度、hash 逐项符合索引；普通视图实装快照的 12,979 个静态文件及 1,234 个资源文件也逐项相符。此签名是资源授权签名，不是 Authenticode 代码签名。

本席位检查生产安装版 shell，找到完整 CRLF finalizer 字节恰好一份，未发现 CI 删除暂停标记。负测安装器 hash `688065e82031ae943e4f9d6cc415e35a0ac995a3182a9f231fd5cc8ba8292ffc` 与冻结 setup 不同，且与 `instrument.json`、实际负测回执相符。负测的包状态/重定向值只在 CI 安装器注入；最终登记删除并发使用生产 finalizer 的内存副本插入一次暂停，未替换锁代码或测试预期。路径、stage marker、实例、精确 shell 和非零退出断言都被实际执行；不把这些 fixture 称作两个真实已安装产品的完整重装实验。

## 独立风险轴

| 轴 | 源码与实际行为核验 | 本次结论/边界 |
| --- | --- | --- |
| R1 字节与来源 | 双独立干净车道；固定源码/工具链；原始 shell、NSIS 处理后 shell、host、Node、pack、index 六类相等。实装版本/来源取签名索引。 | 当前源的可复现证据成立。专门只改版本/资源再重建的实验本次未执行，不能以同源双构建替代。 |
| R2 资源信任与实际加载 | 编译期 Owner/候选公钥、互斥 manifest、用途前缀、安装域与实例登记、同一打开文件验 hash 后供给、RuntimeLease、服务模块限制。真实安装 WebView URL/ready/setId 绑定本次 index。云端污染模块被服务以 78 拒绝，无执行标记及 readiness。 | 候选路径观察成立。候选 JS 的同账户 I/O 不是 OS 沙箱；本次不扩大到全部 native 入口权限/namespace 或正式发行运行的安全声明。 |
| R3 安装/登记并发与视图 | 同域全局登记锁与父目录生命周期锁；上下文拒绝在产品变更前。云端 leaf 冲突退出 2、原文件保留；不同父目录 B 在 A 持锁时退出 2；finalizer 登记删除窗口内 B 被阻止，A 完成并释放后 B 成功。 | 这些具体并发/拒绝路径通过。实际 MSIX/重定向进程运行、交互提示前台可见性，在 e137 上未实测；两个 CI 上下文分支只证明注入后的生产拒绝路径。完整更新/重装交错仍属 R2-06b。 |
| R4 进程、句柄与请求 custody | 读取真实 Job 创建/挂载、取消后 owner、lease/gate 保留代码，核双车道 hard-exit 和 held-file 回执。root/child 属于 exact Job；子进程实际持文件；owner 返回后文件及目录一次删除成功。 | 既定 C07 窄标准成立。frozen 的 child wait=0，repro=258；258 不证明当时 child handle 已终止。fixture 的后续收尾没有先于已记录的一次文件删除，不把它冒充更强的所有进程同步退出保证。 |
| R5 卸载与数据/正式对象保留 | 真实 shell `--uninstall --quiet`、继承锁见证、逐文件 hash/对象身份、实例重核、登记删除期间持域锁；生产 finalizer 不递归删除未知树。 | 云端新增忙码与 Win32 5 两个分支都实际通过：登记和 shell 不变、无提前 receipt。云端最终 DELETED；本机最终 DELETED、候选文件 0、数据逐字节保留、正式四类对象不变。保留空目录并非删除失败或用户数据丢失。 |
| R6 CI/签名权限与公开卫生 | 直接回读 main signer workflow 和 `candidate-resource-signing` environment；当前唯一 custom branch policy 为 main，无逐次 reviewer。签名前先验 exact source run/jobs、artifact digest、完整 inventory，随后只签资源清单，不执行来源源码。所有相关 job 有 timeout/concurrency。 | 当前签名边界及本次 run 成立。没有接触 secret/private key。卫生只覆盖其精确提交；本报告需后续提交树卫生。 |
| R7 产品入口与账本语义 | 云端和本机均实际加载 Home/Tauri，调用 `gogoke_r2_goal_probe`，原生 Controller admitted、host reachable，固定 Git blob `a20115fdd5acf9e7e5025c3b3ca50696001badac` 回读、Result/Outcome/Evaluation/Dream 测试结果成立。 | 一次受控调用为 `VALIDATED_TEST_RESULT_NOT_ADOPTED` / `TEST_FIXTURE_NOT_ADOPTED`、adoption=false。本次没有证明新的受管账本写入、真实 adoption、完整用户操作流程或持续 UI 生命周期。 |
| R8 Win11/SAC、遗留与最终裁决 | 普通 Interactive/Limited 流程的原始脚本和七份同 SID/session、packageStatus=15700 快照；OS `10.0.26200.0`，SAC=1，CI RecordID=7097；安装后 exact shell/host/Node、ready、卸载结果。 | 这些确切字节在本次 Win11 候选操作中的平台证据成立；不外推下次字节或正式域。Owner 离线签名、正式域实装/资源更新和最终 Claude/Owner 裁决仍未完成。 |

## 本机测量错误与产品结果

前提核对：第一版 `run-product.ps1` 把 readiness 放在证据目录、名为 `product.ready`；实际 `ready_path_from_args` 要求进程 temp 根中的 `gogoke-update-*.ready`。因此第一版没有满足被测协议，不能用其超时证明产品失败。第一版结果保留为 INVALID_MEASUREMENT。其 raw JSON SHA-256 为 `576cd91ad9d9239f9b6ed4ddcd072562182d095d4dcb8f0cc3bacd8ca94c6b33`。

第二版使用符合协议的 temp 路径。直接探针在实际 WebView 中验证 Home、Tauri、ready 三字段及一次受控任务全部断言后才以 CreateNew 写出 `PASS_TEST_ONLY`，原件 SHA-256 `74c1022b14d6f846029a7961ebd1c04fcab4df633682ff98870f10d04041fee4`。本席位直接读取实际 176 字节 readiness，SHA-256 `3ddef42fba7fe556d907725a90241a5cc1d293464f8617226785079efb5c4715`，身份相符。

外层脚本随后获取 product PID 失败；原件 SHA-256 `121217a6ff282799b34c3d7705614ad682d55188071d26491be54762ab4763d3`，仍记 FAIL。推理边界：它发生于真实产品回执之后，不推翻已完成的调用，但现存证据没有说明 PID 为什么退出，故退出原因 UNKNOWN，也不证明该脚本执行了正常关窗。没有以第三次启动覆盖失败。随后独立快照验证进程为零与正式状态未变。

本机真实卸载父进程退出 0；本席位按实装 instance hash 定位并读取原始 198 字节 finalizer receipt，SHA-256 `781b9a2e95eb3c457d6a713b58e5443a2dc12796de99042219df30e591c6cc84`，域为 `CI_CANDIDATE_RESOURCE`、状态 DELETED。卸载后快照 SHA-256 `0130dafd3f40959b4b1255d8d11eee4d5851fcc825110e1b77a8e3589611d4c8`：候选登记消失、文件数 0、1,087 个空目录，候选数据树 hash `03289673ba559d4cbed0a47cb928024890cd47a5b82ccc0194a136f9124ad5fe` 与卸载前相同。

七份快照的完整 formal、formalData、formalRegistry、shortcuts 对象逐项比较相同，不只比摘要。覆盖 12,989 个正式文件、14,070 个正式树条目、两项正式数据、正式登记原本不存在、两个快捷方式的存在状态/目标/参数/hash。安装前快照 SHA-256 `67b5c7f610d59643642db713861f85426007963fd68057476145c5c61a8116b3`；立即安装前也重新取样并相同。每份快照均 SAC=1、CI RecordID=7097、进程 0。这是给定取样时点的证明，不声称审计席位实时重跑了普通桌面测试。

## VALIDATION

本席位实际执行：git status/HEAD/diff、有效 main 规则和源代码读取、`git diff --check b025ba44 e1379bb5`（退出 0）；四个相关 PowerShell 文件 parser（各 0 errors）；直接 GitHub run/jobs/artifacts/environment/branch-policy 读取；九份 ZIP digest/长度核对、八个 frozen 文件及七个 shared 文件核对、六类双车道 hash 核对；资源包 1,234 项核对、实装快照 14,213 个索引文件核对；候选签名正/负控；shell 内嵌 finalizer 身份；直接原始本机 receipt 与七份完整快照比较。

云端日志中实际执行、由本席位回读的验证：`npm run typecheck`、`npm test`（146 文件、1,044 测试）、`npm run build` 成功；finalizer 3 测试、update backup 4 测试；每条 Windows 车道资源工具 30 测试，Rust updater 9、resource trust 8、三条独立 custody/hard-exit/module 测试各 1、uninstall 3，均 0 failed、Rust 0 ignored。Rust 的其他被 filter 排除用例没有被冒称本次执行。正向实装 JSON SHA-256 `59977ae05efea2d4a6a1dc345c7855e1836d0864de06c63028544856a8a49c03`；负向 JSON `7bc75cf9bafa89e3b1543891781548583b53986bf45e95ab15c1442b8905317f`。比较 JSON 中通用 `installedCandidateSmoke=NOT_RUN` 不覆盖另外的实际 smoke run。

本席位未执行：本机原生编译/测试、重新安装/卸载、production mutation、Owner 正式签名、正式发行域操作、资源更新/崩溃恢复实验、GUI 提示可见性验证、外部异构最终审查。云端原始行为回执经本席位独立读取，不等同本席位又运行一遍测试。

FAILURES：保留两次本机测量结果及其边界。审计自身第一次 .NET 验签命令使用了错误的嵌套类型/值类型构造，未生成有效验签结果；改正 API 构造后正控成功、两条负控拒绝。第一次 environment 读取使用错误名称得到 404；从实际 workflow 得到正确名称后直接读取成功。这些属于审计仪器错误，没有用于产品结论。

ROOT_CAUSE_IF_KNOWN：第一版本机 readiness 参数不符合实际协议；第二版外层多要求了调用完成后的 PID 仍存活，具体退出原因未知；审计 API 构造/环境名错误已定位。ATTEMPTS_MADE：读取原始脚本与协议、分别保留结果、独立核对直接 receipt 和后续快照，修正只读验签命令与查询名后复算。WHY_BLOCKED：本审计没有需要 Owner 解阻的停点；未执行轴维持未执行。WHAT_REQUIRES_CONTROLLER_DECISION：在上述证据界限内归档 R2-06a 与安排后续包，不得把缺少的持续运行、正式域或更新证据改记为 PASS。

INVARIANTS_CHECKED：精确源/run/artifact/安装字节一致；候选与 Owner 签名用途/域分离；当前用户安装；资源读前校验与 native 入口；生命周期/登记锁与实例连续性；受控测试不等于 adoption；正式四类对象保全；原生构建只在云端；无代码签名/安全设置变化/PR 合并/公开发布。

RISKS：Owner 指定的“已钉住空目录可能原位成为重解析点”记 KNOWN_RESIDUAL。前提是攻击者已经有该当前用户目录的写权限，安装及卸载也只用同一用户权限，因此本安装模式不跨权限边界，不构成此次 R2-06a 修复或停工依据。**将来启用 perMachine 或任何提权安装前必须先修复并验证。** 退役诊断 ZIP digest `a19c5642e21746007e91ca447d32be50246276dbfc23a16921e2db321ec37b6f` 和结果 hash `b9449a31c9f431ce627309abbbce2f10f721901d6dd3d4912d95aea79f8f0601` 已由 MC-155 保全；本席位未复现该条件，不把诊断成功当跨边界证明。

DEVIATIONS_FROM_PLAN：本轮是已授权的只读证据审计，未做破坏生产修复的 mutation，也没有为全轴标签补做被禁止的本机原生测试。包视图拒绝、并发 fixture、单次产品调用按各自实际边界解读。G4 旧项的部分删除已由 MC-156 记录 fresh 审计后处置；本轮未重做旧清理验收。当前新残留由另一个独立任务逐项审计，本报告不授权把旧清单扩用于新对象。

OPEN_QUESTIONS：PID 在第二版直连回执之后退出的原因仍 UNKNOWN；它不改变已完成受控调用或卸载证据。完整正式签名/资源更新/回滚/恢复、并发更新与重装、资源与版本变化下可执行字节稳定性，以及精确最新源的外部 Claude 最终复核，仍需相应后续包证据，不能从本次候选外推。

RECOMMENDED_NEXT_ACTION：Controller 保留原始失败和正向回执，将本报告连同新残留的独立清理结论入检查点，完成报告提交后的卫生检查，然后按 Owner 已给授权继续 R2-06b。Owner 触点保留为正式 SHA256SUMS.windows 精确字节的离线签署、必要的边界决定、最终接受与公开发布决定；日常 CI、证据整理和已授权测试无需新增 Owner 触点。任何实测 SAC 拦截、正式对象变化或需要改变授权边界时，按既定停止条件处理。
