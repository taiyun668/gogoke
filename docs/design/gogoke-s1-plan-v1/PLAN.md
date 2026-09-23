# gogoke S1：底座与核心执行链成套施工计划 V1

**GPT 方案交付；PLAN_READY_FOR_OWNER_REVIEW。不是模板，不是开工回执，不是产品验收。**

本计划将 WP10a、WP01、WP02、WP03、WP09a、WP04 作为一个连续施工阶段，内部按依赖设置集成点，不把每个工作包、PR 或测试失败变成一次 GPT/Codex 往返。方案与关键取舍在此固定；获授权后由 Codex 完成实现、调试、集中修复及独立代码审查。下一项若仍有已定方案与授权，继续 Codex；进入新研究/方案阶段才回 GPT。

## 1. 基线、效力及本次交付边界

| 对象 | 固定身份 |
|---|---|
| 仓库 | taiyun668/gogo-party |
| 生产源码 | bc665a852833952b76d9508401193bedd2198436 |
| 源码 tree | 746024abeb2e6d35ac88fd1b2e315fceb0e9f54f |
| 原 A / snapshot | 620b49270f99f18cd9d8d5e7ffa68b76c5f274df / 55a8443ef98f1cc231f4f45d78c3bc3cad93a9e4 |
| 已独立复核 R3 payload / 登记 | c055e125531e166cb67a45531a3c7a8aa9d70330 / 7427b1eaba920e30a188623cea05807df9114ee5 |
| D 证据 / 登记 | 9ec7ca56566b667271013d75d867f2008aeeceda / 3fbef8d2b16f729d6f99dc05b37bc53ca96636fd |
| 工作流规则 / 本计划父提交 | a06619719bbac73a892d9e7b27b911cac77d8aa8 |

优先次序：Owner 当前明确授权与实际安全限制 > 已关闭 P00 有效证据和 R3 补充 > 本 S1 的具体实现决策 > V3 主计划未被本计划细化的原要求。原第 9 节检查截止不被本计划后移；原 C/T/WP 编号不改。来源固定在 SOURCE_MANIFEST.json；引用 `[SRC:路径]` 均指上述生产 SHA，引用 `[R3:路径]` / `[D:路径]` 指各自固定 payload，不使用浮动 main。

B01/B02/B03/C2-F01/K16/K17 的关闭是证据关闭；旧源码的错误行为仍需实现阶段修复。旧 chronology 不可证明的 Pass B 不恢复。无需为本计划再重跑 P00 盲审；后续实际实现必须做新 SHA 的独立审查。

当前有效状态仍是 P00 NOT_ACCEPTED、code-entry gate CLOSED_PENDING_GPT_P00_REVIEW、Owner PENDING、生产授权[]、runtime_verified=false。此次只提交方案、机器账本和方案检查器，不修改产品、现有测试、CI、AGENTS 或角色配置。D 的 ACCEPTED_FOR_CODE_ENTRY 仅为施工前技术就绪。

完整阅读次序：本文件 → CONTRACTS.md → TASKS.json → TEST_MATRIX.json → CODEX_HANDOFF.md；SOURCE_MANIFEST.json 支持固定来源；VALIDATION.json 仅描述本次计划数据检查。下游成果 SHA、实际席位数和 build digest 是执行时事实字段，不是留给施工者研究的设计空白。

## 2. 阶段交付和明确不做的事情

S1 交付：P10a（外部 remote/voice 保留式封存）、P01（公共合同与边界校验）、P02（新根/Session/耐久写入）、P03（唯一宿主/驱动边界/受限 worker 样本/真实假子进程控制）、P09a（五家能力观察接口与失效逻辑）、P04（受控最小客户端至假驱动的安全投递与事件全链）。

阶段演示应能在专属临时目录创建公共 Session，提交两次同 operationId 请求而只执行一次，模拟接受未知与重启恢复，展现迟到/缺口事件，处理绑定正确的批准/追问，并演示停止结果与 residual custody；不得只展示内存状态机或只有 JSON 文档。Windows 受控宿主/锁/假进程证据是相应 L/H 检查的必需品，不由 Linux 静态编译代替。

不做：重画 UI；五家真实完整对话验收（WP11a/11b）；全部 Git/Files/PTY、提示词和设置迁移（WP07/08/09b）；整体导入 Room/server/scheduler、旧 Rust kernel/store；用户真实历史迁移、真实登录/付费、麦克风/模型操作、系统 Tailscale 身份变更、安装/发布/签名；批量删除 Codex 模块。S1 不开放 remote/voice 以便“顺手测试”。

主产品仍可保留 legacy-only 模式，S1 public-only 路径在隔离根由受控客户端验证；没有完成相应兼容映射的原生命令在 public-only 下明确拒绝，不偷偷回 legacy。正常用户全量切换由后续产品贯通/迁移包完成。这不是维护两个能同时控制同一根/凭据实例的权威。

## 3. 此处已经定案的关键取舍

| 决策 | S1 固定方案 | 不允许施工中改成 |
|---|---|---|
| 实现归属 | 新公共模块放 desktop 的独立 Rust crate 内；现有 gogoke_daemon 作为唯一可持续宿主入口，Tauri 作客户端/视图桥 | 根 crates/* 内核直接成为 gogoke authority，另起 Room 调度器 |
| 数据层 | 新版本根 + 独占 OS 锁 + 单写者追加事务日志；旧 settings/workspaces 原格式保留，只读导入到新根 | 继续以 JSON 覆盖写作新权威、同时另建 SQLite/Room 双 store |
| 本地传输 | 公共宿主用当前用户受限的 Windows named pipe / Unix domain socket；旧 TCP 仅保留有鉴权 loopback 的 legacy 路径 | 把“localhost”当鉴权、重新开放远端、复用明文远程 token |
| 接线切换 | 根/实例显式选 legacy-only 或 public-only，写入入口统一接受同一 lease/custody；旧/新实际进程未封口不得切换 | 路由失败重试另一执行链，lease 到期即认为进程死亡 |
| 原生适配 | 现有 Rust Codex 机制原位封装；TS 只取经登记的协议/纯校验逻辑，做受限 worker 原型 | 直接实例化 Persistent*Seat/InstanceStore，把其 owner/persistence 带入 |
| 能力观察 | 基于二进制身份、模式和授权上下文的三态 supported/unsupported/unknown；只读观察和原生启动分离 | 按品牌或 PATH 命中假报 ready，自动登录/下载/升级 |
| 投递与状态 | 先耐久 intent，再派发；接受未知不盲重发；执行终态与投递/显示/停止分别报告 | Promise/emit/EOF/exit0 即整体完成 |
| 阶段粒度 | 六包一个计划、三次内部集成波次、批量缺陷修复；授权可整段也可限定子集 | 每张任务卡回来制定新方案，或未授权包自动续跑 |

CONTRACTS.md 定义对象、状态机、数值上限、错误分类、持久化、launch、停止、重试与授权细则，是本表的可执行解释。政策上的选择已经固定，普通函数拆分、内部数据结构和等价实现由 Codex 处理；确需改变合同才回 GPT。

## 4. 三个内部施工波次

**波次 A：WP10a + WP01。** 两条独立代码线并行；只有 Controller 写共享 lib.rs/state.rs/types.ts/tauri.ts/根登记等集成文件。Luna 可先做 UI 禁用与假件，Sol 负责统一策略和公共类型。P10a/P01 各自通过 L/D 检查后可进入 B，不要求等待 S2/N1/I/M。

**波次 B：WP02 → WP03。** 先根/锁/日志/恢复，再唯一宿主、进程 containment、worker 样本和 adapter gate。测试编写可基于固定 P01 合同提前并行，但 P03 不得在 P02 未验收时切运行接线。T55.L 两个锁测试进程与 T55.H 两个完整测试宿主分开取证。未知旧 writer 必须保管，不以测试跑完取消保管。

**波次 C：WP09a → WP04。** 先能力快照/拒绝策略，再投递、事件、批准、最小委托及最小客户端；三个 WP04 实现面按目录和接口分写。最终同一个提交经过全轴审查；处于 deferred 的真实提供方/产品 UI/迁移义务必须完整传给 S2/S3。

依赖严格保留：WP10a←WP00，WP01←WP00，WP02←WP01，WP03←WP02+WP10a，WP09a←WP03，WP04←WP03+WP09a。授权整个 S1 时，这些是 Codex 内部里程碑而非六次 Owner 询问。若 Owner 只批准 WP10a/WP01，则完成波次 A 后等范围授权；这不是重新研究、也不需要重新制定 B/C 方案。

## 5. 按既有团队执行的任务分工

TASKS.json 给出 19 张有限任务卡及 Controller 集成责任、依赖、允许目录、检查与回执；它们不是 19 个并发 agent。

Sol/medium Controller 核实实际角色/工作区/授权、排程、集成、提交与回执。Luna/high 做简单 UI/机械接线；construction Luna/max 做大批明确实现与测试；Sol/high 做已定界的 root/store/宿主/投递/权限核心。普通 fresh Sol/high 独立核验与修复聚焦；高风险首次及最新候选最终全轴使用 fresh Astra/xhigh。实现者不自我 acceptance。

`.codex/agents/construction.toml` 禁止 worker 自行 commit/push，不能因计划写了“交付提交”就让它越权；worker 返回文件差异，获授权 Controller 提交。固定 role 覆盖通用模型字段的情况按实际 runtime 核实；本计划不声称已经启动、改模型或增加并发。

容量适配：可并行资源≥3时取一个核心线、一个互不冲突施工线、一个测试线；这只是调度上限建议，不是已知实际槽数。只能2条时合并简单接线与测试队列；只能1条时按 DAG 串行，审查仍必须独立 fresh，不变成作者自验。容量不足排队/复用已结束槽位，不外建 Codex 会话绕上限。

Grok 当前受限只读、无终端/子进程，不在关键施工或验收链；没有新的有效能力与授权不得派写任务。Astra Specialist 仅做合同内疑难定位；需要新架构/方案回 GPT，不借强模型继续在 Codex 研究。外部 Claude 最终异构义务保留至原计划要求的最终候选，不冒称已连接或自动派发。

## 6. 共享文件、来源和变更范围

共享热点由 Controller 唯一集成：`apps/desktop/src-tauri/src/lib.rs`、`state.rs`、`types.rs`、`bin/gogoke_daemon.rs`、`shared/mod.rs`、`apps/desktop/src/types.ts`、`services/tauri.ts`、`services/events.ts`、`features/app/components/MainApp.tsx`、manifest/lock/CI。worker 返回接口 patch，不并发修改这些文件。

受允许的产品落点为 TASKS.json 明示目录及其同域测试；新目录均标 planned_new，不冒称已存在。`packages/seat-runtime`、`packages/room`、根 crates 只读参考。需要采用纯片段时记原文件/符号/blob/许可和目标位置，不创建指向研究副本的软链，不把参考模块整包导入。

允许未来把 desktop 已锁定的 windows-sys 0.61.2 提升为直接依赖，用于独立的 Win32 handle/job/pipe/ACL 封装；不重用根 authority crate 的整体权威。其它未列新依赖/升级/运行时下载需 GPT/Owner 变更决定。Rust desktop最低1.89与根内核1.85 CI分开；Node24与desktop TypeScript5.8.3、seat参考TypeScript5.9.3也分开，不“统一升级”消除差异。[SRC:apps/desktop/src-tauri/Cargo.toml] [SRC:.github/workflows/gogoke-desktop.yml]

## 7. 施工测试：命令存在不等于已经执行

TEST_MATRIX.json 从固定主计划逐行提取全部 T01–T68 及所有检查截止，并标出 S1 当期必验项和后续义务；保留主责，不把 T43/T54 等后期全通路测试改称 S1 已通过。提前做的本地子用例标 S1 补充，不注销原检查。

现有工具链命令：desktop `npm run typecheck`、`npm run lint`、`npm test`、`npm run test:localization`、`npm run check:product-identity`；Rust对明确的desktop manifest执行 fmt/clippy/test，不拿根workspace绿灯当desktop通过。依赖须已按各自lock安装到批准工作区；不得临时用 npx 自动下载。`npm ci` 的 postinstall、build 的 prebuild、doctor/tauri-build、root room:start 的副作用不同，不能为检查而无审查运行。[SRC:apps/desktop/package.json] [SRC:package.json]

拟新增 `tools/gogoke-s1/check_boundaries.py`、`check_traceability.py`、`run_checks.py` 及 `apps/desktop/src-tauri/tests/gogoke_s1_*.rs`：由 WP01/对应实现包交付，当前不存在。运行器要发现0测试/缺平台/缺产物/skip，不能输出假PASS；每个 checkId 输出 source/result/build/plan/auth SHA、命令、exit、用例、fail/skip、原始日志。`python tools/gogoke-s1/run_checks.py --wp WP03 --out <批准artifact目录>` 是待实现接口，不是本轮实际命令。

通用旧CI失败独立记录；复現到固定基线者登记BASELINE_FAILURE，不能隐藏、更改预期或顺手修无关内核。它不自动证明新代码失败，也不提供合并许可；相关受影响测试或新测试失败则阻断对应产物。既有CI/测试保留，新增 S1 job 精确定位桌面图，不触发发布；原会生成unsigned安装包的workflow不冒充本次许可。

## 8. 验收、失败批次与可恢复施工

每个局部产物经过自测＋fresh代码复核后供下游消费；涉及 root/权限/并发/进程/状态的首次集成和阶段最终候选执行全轴 Astra 审查。一次审查继续完成其余不依赖缺陷且安全的轴，批量返回发现；原工人集中修、fresh Sol 聚焦，再对最新SHA全轴。审查/patch/恢复全过程不得改测试预期、关闭功能来消掉原本必验行为。

Windows物理/受控平台缺失时保留平台阻断，不把mock当T55.L/H或T52.L的真实行为。未授权真实提供方留到N1/I，而新假CLI控制机制、锁与恢复在S1按批准的本地副作用测试。未知所有权、身份错版、源码基线漂移影响合同则只暂停受影响路径回GPT；独立安全队列可继续。

代码回滚使用 Controller 的阶段提交/独立worktree，不reset用户dirty tree；数据回滚只用S1临时根与版本化快照/日志，原件不删。中途失败保留目录、日志、lease/custody；未能确认进程退出不得仅清理目录。恢复到legacy必须确保所有新writer停止且旧程序不读取新schema，绝不因public失败自动双启。

S1 完成必须同时具备：六个产物、所有S1截止检查、最新独立全轴结果、C/B03/C2来源义务落地、一次受控纵向演示、工作区保全、真实未执行矩阵、明确后续研究/施工去向。执行时证据归 `artifacts/s1/` 或批准外置目录，不把日志假写成此前P00报告。

## 9. 交接与下一项工作

本计划覆盖完整S1，不能要求Owner每个task搬运。CODEX_HANDOFF.md 是一次性接包指令；接收前先核本计划固定SHA及Owner的scope/test-effects授权。整段获批则内部按DAG平推；只获子集则不超范围。无需再回GPT逐个选 schema/持久化/进程方案。

S1结束若S2仍待研究，Codex固定actual-head、产物digest、检查/失败/未执行、独立报告、残留和待决事项后，第一时间回GPT。若后继施工已定案、依赖满足且获权，可继续Codex。普通编译/debug不切平台；修改公共合同、安全/迁移/采用边界才触发GPT决策。

本轮计划提交不会替代Owner接受P00，不会自动派Codex、建生产分支或合并PR。建议下一次Owner以固定计划SHA一次确认授权整个S1的代码及临时根/假进程/私有本地IPC限定测试；或明确只授权波次A。真实账号/费用/用户数据/远程网络/安装发布仍须单独授权。
