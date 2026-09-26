# GOGO 要做的和不做的

> **动任何 GOGO 设计或施工之前先读这份。**
> 第二节是 gogoke 已经有的（别当成没有），第三节是已经砍掉的（**不要再提**）。
> 每条带日期。要改就在这份上改，不要在对话里重讲。

---

## 一、界面上新做的（九样）

| # | 样 | 是什么 | 现成依据 |
|---|---|---|---|
| 1 | **派活的工作树** | 正在进行：钉在主控输出最下方、发散连线、收敛、归一折成一行 | 演示 `gogo-work-tree.html`（八版，Owner 看过）。gogoke 现在只把 collab 项按流内联渲染，钉底与折叠都没有 |
| 2 | **实例管理页** | 设置里新增一节：账号、CLI 安装与登录、并发与额度来源 | 设置现有 16 节，加一节 |
| 3 | **角色管理页** | 右侧面板一个 tab，与 Git／Files／Prompts 并排 | Owner 2026-09-16 指定位置 |
| 4 | **旁聊与递话** | 面板里第二条对话（秘书长或本项目审计）；选中一句递给主控，排队或插话 | 施工抓不到，一律走主控 |
| 5 | **秘书长在左栏的位置** | 独立一个位置，全局，跨项目 | — |
| 6 | **未读（完成未看）** | 打开那条对话就清 | gogoke 的 `ThreadSummary` 没有 unread 字段 |
| 7 | **改动就绪那行字** | 左栏行上 + 折起来那行：`N 个 commit · 已在你的工作区` 或 `· 在 feat/x 上` | 要 commit 归属才写得出来 |
| 8 | `userInput` 卡摆在哪 | 摆放决定，组件现成 | `RequestUserInputMessage.tsx` |
| 9 | Git 行选反馈改用途 | 收件人改成本项目主控；加"这里不对"的自由文本意图 | `buildPullRequestReviewPrompt` → `sendUserMessageToThread` 已跑通，现在发给 `reviewThreadId`，预设四个审查意图 |

---

## 二、gogoke 已经有的界面形状（**不是能力**）

> **警告：下面这些不是 gogoke 的能力，是 gogoke 给 Codex app-server 写的渲染层。**
> `src-tauri/src/` 里只有 `codex/` 模块，claude／anthropic／grok／xai／gemini 零引用（2026-09-17 核）。
> 换成 Claude 或 Grok，下面每一项都不存在。**界面形状可复用，产品能力为零。**
> 把一家的协议当成产品本体，是 V0.12 踩过的坑，不要再踩。

2026-09-16 逐条核过类型与源码。

| 东西 | 在哪 |
|---|---|
| ~~席位 = 带角色的线程~~ | `ThreadSummary.isSubagent / subagentNickname / subagentRole` 是 **Codex 的 sub-agent** 形状。**改造后席位 = 一个独立 CLI 进程 + 它自己的会话**，不是线程——渲染可复用，**概念要换**（整合设计 §8.5） |
| 左栏按项目排 | `ThreadListOrganizeMode = by_project / by_project_activity / threads_only` |
| 派活的发散结构 | `tool.collabSender / collabReceivers[] / collabStatuses[]` —— **来自 Codex app-server（`sender_thread_id`），不是 gogoke 在编排** |
| CLI 问题 = 要你定 | `RequestUserInputRequest`，绑 thread + turn + item —— **app-server 的请求**。主控要你定走的是同一条路，一个组件不是两个 |
| 待处理标记 | `Sidebar.tsx` 的 `pendingUserInputKeys` |
| 席位不直接找你 | `useResponseRequiredNotificationsController` 的 `isSubagentThread`——**判据绑在「是不是子线程」上；改造后席位不是线程，这个门要跟着改** |
| 权限请求 | `ApprovalRequest { workspace_id, request_id, method, params }` —— **app-server 的 JSON-RPC 透传** + `ApprovalToasts` |
| 改动文件 | `tool.changes[] { path, kind, diff }` —— **没有 commit SHA** |
| 成果阅读与定位反馈 | `GitDiffViewer` 的 `SelectedLineRange` + `onRunReviewAction` + `pullRequestComments` |
| 中顶 | 工作区名、分支、worktree 信息弹层（改名／upstream／复制命令） |
| 设置 | 16 节 |
| 右侧面板 | Git / Files / Prompts 三个 tab |

---

## 三、已经砍掉的，不要再提

| 砍掉的 | 为什么 | 日期 |
|---|---|---|
| 看板（五列、独立页） | 功能由「正在进行」囊括，归一折起来那一行就是它 | 2026-09-16 |
| 归栏五列 | 同上。只剩一个状态字，在对话行上 | 2026-09-16 |
| 成果 tab | 首版只做代码生产，成果就是 Git 和 Files 里那些东西 | 2026-09-16 |
| 交付清单页 | 折起来那行展开就是 | 2026-09-16 |
| **验收动作** | 没有下游需要它。你看过就完了，接着说是继续，不说就是到此为止 | 2026-09-16 |
| 闸门（对账／低进展／花费／停止卡） | 首版不做 | 2026-09-16 |
| 「规划中」这个词 | 没活动就是没活动，不用一个词描述它 | 2026-09-16 |
| `branchId` | 发散结构就是 `collabReceivers[]` | 2026-09-16 |
| 送达七种状态 | 席位行按演示写最近动作与交回的第一句，不写状态词 | 2026-09-16 |
| 「独立性未确认」标记 | **项目审计就是独立审计**，独立性是结构性的（另一个席位／线程／可能另一家） | 2026-09-16 |
| 房间那套词汇（NOTE/RESPONSE/ACTIVITY/SYSTEM_STATUS/CONTROL/CLI_INPUT） | gogoke 里一个都不存在 | 2026-09-16 |
| 「接通房间」 | gogoke 与房间底层不是一个东西，没有这条路 | 2026-09-16 |
| 第 0 批「接通」、单列的「事实合同」批 | 并进第 1 批的前置 | 2026-09-16 |
| 「成果」tab（研究 §5 要的成果本体） | 首版只做代码生产，成果就是 Git／Files；通用成果 M10–M14 全部未实现，属首版之外 | 2026-09-16 |
| 验收记录（研究 §9「验收归属不能去掉」） | Owner：不用点。留下的是事实——改动就绪、落在哪、未读打开就清 | 2026-09-16 |
| **远程访问**（`tailscale` 1873 + `remote_backend` 515） | Owner：暂时关掉。**封入口不删码**；重开前要先解决「远端进来的人不在三个对接面里」 | 2026-09-17 |
| **语音输入**（`dictation` Rust 1606 + 前端 435，含本机转文字模型） | Owner：暂时关掉。**封入口不删码** | 2026-09-17 |
| 「有待授权的落地动作」当成一个状态字 | 活干完了改动就在那儿，那是**改动就绪**，只是落点在分支上。「并进来」是行上的一个动作，不是在等你 | 2026-09-17 |
| **「一席一实例」**（老 GOGO `31` §2.1） | 与「同一个实例可以服务不同项目的席位」直接打架。一个实例可服务多个席位；能同时服务几个按各家 CLI 实测的并发能力定 | 2026-09-17 |
| 「先统一组件再做画布」的排序（研究 UI 拆解） | 组件点选已挪到最后；第 1 批不挑任何组件，工作树的 chosen 已定 | 2026-09-16 |

---

## 三点五、组件库（已建，三件一套）

| 件 | 在哪 |
|---|---|
| **库** | `apps/desktop/src/features/design-system/kit/index.json` —— 槽、候选、来源、许可、commit、本地路径、尺寸值、规矩、**砍掉清单** |
| **展示面** | https://claude.ai/artifact/FSkMBLHwHPxHrDJU9yiqWy （db 存选择；槽 01 做到了质量，其余潦草） |
| **agent 渠道** | `.agents/skills/ui-kit/SKILL.md` —— 动 gogoke UI 之前先读 `kit/index.json`；`chosen` 为空就停下来问 Owner |
| **演示** | `docs/design/demos/` —— 工作树（八版）与槽 01 的全保真候选，**已进仓库**，不再只存在于临时目录 |

**另外两个 artifact**：填充清单 https://claude.ai/artifact/3B5rbhNwUxGh5LcgPMiQKi （已被《GOGO-拆块清单》取代）；工作树演示 https://claude.ai/artifact/U9JwR8idVCNskcx413XSaC （源文件已进 `docs/design/demos/`）。

**Aster 源码没有搬进仓库**（第三方，ADR-0009 未授权复制上游源码）。取值和 commit 记在 `kit/index.json` 的候选里。

**2026-09-16 按本轮结论重排，十个槽：**

| 槽 | 状态 |
|---|---|
| `user-input-card` 要你定的那一下 | **待定摆放**。原 `decision-card` 与 `cli-question-place` 合并——同一个机制，不是两个槽 |
| `work-tree` 派活的工作树 | **已定 = 我们那个八版演示** |
| `permission-request` 权限请求 | **已定 = 用现状**（`ApprovalRequest` + `ApprovalToasts`） |
| `composer` 输入区 | **已定 = 用现状** |
| `row-state` 左栏行上的状态字 | 无候选 |
| `sidechat` 旁聊与递话 | 无候选 |
| `roles-page` 角色管理页 | 无候选 |
| `chief-entry` 秘书长的位置 | 无候选 |
| `unread` 未读 | 无候选 |
| `instances-page` 实例管理页 | 无候选 |

六个无候选的**先定后端事实，再挑组件**——后端没有就挑组件太早（Owner 2026-09-16）。

---

## 三点七、首版要适配的五家（2026-09-17 逐个核过本机）

Owner 2026-09-16 定：首版适配 Claude Code、Codex、OpenCode、Grok Build、Antigravity。

| CLI | 本机版本 | 适配现状 | 通道 |
|---|---|---|---|
| **Codex** | codex-cli `0.149.0`（npm） | **已有** `PersistentCodexSeat` | `codex app-server --stdio` |
| **Claude Code** | `2.1.196`（npm） | **已有** `PersistentClaudeSeat` | `claude -p --input-format stream-json --output-format stream-json --verbose` |
| **Grok Build** | `1.0.30` `04b7ffed98c6` stable（`~/.grok/bin`） | **已有** `PersistentGrokSeat` | `grok agent`，ACP stdio |
| **OpenCode** | `1.17.18`（npm） | **要新写** | 待查 |
| **Antigravity** | IDE `1.107.0`；**无头 CLI 是 `agy`，本机未装** | **要新写，且只能当一次性席位** | agy，事件流与 ACP 同形 |

### 三家已经做完了，在 `packages/seat-runtime`

`@gogo/seat-runtime`：**4987 行，零运行时依赖，明确不 import 房间**（源码注释说明了为什么不跨包依赖）。
自带 `isolation.test.ts`、`close.test.ts`，以及 `fake-codex-app-server.mjs` / `fake-grok-acp.mjs` 两个假件做一致性测试。
每个席位收 `credentialHome`（绑定实例的隔离家目录），`allowRealUserHome` 只作为显式例外。
`detectProviders()` 用 `where`/`which` 探测——**可用性是探出来的，不是写死的**，不可用的写明原因。

**这一层不用从零建，而且是我们自己的代码**：首个提交 2026-08-22（`feat(room): seat runtime + project room — M1/M2/M3`），20 个提交，只 import node 内置与自己的兄弟文件，无上游依赖。AGPL 来自仓库根 `package.json`（`gogo-party` 就是 AGPL-3.0-or-later），`apps/desktop`（`gogoke`）没有自己的 license 字段、同在这个仓库里；CodexMonitor 是 MIT，并进 AGPL 无冲突。**没有许可问题要解。**

另：**`close.ts:188` 的 `tryQueryProcessStartTicks` 已经实现了 `pid + processStartTicks` 的进程身份**——也就是审计里用来替代「步骤隔离」的那个围栏，不是提案，是跑着的代码。

### Antigravity 的限制（证据在 AionUi 源码注释）

> **2026-09-26 补注：本节已过时，不能再当依据。**claw-orchestrator 的实测表明，agy 可以用 `--conversation <id>` 续接，上下文会累积；Google 另有一个常驻的官方 ACP 内核。能否中途插话、有没有只读模式，还要在本机实测。见 [Antigravity CLI 事实](../research/2026-09-26-antigravity-cli-facts.md)。

- 事件流与 ACP 同形：*"the renderer treats it as an ACP-family conversation because the extra payload and event stream are identical"* —— 适配路径与 Grok 那个同类
- **一次调用一个进程**：*"agy is a direct-CLI integration (one process per turn)"*
- 因此它**中途插话纠偏不可能、上下文不累积**（`22-seat-runtime-inheritance-ledger.md` §4.2 记过这个代价）
- **结论：能当一次性席位（审计、检查跑一轮），当不了主控**
- 有 PreToolUse 钩子回调（AionUi changelog `#860`）。Omnigent 那句"Antigravity native 只能事后审计、无法在工具执行前阻断"（`docs/research/source-audit/02-omnigent-claudexor.md`）说的是它自己那条 native 路径，与经 agy 不矛盾

### 实测的版本漂移

`docs/research/adapter-spike/01-capability-evidence.md` 测的是 Codex `0.147.0` 和 Grok `1.0.0`；本机现在是 `0.149.0` 和 `1.0.30`。
**Claude Code、OpenCode、Antigravity 三家在本仓库零实测。** V0.12 通篇只谈三家，本身就少了两家。

---

## 三点八、老 GOGO 里可以直接搬的（2026-09-17 核）

**十三个模块全部零依赖 `packages/room/src/server.ts`**，是独立的，不是「接房间」——是把我们自己写过的代码拿过来。

| 模块 | 行数 | 顶上哪一项 |
|---|---|---|
| `packages/seat-runtime`（整包） | 4987 | **三家席位适配**（Codex / Claude / Grok），含隔离测试、关闭测试、两个 fake 一致性件；`close.ts:188` 的 `tryQueryProcessStartTicks` 就是来源归属那道围栏 |
| `room/src/accounts.ts` | 1214 | 实例管理页：多账号 |
| `room/src/install.ts` | 1045 | 实例管理页：CLI 安装与探测 |
| `room/src/repository-preview.ts` | 703 | 仓库文件读取与预览 |
| `room/src/halt-conditions.ts` | 490 | 停止条件（**Owner 立过规矩：+1 公式不许动**） |
| `room/src/instances.ts` | 489 | **实例管理页**本体：`InstanceStore`、凭据路径。`InstanceProvider` 现在是 `claude \| codex \| grok`，要扩到五家 |
| `room/src/onboard.ts` | 367 | 首次使用 |
| `room/src/projects.ts` | 361 | 项目与工作边界 |
| `room/src/input-references.ts` | 343 | 输入区的「本次引用」 |
| `room/src/runtime-context.ts` | 165 | 运行上下文 |
| `room/src/room-registry.ts` | 105 | 登记表 |
| `room/src/proc.ts` | 51 | 进程工具 |
| `room/src/role-policy.ts` | **26** | **角色管理**：四个角色的中英别名、歧义判 null、`defaultAccessForRole`（主控与施工可写工作区，审计与秘书只读） |
| `room/src/mask.ts` | 20 | 脱敏 |

合计约 **9700 行**我们自己的、可搬的代码。这直接改写了第四节「后端基本从零」那句——**从零的是编排与界面的接线，不是这些零件**。

### 第三方源码的许可

| 来源 | 许可 | 能不能搬 |
|---|---|---|
| Aster（派活工作树的参考） | **Apache-2.0**（不是 MIT） | 能。Apache-2.0 单向兼容 AGPL-3.0；要保留许可声明与 NOTICE、标注改动 |
| Beautiful UI | MIT | 能 |
| Keyline 图标、Amicro | MIT | 能 |
| CodexMonitor（gogoke 上游） | MIT | 已经在用 |

注意 ADR-0009 的一条既有规矩：**在模块级 provenance、依赖、NOTICE、平台和测试 Gate 之前不复制候选上游源码**。要搬 Aster 得 Owner 明确放行。

### Codex 那份抽取矩阵怎么用

`docs/research/source-audit/09-cross-repository-extraction-matrix.md` 把十九个仓库分了级，**只有 `SOURCE-CANDIDATE` 才是「可以抄代码」**，`PRODUCT-PATTERN` / `SEMANTIC-PORT` 明写不是生产准入。

但整张表瞄的是**已经搁置的 Rust 内核线**（`harness-runtime-core`、`adapter-sdk`、`task-ledger`、`workflow-engine`…），那些模块不存在了。它推荐的 process runtime bake-off，结论我们已经有——`close.ts` 的 `pid + processStartTicks`。

**还活着的只剩语义**：Claudexor 的 terminal receipt fence 与 failure taxonomy、Omnigent 的 capability vocabulary。这两条进设计，不进代码。

---

## 三点九、整合时还要收进来的（2026-09-17 系统过了一遍，别再漏）

### A. 老 GOGO `31-product-design.md`（528 行，Owner 2026-08-23 定，自包含）

**这是老 GOGO 的统筹设计。它把我们这两天当成「要新设计」的东西已经定完了。**

| 节 | 内容 | 顶上哪一项 |
|---|---|---|
| **§7.1 分层与准入** | 「能被装被登录」≠「能真跑一个席位」。**一个厂商必须配置层与运行层都做完、隔离过反向验收才进注册表；产品里不存在「半支持」**。注册表条目 `seat` 字段必填，没适配器加不进来。但第一屏要如实说支持范围 | **五家怎么上架** |
| **§7.2 已上架** | Claude Code `@anthropic-ai/claude-code`／stream-json；Codex `@openai/codex`／app-server；Grok Build `@xai-official/grok`／ACP | 三家 |
| **§7.3 候选** | `@google/gemini-cli`、**`opencode-ai`**、`openclaw`、`@deepseek-ai/dsh`。Pi 与 Hermes 查不到可信包，不写进任何清单 | **OpenCode 已在候选里** |
| **§7.4 上架一家的九步** | ①核实分发 ②手动跑状态命令两种情形 ③判据+双向用例 ④实测隔离变量 ⑤空目录反向验收 ⑥查跨实例共享与能放宽沙箱的环境变量 ⑦写适配器 ⑧能力边界映射成启动参数+反向用例 ⑨席位启停与流式测试 | **OpenCode 与 Antigravity 照这个走** |
| §6 实例 | 一个实例是什么、免费状态命令、实例状态、**实例页**、**不显示剩余额度**、登录 | 实例管理页 |
| §5 本地环境 | 产品自己装：先 Node 再 CLI、装失败怎么办、**升级永远由人点**、装在别处的 CLI | 安装 |
| §4 第一次进来 | 九小节，从「你有哪个账号」到「让它真的说一句话」 | 首次使用 |
| §2.1 / §2.2 | 一席一实例；「占用」是什么意思 | 席位与容量 |
| §9 / §10 / §12 | 失败与退路、不变量、明确不做 | 收口 |

**Antigravity 直接撞上 §7.1**：agy 是一次调用一个进程，`seat` 能力是降级的。要么按一次性席位做一份完整适配器，要么按「不存在半支持」的规矩，产品里根本不出现。

### B. 老 GOGO 其它没收的

| 文件 | 内容 |
|---|---|
| `21-cross-project-resource-arbitration.md` | **项目隔离 ≠ 每个项目有独立物理容量**；共享稀缺资源必须先拿到原子、可恢复的许可。正好补 Owner「同一实例服务不同项目也要隔离」的另一半 |
| `19-execution-target-model-routing.md` | 每次执行绑一个精确、可审计、**不可静默替换**的目标 —— 就是「不自动换号换模型换 Harness」 |
| `20-local-control-host-lifecycle.md` | **桌面窗口是 Control Host 的客户端，不是生命周期所有者** —— 关窗 ≠ 退出 |
| `18-project-context-room.md` | 卡片式项目上下文现场（Owner 提供的视觉参考） |
| `32-gemini-adaptation.md` | 第四家的适配调研，扩厂商时照着做 |
| `crates/gogo-win32-authority`（7484 行） | 路径与文件身份权威：canonical parent、file identity、reparse tag、硬链接、no-clobber 发布。正是路径逃逸那道防线 |
| `packages/room/public/work-canvas.js`（43K） | 节点、边、SVG 连线的画法 —— 工作树可参考（五列本身已砍） |

### C. V0.12 里还没收的（按可用性排）

| 节 | 内容 |
|---|---|
| **§18 前后端状态对照表** | 20 行「后端事实 / UI 主状态 / 用户动作 / **不能显示**」。**最实用的一张**，直接当接线断言 |
| **§23 AT-01～AT-45** | 45 条负测场景。**最值钱的资产之一**，直接当验收清单 |
| §12.1 / §12.2 | CSP 基线字符串；前端内容渲染安全清单：Markdown 禁原始 HTML、SVG/HTML 当文本或隔离栅格化、图片重编码剥元数据、外链只允许规范化 http(s) 并显示真实主机、终端限制 OSC52、**真正的审批控件不从 Markdown 生成** |
| §3.4 | 六种现场表：CLI 等回答／资源占满／投递未知／改向停靠失败／成果有冲突／私聊依据落后，每行带「不允许的捷径」 |
| §16.4 | UNKNOWN 的四个动作：核对原执行／确认停止／恢复上下文／作为新执行继续 |
| §16.2 | **容量 ≥2 时保留 1 个前台槽位**给直接交互；容量为 1 时明说不能同时推理 |
| §11.2 | 信任主体访问矩阵，七行 |
| §4.1 | Session 状态 → 默认发送行为表，七行 |
| §4.2 | **中文输入法 composing 时 Enter 不发送**；流式不抢焦点不强制滚动；长日志虚拟化；状态有文字与屏幕阅读标签、错误不只靠颜色；断线显示最近已知状态和时间，不乐观显示「仍正常」 |
| §4.3 | 附件四态 `IMPORTING → FROZEN → ATTACHED → SENT`；三家共同附件合同（共同图片输入不过就整体不发布） |
| §9.2 | 敏感答案不进公共 timeline／通知／普通日志；要密码 token 验证码的场景优先跳官方登录；通用问答工具不得收集可复用凭据 |
| §5.3 | 故障四分：网络未知／登录过期／并发不足／额度耗尽；**本地累计用量不冒充官方剩余额度** |
| §10.2 | `.git` 不给模型（受管工作副本）；检查命令用受批准的声明固定；**测试 stdout 的「PASS」不替代退出状态** |
| §22.2 | 现有体验保留矩阵九行：每项原有体验、新接缝、**迁移必须证明什么** |
| §2.3 | 关侧栏只关视图，关桌面只断 UI，真正退出另走停止流程 |
| §17.2 | INV-01～INV-12 不变量，可直接当断言 |

---

## 四、后端（大头，基本从零）

| 要做的 | 说明 |
|---|---|
| **会话管理（骨架）** | 改造后是 **CLI 派活给 CLI**，不是 Codex 的 sub-agent。隔离全落在会话怎么管上：一席位一进程一会话；施工的会话只装任务包；正式审计必须新建干净会话；续接模式如实写（原生续接／分叉／冷重建）；会话不跨项目复用。**必须实测**：主对话里说一句只在那出现过的话，问施工答不答得上来 |
| **席位编排本身** | gogoke 是纯 Codex 的：`src-tauri/src/` 只有 `codex/`，没有任何别家。现在的「席位」是 Codex 的 sub-agent；**改造后是宿主做 CLI→CLI 派活**。**归一层不用新建**——`packages/seat-runtime/src/seat.ts` 的 `Seat` 接口与八种 `SeatEvent` 就是厂商中立的那一层。角色不绑 CLI、审计可以是另一家、同一实例服务不同项目——**适配层整个从零**。注意：审计要独立，至少得有第二家，所以首版不可能只有 Codex 一家 |
| 主控拉席位、拉角色 | 在 Owner 给的授权范围内自主，不必回来问 |
| 实例 | 账号、CLI 安装与登录；**两级家目录**（持久的按账号装认证与模型缓存，临时的按每次调用装 HOME／USERPROFILE／LOCALAPPDATA） |
| **项目级隔离** | 同一实例服务不同项目的席位也要隔：资料、记忆、执行权一概不共享 |
| 交接 | 交的是 commit 与改动文件引用，不是上一位的转述 |
| 审计会话 | 审计不与施工共用会话；Owner 私聊不进主控委托的那份审查 |
| commit 归属 + 改动落点 | 哪一回合交回哪几个 commit；落在工作区还是分支——**由主控定夺，产品不定死** |
| 停止 | 你喊停就停 |
| 断线与未知 | 不假装，如实写 `待核对` / `未知` / `已请求停止` |
| 工具收口 | 按 `<local>\Grok Worker Provider` 那六样（spawn 前断言启动契约、两级家目录、硬拒默认家目录、路径围栏、项目权威预检、35 行钩子）。**不建统一工具网关** |
| 进程生命周期 | 席位常驻，不每回合换。来源归属用 `pid + processStartTicks` 加 receipt fence |

---

## 五、还没对过的

19 块里这几块本轮没碰：资料与已确认决定、断线恢复与导出、首次使用、项目与工作边界、输入的插话与撤回。
`docs/design/GOGO-施工日志.md`（Owner 给的那批视觉来源）和 `GOGO-第一枪-API-界面入口核对.md` 本轮没引用过。

**研究已搬进仓库**：`docs/research/gogo-ui-2026-09-13/`（七份，字节未改）。**U01–U18、M01–M22、组件与设置编号的出处是 `研究覆盖与缺口.md`**，里面不少标着「实现待审／证据不足」——不能当成已实现。被 Owner 2026-09-14 冻结的三份（双模式实施方案、改版研究总图、整合产品方案）故意没搬。
D07（远程控制、公开托管分享、账号轮换、桌面宠物、并发阈值、复杂实验）Owner 2026-09-16：先挂着。

---

## 六、定死的规矩

- **Owner 直接对接三个**：主控（主窗）、本项目审计（侧窗）、全局秘书长（左栏独立位置，也能拉进侧窗）。其余席位对主控负责，够不到你。
  **这是一把筛子：** 凡是「需要你同时管好几个 agent」才成立的设计，在这儿不成立——研究里那些是照多 Agent 产品写的。
  **但别把筛子当铁律**：侧窗的审计和秘书长是真的对接面，不是要被筛掉的东西。
- **席位向主控负责，你只是监督查看。** 席位卡住、失败、重试都由主控处置，界面如实显示，不变成你的待办。
- **只有 CLI 的真实问题能占用你。** 其余一律安静。**左栏行上写「待处理」的也只有它**——同一条规则的两个面。
- **施工抓不到**，施工的事一律走主控。旁聊只对秘书长和本项目审计开。
- **不动 gogoke 现有 UI，只加。** 中顶、设置各节、面板三个 tab 一律保留。
- **首版只做代码生产**，其余先不管。
- **长期愿景**：说一句「做一个生产级系统」就一路跑到交付。首版不按这个尺度设计，但任何一处都不得假设 Owner 一直看着。

---

## 七、2026-09-26 Owner 补充（旁聊、秘书长、角色与实例）

- **旁聊是一个窗口，不是一个岗位。**
  - 在这个窗口里选"角色 + 实例"来聊，跟 Codex 桌面端的旁聊（`/side`）一模一样，只是把"选模型"换成了"选角色 + 实例"。
  - 审计和秘书长不只绑在这一个旁聊窗口上，它们在别处也照常存在、照常工作。
- **旁聊要 fork 主控的上下文**，也就是项目的上下文。不 fork 就没法真的帮你解决问题。
  - **旁聊机制直接照搬 Codex 的侧边聊天，不需要"选中一句再递过去"这一步。**（2026-09-26 Owner 更正，取代[要什么](GOGO-要什么.md)第 7 条里"只把选中的那一句递给主控"的写法。）
  - **旁聊默认保留，可以先归档再删除。保留的只是旁聊自己新产生的内容**，fork 和同步进来的主控上下文不重复保存，只记引用位置。（2026-09-26 Owner）
  - **旁聊能持续更新主控的上下文**：主控后来的进展会不断同步进旁聊，但同样只作参考，不执行其中的指令。（2026-09-26 Owner）
  - **旁聊机制必须通用，不能拿其中哪一家 CLI 的功能来当实现。**主控上下文来自 gogoke 自己的厂商中立账本，各家的原生 fork 只能当优化。（2026-09-26 Owner）
  - fork 出来的旁聊不是正式审计。正式审计仍然必须是干净会话（第六条那几条规矩不变）。
- **秘书长不只是一个称呼**，它是你与主控、审计之间的沟通桥梁。通过秘书长跟主控对接是基本操作。
  - 它还有很多别的职能，对标 Grok Bot 的"幕僚长"（Chief of Staff）：统筹、管理其他 agent。
- **旁聊能实时看到本项目所有线程的上下文**（需要时去读），看不到别的项目；在旁聊里选秘书长时也一样只看本项目。**自动同步进旁聊的只有主控的上下文。**（2026-09-26 Owner）
- **秘书长能实时看到所有项目、所有线程的上下文**，不只读上报的摘要。它往某个项目转达时，不把别的项目的细节带进去。（2026-09-26 Owner）
- **产品只提供基础设施，不定死用法**：设几个岗位、设多少道审计、流程怎么走，是产品化之后用户自己的用法。产品提供的是：可配置的席位、可配置的调用权限表（由宿主强制执行）、关口原语（准或打回、打回上限、升级）、阶段状态、流转记录。（2026-09-26 Owner）
  - Owner 自己的用法（分最小实现模型阶段和正式阶段，每个阶段都是：需求采集与调研 → 设计大纲 → 审计 → 施工方案 → 审计 → 施工包 → 审计 → 施工 → 审计 → 合并 → 总审计 → 交付）只作为默认模板示例。
- **主控下辖的角色不能直接找你。**
- **角色体系要完善，可以借鉴"三省六部"**（[cft0808/edict](https://github.com/cft0808/edict)，MIT）：分拣、规划、审核封驳、派发、执行，分权制衡。
- **两个管理面**（2026-09-26 Owner）：
  - **实例管理在项目外，属于全局**，放在设置里：账号、CLI 安装与登录、并发、额度来源。
  - **席位管理在项目内**，就是侧栏原来叫"角色管理"的那一页：这个项目有哪些席位，每个席位的职责、权限，以及当前由哪个实例担任。
  - 换实例时席位不变，席位的账本和记忆跟着席位走。
  - **职责定义用"先有模板、再微调"的方式**：新建席位时从模板复制一份，之后各项目可以自己改，不会影响其他项目。
- **同一个实例可以开多个会话，同时服务多个角色或多个项目**，做好隔离就行。参考 ChatGPT 的 Project：项目独享自己的上下文。
- **做法是"拼 + 补全"**：拿开源内核来拼、来改，内核整体也好，零件也好，都可以；拼不上的缺口自己补全。不局限于只拿成熟整体，也不必从零写。（2026-09-26 Owner）
- **沿用 gogoke 现在的 Tauri 壳**，界面和 Rust 宿主都保留。借来的零件拼进这个壳：TypeScript 写的可以作为 Node 进程挂在宿主下面运行。不换成 Orca、Emdash 这类 Electron 应用。（2026-09-26 Owner）
- **可以拿几个开源内核来拼、来改，不必从零写。**Orca（[stablyai/orca](https://github.com/stablyai/orca)，MIT）和 Hermes Agent（[NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent)，MIT）只是候选之一，哪个更像、更合适要先研究，还没有选定。
- **外部调研找到的东西不是拿来直接用的。**开源协议允许的，可以借内核、搬代码，也可以只借思路；不能因为外部材料怎么写，就把我们自己的用途收窄。
