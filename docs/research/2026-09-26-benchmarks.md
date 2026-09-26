# 对标清单：每个效果找谁来比

- 日期：2026-09-26
- 性质：候选对标清单。只核实了"它存在、它声称做这件事"，还没有读架构。
- 用法：对照[从效果倒推的问题与覆盖](2026-09-26-effect-decomposition-and-coverage.md)里的 E1–E14，每个效果挑出最能回答它那个调研问题的对象。开源的用来读源码，闭源的读官方文档。
- 星数、许可证、最近推送日期，都是 2026-09-26 通过 GitHub API 读到的。闭源产品附官方文档链接。
- 另外有两个目录可以查漏：[awesome-agent-orchestrators](https://github.com/andyrewlee/awesome-agent-orchestrators)（CC0，约 90 个编排器），以及 [第一批底稿 §3.1](2026-09-25-next-stage-research-base.md)。

---

## 1. 核心对标：覆盖效果最多，值得整体读一遍架构

| 对象 | 规模 | 为什么是核心 | 主要对应 |
|---|---|---|---|
| **Gas Town**（[gastownhall/gastown](https://github.com/gastownhall/gastown)，Steve Yegge） | 18.2k★，MIT | 有 **Mayor**：你只跟它说话，它是全局协调者。项目叫 Rig。按角色分工：Polecat 是临时工人，Crew 是长期协作者，Witness 负责监督，Refinery 管合并队列。它明确主张"**会话是牲口，身份长期存在**"，所有状态（身份、派工、编排）都放在 Git 里的 Beads 上。跟我们的"会话是缓存，席位是身份，事实在 Git"几乎一样 | E1 E2 E3 E9 E10 E14 |
| **Paperclip**（[paperclipai/paperclip](https://github.com/paperclipai/paperclip)） | 85.8k★，MIT | 定位是"零人公司"的控制面。有组织架构图和角色，**人和 agent 的权限、边界**，按公司、agent、项目、目标、厂商、模型分别设预算，超支时硬停。靠心跳驱动，同一任务的上下文在两次心跳之间延续。任何运行时的 agent，只要能接收心跳就能加入 | E2 E4 E11 E12 E13 |
| **Multica**（[multica-ai/multica](https://github.com/multica-ai/multica)） | 51.4k★，许可待核（业务底稿记录的是 Apache 加托管和品牌限制） | 让人和 agent 作为一个团队协作，业务调研里 Ravenopus 用它来协调 | E2 E3 E4 |
| **Orca**（[stablyai/orca](https://github.com/stablyai/orca)） | 78.6k★，MIT | 管一队并行 agent 的桌面工作台。多账号托管，**额度预警后自动切换账号**（按"最紧的窗口到 N%"提前切，见其 issue #20512）。守护进程带冷恢复和检查点 | E5 E10 E11 |
| **Paseo**（[getpaseo/paseo](https://github.com/getpaseo/paseo)） | 18.6k★，自有代码 Apache-2.0 | 已查到架构层（[执行层对照](2026-09-26-execution-layer-capability-table.md) §3b），作为对照基线 | E10 E11 |
| **Claude Code agent teams**（[官方文档](https://code.claude.com/docs/en/agent-teams)） | 闭源，实验功能 | 一个队长加几个队友，共享任务清单，队友之间可以直接发消息。**人可以直接找任何一个队友**，这一点跟我们"席位不直接找你"的规矩正好相反，值得对照 | E3 E4 E14 |

## 2. 按效果补充的对标

### E1 侧聊与递话
- **Cursor Side Chats**（[更新日志](https://cursor.com/changelog/side-chat)，v3.11，2026-07）：侧聊是一条可以长期保留的完整对话，不打断主对话，事后用 @ 把它拉回主对话。这跟"递话"最接近。
- **Claude Code `/btw`**：能读当前对话，但不写进主对话的历史，也不打断主任务。
- **VS Code `/btw`**：侧聊**和主对话共用提示词缓存**，这点直接对应"共享上下文而不重算"。
- **ChatGPT Projects 的"仅项目记忆"**（[帮助中心](https://help.openai.com/en/articles/10169521-using-projects-in-chatgpt)）：项目里的对话不读全局记忆，也不读其他项目；项目里学到的东西只留在项目里。只能在创建时设定，设定后不能改回。这是**全局与项目两层隔离**现成的产品形态。
- Gas Town 的 Mayor：对应全局秘书长。

### E2 角色与实例解耦
- **Codex custom agents**（`.codex/agents/*.toml`）和 **Claude Code subagents**（`.claude/agents/`）：角色文件里写着模型和推理强度，角色和实例部分解耦。本仓库的路由文档已经在用前者。
- **oh-my-codex**（[Yeachan-Heo/oh-my-codex](https://github.com/Yeachan-Heo/oh-my-codex)，33.4k★，MIT）和 **oh-my-claudecode**（39.4k★）：在单一厂商上组团队、定角色。
- **Untrivial agent-orchestrator**（[Untrivial-ai/agent-orchestrator](https://github.com/Untrivial-ai/agent-orchestrator)，12.4k★，Apache-2.0）：从规划一直管到合并，号称能接任何一种 agent。

### E3 只跟主控说话 / E4 主控自主拉席位
- **agentchattr**（[bcurts/agentchattr](https://github.com/bcurts/agentchattr)，1.5k★，MIT）：本地聊天室，agent 之间可以互相 @。它是设计 22 的来源之一，对应 @ 路由和房间。
- **cumora**（[yetone/cumora](https://github.com/yetone/cumora)，3.9k★，MIT）：团队聊天，agent 是一等成员。
- **AgentTeams**（[agentscope-ai/AgentTeams](https://github.com/agentscope-ai/AgentTeams)，5.7k★，Apache-2.0）：强调"人在环中"的任务协调。

### E9 交接
- **CASR**（[Dicklesworthstone/cross_agent_session_resumer](https://github.com/Dicklesworthstone/cross_agent_session_resumer)，122★）：跨家转录会话，零件拆解 §2 已看过。
- **AgentBridge**（[raysonmeng/agent-bridge](https://github.com/raysonmeng/agent-bridge)，361★，MIT）：让 Claude Code 和 Codex 在同一个会话里对等协作，额度到边界时交接。
- **Amp Handoff**（闭源）：9 月 23 日报告引用过，还没核实。

### E11 多厂商、额度调度
- **Orca**：见上。自动切换账号是一个可选策略。
- **neomax-orchestrator**（[NeotaskInc/neomax-orchestrator](https://github.com/NeotaskInc/neomax-orchestrator)，1★，MIT，很新）：自称按额度余量选账号，同等时优先用快要重置的额度。星数极少，只能当思路参考。
- **claude-code-router**（[musistudio/claude-code-router](https://github.com/musistudio/claude-code-router)，37.4k★，MIT）：本地控制面，在多个模型之间路由。
- **Paperclip**：按多个维度设预算并硬停，是花费这一侧的对标。

### E12 越做越少找我
- **Paperclip**：人和 agent 的治理与权限边界。
- **HumanLayer**（[humanlayer/humanlayer](https://github.com/humanlayer/humanlayer)，在 awesome 目录里）：专门做"什么时候必须找人批准"。星数未核。
- **AsDecided**：决策登记，零件拆解 §3 已看过。

### E5 多项目与状态汇总 / E14 并行
- **Gas Town**：Refinery 管合并队列，所以并行工作不会互相冲突。
- **Happier**（[happier-dev/happier](https://github.com/happier-dev/happier)，1.7k★，MIT）：网页、桌面、手机三端客户端，有编排器，也有分叉。
- **Superset**、**OpenMausBot**（[milind-soni/OpenMausBot](https://github.com/milind-soni/OpenMausBot)，3.6k★）：底座拆解 §1 已看过文件树。

---

## 2b. 外部整理补充的对标（2026-09-26 核实）

Owner 转来一份外部整理，是按"编码 CLI 的岗位运行时"这个思路找的对标，其中好几个是上面漏掉的。核实结果如下：

| 对象 | 核实结果 | 对应 |
|---|---|---|
| **claw-orchestrator**（[Enderfga/claw-orchestrator](https://github.com/Enderfga/claw-orchestrator)） | 582★，MIT，2026-01 创建，12 名贡献者。源码里确实有：命名会话（`session-manager.ts`）；跨会话收件箱（`inbox-manager.ts`，对方空闲就立即投递，正忙就排队，与"递话排队"相同）；跨引擎交接（`handoff.ts`）；八家常驻会话适配：Claude、Codex（两种）、Cursor、OpenCode、Grok、Antigravity、Gemini、自定义；另有 council、fanout、预算、熔断、人工闸节点、文件锁。**交接是把用户与助手的文字对话重放进新引擎，上限约 6 万 token，工具轨迹和推理不带**，源码注释明说不去改对方的原生会话文件。Windows 支持未见说明，安装脚本是 bash | **E1 E2 E9 E10 的核心对标**，上面漏掉了 |
| OpenClaw（[openclaw/openclaw](https://github.com/openclaw/openclaw)） | 39 万★，MIT（OpenClaw Foundation），TypeScript。它是通用的 AI 助手网关，不是编码工作台。claw-orchestrator 能作为它的插件装进去 | E1 秘书长的对标 |
| Emdash（[generalaction/emdash](https://github.com/generalaction/emdash)） | 5.8k★，Apache-2.0，YC W26。多 CLI、worktree、diff；树里有 12 个带 win 字样的路径 | E5 E14 舞台层 |
| agent-git（[Einsia/agent-git](https://github.com/Einsia/agent-git)） | 364★，MIT。把 agent 会话做成可以版本化的对象 | E10 |
| 交接和信箱类小库：waybill（95★，Apache）、agents-can-communicate（104★，MIT）、claude-codex-handoff（40★，MIT）、handoff（20★）、magents（6★）、shiftlog（0★）、plano（6★） | 都存在，但星数很小，只能当思路 | E9 E1 |
| AgentsRoom | 商业闭源桌面应用。GitHub 上同名的 `agents-room`（6★）是另一个项目 | 只能看产品形态 |

## 3. 建议先读哪几个

读架构最省力的，是一个对象能回答很多个效果。建议先读四个：

1. **Gas Town**：E1、E2、E9、E10 都在它的核心设计里，而且理念跟我们最接近。先读它，最能检验我们的会话架构有没有漏掉什么。
2. **Paperclip**：角色、权限、预算、治理，对应 E2、E4、E11、E12。
3. **Cursor Side Chats** 加 **ChatGPT 仅项目记忆**：E1 的产品形态，读官方文档即可。
4. **Orca**：多账号和按额度切换，对应 E11。

每读一个，都按 E1–E14 的调研问题去记：数据模型是什么、谁做决定、状态放在哪，每条注明出处。读完就更新覆盖表。

## 4. 已知的局限

- 这份只核实了"存在且声称做这件事"，每一条说法在读源码之前都可能不成立。Paseo 那次就是先例。
- 星数在 2026 年涨得很快，不代表成熟度。Paperclip 三周就涨到三万星。
- 闭源产品只能读文档，读不到实现。
