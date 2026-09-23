# GOGO 两稿对账

对象：`docs/design/GOGO-统筹设计.md`（产品/界面合同，本仓库）与 `gogoke_design_v0_12.md`（Codex 出的后端/安全合同，2026-09-16）。

两份是各写各的。Codex 稿 §0 原话：没有读取或冒称采用未定位的 Claude《GOGO-统筹设计》。
**所以撞上的地方是独立撞上的，撞不上的地方一条都没调和过。这份只做三件事：判归属、列统筹设计要改哪几处、列必须你定的三件。**

---

## 一、层次关系

| 议题 | 统筹设计 | Codex V0.12 | 结论 |
|---|---|---|---|
| 面向谁 | Owner 看得见的那一层 | 宿主、适配器、进程、数据库 | 两层，不是两案 |
| 对象数 | 6 个（项目/工作/主对话/旁聊/闸门/交付集合+验收） | 约 20 个（Goal、Session、NativeBinding、SourceEpoch…） | 不冲突：它的 20 个里只有 Goal 需要露脸，其余都在折叠层下；它自己 §3.1 也说 NativeStep、Job 不进一级导航 |
| 「工作」是什么 | 一等对象，沿用现有 conversationId | `Goal`，与 Session/Invocation/Execution 分开 | 同一个东西。Goal 是它的名字，工作是对外的名字 |
| 归栏五列 | 有，十条只读判定 | 无，只有 `lifecycle` + `run_gate` + `blocking_reasons` | 它 §20.4 那个返回结构只够算“待处理／在运行／已验收”；“可验收”还要材料、检查、审查那几个字段，它放在 P07 另一个快照 |
| 谁能打断你 | 只有真实 CLI 问题和真实闸门 | `HumanActionRequest`（PERMISSION/QUESTION） | 同一条规则的两种写法 |
| 施工顺序 | 原按完整路径切（0～6 批） | 按底座依赖切（M0～M6） | 不再是分歧：gogoke 与房间底层不同，没有"先接通"可选，见第五节 |

---

## 二、对上了，不再议

两边独立写出同一条，视为已定：

- 三家对等；席位是职责身份，不绑品牌（统筹 §1 ／ V0.12 §1）
- 正文里的 `@` 只是文本，不触发派工（统筹 §6 ／ V0.12 §8.1）
- 模型不能自封 Owner 权限（统筹 §5 ／ V0.12 §1）
- 私聊内容不回流主控，只有你选中递过去的那一句算数（统筹 §7-11 ／ V0.12 §2.2、§4.4）
- 不知道就写不知道，送达未知不画成已送达（统筹 §2 ／ V0.12 §13.1、§16.4）
- 沿用 gogoke 现有主题与组件接缝，不另造外部风格壳（Owner 立过的规矩 ／ V0.12 §3.2）
- 远程控制、移动端暂不纳入首版生产面（统筹 §9 的 D07 ／ V0.12 §1）

**D07 因此有了倾向。** 两边独立判到同一处，按「延期 + 写明重新启用条件」处理，比硬留一条未适配授权的远程执行入口更可辩护。仍需你点头。

---

## 三、Codex 对、统筹设计要改的五处

| # | 它的规则 | 统筹设计现状 | 改成什么 | 状态 |
|---|---|---|---|---|
| P1 | 采用（Adoption）与验收（Acceptance）是两件事（§10.1、§10.5） | §2 只有三个「完成」：回合结束 → 改动就绪 → 已验收 | 改成四个：**回合结束 → 改动就绪 → 已采用 → 已验收**。「采用」是交付集合的一个状态，不新增第七个对象 | **已改** |
| P2 | 验收/取消先进 CLOSING，结算在途才终态（§10.7） | §3 归栏第 2 条「有 acceptedAt 且未开启下一轮 → 已验收」 | 在它前面插一条：**已登记验收或取消、仍有在途未结算 → 待处理**，行上写「N 项在途待结算」。原第 2 条降为「无在途」才成立 | **已改** |
| P3 | 回答的两维状态：`response_fact` × `execution_control`，不许一个 status 互相覆盖（§9.3） | §6 事件映射把 `CLI_INPUT` 当单状态卡 | 问题卡与权限卡都带两个字段；允许显示「已回答，执行正在停止」，禁止同卡同时说「已批准」和「从未提交」 | 待改，属组件库的槽 02/03 规格 |
| P4 | 项目规则快照：哪些规则、哪个版本、缺了拒绝执行（§14.5） | 整份未提 | 执行详情增加「本次加载的项目规则」；缺规则时相关动作返回未就绪，不是静默少装载 | 待改，§4 窗口职责加一行 |
| P5 | 私聊分室：Owner 私聊与主控委托审查用不同逻辑会话、不同历史、不同可见域；盲审另建干净会话（§4.4） | 第 19 块「独立审计做不真」一直悬着 | 按分室落地。**第 19 块不能销** —— 它给的是一条设计规则，不是实现；gogoke 里还没有任何会话隔离，而房间那边是按席位复用（`ensureSeat`），那条路也不能照抄 | 待改，拆块清单第 19 块 |

P1、P2 是我判定表里的实错（一个缺状态、一个会把还在跑的工作画成已验收），已直接修在 `GOGO-统筹设计.md`。P3～P5 涉及组件规格和拆块清单，等三项待决定了一起动。

---

## 四、统筹设计对、Codex 稿缺的五处

| 要求 | 出处 | V0.12 状态 | 谁补 |
|---|---|---|---|
| 「正在进行」永远在主控输出的最下方，所有信息含选项卡都在它上方 | Owner 原话 | §3.2 画了派出卡在主控工作区，未定位置 | 界面层，统筹设计 §4 已定，Codex 稿需接受 |
| 发散／收敛／归一的派活展示，看板功能由它囊括 | Owner 原话 + Aster 源码 | 零提及 | 界面层，Codex 稿需接受 |
| 归栏五列 | 《工作主路径研究》§3 + Owner「一眼看全」 | 无这一层投影 | 界面层用它 §20.4 的字段算 |
| 对外只用一套词汇 + 禁用词 | 统筹设计 §2 | 正文里工人／派出卡／主控混用 | 发布前统一，不改它的内部命名 |
| 组件库五个槽各用哪个样例 | `kit/index.json` | §19 只列新建哪些目录 | 待你点选 |

这五条都在界面层，不与它的后端合同冲突——**是它没管，不是它反对。**

---

## 五、两条贵选择的结论（已查过现成做法）

这一节原来写的是"三个待决"。其中一个作废，两个已经有答案。

### 作废：施工顺序不是一个选项

原争点是"先建底座还是先接通"。作废，两个原因：

1. **gogoke 跟房间底层不是一个东西**（Owner，2026-09-16）。没有"接通"可选——壳要改成能干这件事，不是把壳接到房间上。
2. **V0.12 的 Rust 宿主不是已搁置的那条内核线。** 它 §19 写的是 `src/control_host/`，在 gogoke 自己的 src-tauri 里面；§1 明写不引入旧 Core、Store；§7.1 明写不引入第二个老 Room 服务。整份是照 gogoke 现状写的——§19 那张表逐行对的都是现有文件（`App.tsx`、`AppLayout.tsx`、`useThreadMessaging.ts`、Git/Diff/Files）。

### 结论一：工具收口照现成做法，不建网关

查过四家，**没有一家把 CLI 内建工具全关掉再重写一遍。**

主证据 `<local>\Grok Worker Provider`（4954 行；实测 51 次调用 / 7240 万 token / 4 账号 active；源码本体已核，非仅读文档）：

| # | 做法 | 出处 |
|---|---|---|
| 1 | 启动契约当数据，spawn 前逐条断言：`noPlan` `noMemory` `streamingJson` `terminalDenied` `subagentsDenied` `folderTrustEnabled` `trustFlagAbsent` `compatHooksDisabled`；并老实写 `windowsSandboxEnforcement: false` | `lib/provider.js:656`，断言在 `:441` |
| 2 | 两级家目录：持久按账号装认证与模型缓存，临时按每次调用装 HOME/USERPROFILE/LOCALAPPDATA | `:698` |
| 3 | 硬拒默认家目录及其父子路径 | `:424` + `:327` |
| 4 | 路径围栏：拒符号链接与 junction，限死在允许根内 | `:308` / `:320` |
| 5 | 项目权威预检：拒绝把席位派进带 `.claude/settings.local.json` 这类可授权配置的目录 | `:517` / `:527` |
| 6 | 剩下的工具边界共 **35 行**：`bash`／`run_terminal_cmd` 直接拒；`edit`／`write` 比对永久禁止根与允许写入根；每条决定追加一行审计 | `lib/hook-boundary.js` |

第 2 条解掉了一个实测过的死结：隔离 `CODEX_HOME` 会丢登录态（见 `docs/research/adapter-spike/01-capability-evidence.md`）。认证留在持久层，隔离做在临时层。

另外三家：Codex 自带 Windows 沙箱（ACL + WFP + 独立身份 + helper materialization + 提权后端）；Grok Build 自带 permission rules / exec risk / preflight / auto mode；DeepSeek Harness 带 `sandbox-windows-acl`——本项目审计判定它只覆盖 ACL 能表达的那一片文件权限，只能标 partial，不能当隔离证明。

**结论**：V0.12 §11.1 的 Managed-Tools 降级为「收口清单 + spawn 前断言 + 一个钩子 + 路径围栏」。不建统一工具网关，不建 LPAC ToolWorker 子系统。厂商自己维护的沙箱照用，能力如实标注，不宣称 OS 级强制。

**还缺的不是资料，是实测**：grok-worker 全篇写死 Grok（`GROK_HOME`、`models_cache.json`）。Codex 与 Claude 的等价开关是哪几个、关不关得住，要在目标机器上量一遍——这是施工任务。

### 结论二：步骤隔离不做

`docs/design/22-seat-runtime-inheritance-ledger.md` §4.1 原话：**`provider.js:433` 的 `spawnSync` 是 GOGO 与 grok-worker 分岔的那一行。** 隔离、契约、密钥卫生、围栏、审计全部可继承，只有进程生命周期这一处必须换掉——换成常驻。§4.2：

| | 一次性席位 | 持久席位 |
|---|---|---|
| 中途插话纠偏 | 不可能 | 可以 |
| 上下文 | 每次 `@` 都是新人 | 累积 |

准确说，V0.12 是中间态：回合内进程活着（回合内插话保得住），回合之间靠官方 resume 把上下文接回来。它买回了 grok-worker 丢掉的那半，代价是押在 resume 上——而 resume 的证据等级只有 SOURCE，从未动态测试。

它要解决的"旧帧误归新回合"，四家都不用重启解决：

| 做法 | 出处 |
|---|---|
| Windows suspended spawn，子进程加入 Job 之前起不来；持有 process handle 防 PID 复用 | Codex `codex-rs/utils/pty` |
| Weak registry 防 PID 复用误杀；close/spawn 竞态即杀晚到子进程；`kill_all` 幂等 | xAI `ProcessScope` |
| 记 `pid + processStartTicks`，区分数据属于活进程还是已死进程 | grok-worker `captureRunOwner` / `inspectRunOwner` |
| terminal receipt fence：把"进程结束了"和"回执提交了"分成两件事 | Claudexor |

**结论**：席位保持常驻。用不可复用的进程身份（`pid + processStartTicks`）加 receipt fence 解决来源归属，不每回合冷启动。

### 仍挂着：D07

远程控制、公开托管分享、账号轮换、桌面宠物、并发阈值、复杂实验（拆块清单不做的九条第 7 条）。Owner 2026-09-16：先挂着。

## 六、本次核到的事实

| 事实 | 出处 |
|---|---|
| `"csp": null` 仍在当前树里，未修 | `apps/desktop/src-tauri/tauri.conf.json:33` |
| 席位常驻，memory / midTurnInterject / streaming 挂在活进程上 | `packages/room/src/server.ts:1347-1350` |
| Codex 稿基线 `ab006d5`，与 `codex/gogoke-shell` 远端一致 | V0.12 §0 |
| grok-worker 的启动契约与 35 行钩子确实存在，已读源码本体 | `<local>\Grok Worker Provider\lib\provider.js:656`、`lib\hook-boundary.js` |
| Rust 内核线搁置记在施工计划，**不在** `docs/adr/`；ADR-0009 仍标 Accepted，README 仍写 M0 已接受 | `docs/design/GOGO-施工计划.md` §1 第 4 条；`docs/adr/README.md:24` |

V0.12 §12.1 把 `csp: null` 写成历史配置事实。**它现在还是当前事实。**

---

## 七、这份之后

两份都不是能开工的状态：Codex 稿自己说「供设计复核，不自报审计通过」，统筹设计第 1 批卡在五个槽没点定。
第五节两条已有结论、D07 先挂着之后，合成一份——界面那层用统筹设计，底座那层用 V0.12，接缝在 §20.4 那个返回结构和 §6 事件映射之间。不再各留一份。
