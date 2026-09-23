# gogoke 本体 × 整合设计 · 全量互审

2026-09-17。两边都扫，逐模块对逐节。

**范围**：gogoke 本体 = `apps/desktop/src-tauri/src`（约 28,000 行）+ `apps/desktop/src`（约 72,000 行）。
**排除** `src-tauri/src/codex/`（1519 行）与前端里纯属 app-server 协议渲染的部分——那是一家的协议形状，不是产品本体。

---

## 一、gogoke 已经有、整合设计没提或说反了（十二处）

| # | 模块（行数） | 它实际在做什么 | 设计里的状态 | 要改成 |
|---|---|---|---|---|
| 1 | `bin/gogoke_daemon` + `daemonctl`（4902）、`backend`（1437）、`remote_backend`（515） | **宿主已经存在**：独立 daemon 进程，自带 rpc、文件策略、git、codex 参数与 home 管理；`remote_backend` 把调用转发到它 | 设计通篇写「gogoke 自己的宿主要产生这些事实」，读起来像要新建 | **不是新建，是在现有 daemon 上加。** 每条「宿主要产生」都要落到 `backend`／`gogoke_daemon` 的具体位置 |
| 2 | `tailscale`（1873：core / daemon_commands / rpc_client） | **远程访问已实现**，能从别处连到这个 daemon | **D07 把「远程控制」列为暂不纳入** | **砍的是一个已经在跑的东西。这条要重判**——是关掉、是保留不宣传、还是纳入 |
| 3 | `dictation`（Rust 1606 + 前端 435，含 `useDictationModel`、`useHoldToDictate`、`DictationWaveform`） | **语音输入已实现**，而且有本地模型选择 | 整合设计**零提及**；Owner 立过规矩「语音输入走各家 CLI 的原生输入，不用本地模型」 | 要么承认现状（已有本地模型），要么按规矩改。**不能继续装作没有** |
| 4 | `notifications`（前端 975 + Rust 56）：`useAgentResponseRequiredNotifications` / `useAgentSoundNotifications` / `useAgentSystemNotifications` | 三套通知；系统通知的门是：**未开启不发、席位线程默认不发（有 `subagentNotificationsEnabled` 开关可开）、时长不够不发、窗口聚焦不发、1500ms 去抖** | §6 写「只有 CLI 真实问题能占用你；席位不提醒」 | **默认行为其实一致**（席位默认不通知）。要补的是：那个开关的存在、以及声音通知归哪一类。说成「零」是错的 |
| 5 | `shared/agents_config_core.rs`（1026）：`MANAGED_AGENTS_DIR="agents"`、`multi_agent_enabled`、**`agent_max_depth`** | **已在管一组 agent 的配置**，含**最大深度** | §8 说角色与席位「整个没有」；§16.3 我把「循环保护」当成要新做的 | `agent_max_depth` **就是**我从 V0.12 收的循环保护。角色管理不是零起步，是在这份配置上长 |
| 6 | `shared/local_usage_core.rs`（840）+ `local_usage.rs` | **本地用量统计已有**：按天扫会话，input / cached / output / total，近 7 天汇总 | 设计把「用量如实」写成要新建 | 不是新建。**要加的是来源标注**——这是本地累计，不是官方剩余额度 |
| 7 | `rules.rs`（162）：`append_prefix_rule`、`format_prefix_rule`、文件锁 | 写的是 **Codex 的 always-allow 命令前缀规则**，不是项目规范 | 我一度以为它是项目规则 | 正好对上我收的「`allow_once` ≠ `allow_always`，语义不许改写」——**gogoke 已经在写 always 规则了**，这条规矩有落点。§10 的项目规则快照仍然是缺的 |
| 8 | `workspaces`（Rust 1827 + 前端 5260）、`shared/worktree_core.rs` | **worktree 支持已有** | §10「落点在工作区还是分支，由主控定夺」没说底下用什么 | 落点的底座是现成的 worktree 能力 |
| 9 | `tray.rs`（701） | 系统托盘 | §2「关窗 ≠ 退出」没说靠什么 | 托盘就是它的落点 |
| 10 | `update`（前端 656 + `gogoke_update.rs` 775） | 自更新已有，含更新主机白名单 | 设计零提及 | 老 GOGO `31` §5「升级永远由人点」要对到这里 |
| 11 | `apps` + `appMentions`（344） | **`@` 提及已实现**（提的是 app） | §1.4 写「正文里的 `@` 只是文本，不触发派工」 | 要说清是**哪个 `@`**：现有的 app 提及是产品自己的输入辅助；规矩针对的是**模型输出里的 `@`** |
| 12 | `plan`（60）、`skills`（114）、`models`（440）、`prompts`（841 + `prompts_core.rs` 506）、`debug`（252）、`home`（1079） | 计划面板、技能、模型目录、提示词管理、调试面板、首页（含用量视图） | 设计**零提及** | 至少要在「不动现有 UI，只加」里点名保留；`plan` 尤其要看——设计说不固化「设计→计划→施工」阶段，而这里有个计划面板 |

---

## 二、整合设计要的、gogoke 本体确实没有（核过，这些是真缺）

| 要的 | gogoke 现状 |
|---|---|
| 秘书长在左栏的独立位置 | 无 |
| 项目 ↔ 主控／审计的固定绑定 | 无。`ThreadSummary` 有 `subagentRole`，但那是 Codex 子 agent 的角色字段，**不是项目级绑定**；改造后席位也不再是线程 |
| 对话行上的状态字 | **待处理已有**（`Sidebar.tsx` 的 `pendingUserInputKeys`）；**在运行**有（`working-spinner`）；**改动就绪**没有，要 commit 归属 |
| 未读（完成未看） | 无。`ThreadSummary` 没有 unread 字段 |
| commit ↔ 回合 的归属 | 无。`tool.changes[]` 有路径与 diff，**没有 SHA** |
| 旁聊与递话 | 无第二条对话通道；面板只有 Git／Files／Prompts |
| 角色管理页 | 无（`agents_config_core` 是 Codex 的 agent 配置，不是项目内角色） |
| 实例管理页 | 无（`account.rs` 221 行有账号概念，没有实例页） |
| 工作树（钉底、发散连线、归一折一行） | 无。collab 项按流内联渲染 |
| 五家适配 | 只有 Codex。`src-tauri/src/` 里 claude／anthropic／grok／xai／gemini 零引用 |
| 项目规则快照（AGENTS.md 等） | 无。`rules.rs` 管的是命令前缀审批规则 |

---

## 三、真冲突，要 Owner 定（四条）

### 1. ~~D07 砍远程控制 vs `tailscale`~~ —— **已定：暂时关掉，封入口不删码**（Owner 2026-09-17）

D07 把「远程控制、公开托管分享、账号轮换、桌面宠物、并发阈值、复杂实验」列为暂不纳入，Owner 2026-09-16 说先挂着。
**但 gogoke 本体里远程访问是实现好的**（`tailscale/core.rs`、`daemon_commands.rs`、`rpc_client.rs` + `remote_backend` 515 行）。
「暂不纳入」对一个还没有的东西是延期；对一个已经在跑的东西是**拆除**。三条路：关掉、保留但不宣传、纳入并按项目隔离重新审。

### 2. ~~语音规矩 vs `useDictationModel`~~ —— **已定：暂时关掉，封入口不删码**（Owner 2026-09-17）

Owner 立过：**语音输入走各家 CLI 的原生输入，不用本地模型**。
gogoke 的 `dictation` 有 1606 行 Rust + `useDictationModel`（本地模型选择）+ `useHoldToDictate` + 波形组件。
要么承认现状，要么按规矩改——**但设计里一个字都没写，等于假装它不存在**。

### 3. 「`@` 只是文本」vs `appMentions` —— **已定：写清区别，并且要做隔离实测**（Owner 2026-09-17）

设计 §1.4：正文里的 `@` 只是文本，不触发派工。
gogoke 有 `appMentions.ts` —— 一套 `@` 提及机制（提 app）。
两者不必打架，但**必须写清区别**：产品输入框里的 `@` 是给你用的选择器；规矩管的是**模型输出正文里的 `@`**。不写清，施工的人会把现有的 `@` 一起禁掉。

**Owner 2026-09-17 追加**：光写清不够，**还要做隔离**——**项目施工角色／席位不应该看到 Owner 与主控的对话内容**。
施工拿到的只能是主控构造的任务包（目标、范围、禁止项、资料引用、成功条件）。
**而且这条必须实测**。注意理由已经变了：**改造后是 CLI 派活给 CLI**（Owner 2026-09-17），两个独立进程两份独立会话——隔离靠的是**会话怎么管**（整合设计 §8.5），不是某家子 agent 的语义。传了任务包不等于隔离成立。
实测办法：给施工席位一句只在主对话里出现过的话，看它答不答得上来。**测出来之前界面上不许写「施工看不到」。**

### 4. ~~`plan` 面板~~ —— **已核，不是冲突**：`PlanPanel.tsx` 60 行，只读 `TurnPlan` 的步骤与状态算进度，**没有任何确认／批准／推进按钮**，无活动计划时自动收起。是模型计划输出的显示，不是产品阶段。留着不动

设计说不把「设计 → 计划 → 施工」固化成所有项目的阶段（研究也这么说）。
gogoke 有 `PlanPanel`。要说清它是**Codex 自己的计划输出的显示**，还是一个产品阶段——前者留，后者与规矩冲突。

---

## 四、措辞要改的两处（不是冲突，是会误导施工）

1. **「gogoke 自己的宿主」通篇读起来像要新建一个**。实际有 `bin/gogoke_daemon`（4902 行）+ `backend`（1437）+ `remote_backend`（515）。
   第〇节和 §15 分批里每一句「宿主要产生 X」，都应指到 `backend`／`gogoke_daemon` 里的具体位置。
2. **几处「缺」其实是「已有，要加的是一小块」**：用量（已有统计，缺来源标注）、worktree（已有能力，缺回合归属）、托盘（已有，缺"关窗≠退出"的说明）、自更新（已有，缺"升级由人点"）。

---

## 五、这轮核过的事实

| 事实 | 出处 |
|---|---|
| gogoke Rust 侧约 28,000 行；前端约 72,000 行，25 个 feature | `wc -l` 全量 |
| `src-tauri/src/` 里 claude／anthropic／grok／xai／gemini **零引用** | grep -il |
| 独立 daemon 存在，含 rpc、文件策略、git、codex home | `bin/gogoke_daemon.rs` 的 mod 清单 |
| 系统通知对席位线程**默认不发**，有开关可开 | `useAgentSystemNotifications.ts:125-145` |
| `agent_max_depth` 已存在 | `shared/agents_config_core.rs:21` |
| 本地用量按天统计 input/cached/output/total 与近 7 天 | `shared/local_usage_core.rs:110-130` |
| `rules.rs` 写的是命令前缀 always-allow 规则 | `rules.rs:14,97` |
