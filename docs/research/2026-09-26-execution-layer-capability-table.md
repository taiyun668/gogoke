# 执行层能力对照：我们的实现 vs Paseo（首版五家）

- 日期：2026-09-26
- 性质：事实核查。它回应[调研评判 §4](2026-09-26-research-evaluation.md) 留下的那项检查："如果 Paseo 在这五家上明显更全，而且补齐的成本高于接入它的成本，就改为借底座。"
- 方法：按源码逐项核对，只读，不运行。每一格都注明代码依据。
- 版本：
  - 我们这边分两份：`seat-runtime`（gogo-party 归档，2026-09-18 版，约 5,000 行）和 gogoke 当前的 Codex 通道（`apps/desktop/src-tauri/src/codex`，施工分支 `088fc1e1`）；
  - Paseo 为 `getpaseo/paseo` main `aa3ffaeb`（2026-09-26），自有代码 Apache-2.0。

---

## 1. 先纠正评判里的一句话

评判 §4 写"前三家已经有了"，这句话不准：
- `seat-runtime` 确实接了 Codex、Claude、Grok 三家，但它留在 gogo-party 归档里，**gogoke 当前代码里没有它**；
- gogoke 现在只接了 Codex 一家，走的是 CodexMonitor 带来的 app-server 通道；
- `seat-runtime` 对这三家的支持也不完整，缺多少见下表。

## 2. 对照表

✓ 支持；△ 部分支持或有条件；✗ 没有。

| 厂商 | 能力 | 我们 | Paseo |
|---|---|---|---|
| **Codex** | 启动 | ✓ `thread/start` | ✓ `thread/start` |
| | 续接 | gogoke ✓ `thread/resume`；seat-runtime ✗ | ✓ `thread/resume` |
| | 分叉 | gogoke ✓ `thread/fork`；seat-runtime ✗ | ✓ `thread/fork`，另有 `thread/rollback` |
| | 权限请求 | gogoke ✓（审批请求交给界面）；seat-runtime ✗（`approvalPolicy: "never"`） | ✓ `permission_requested`，可交自动审查子 agent |
| | 用量 | gogoke ✓ `thread/tokenUsage`、`account/rateLimits`；seat-runtime ✗ | ✓ `usage_updated`，另有账号额度读取器 |
| | 中途插话 | ✓ `turn/steer` | ✓ `turn/steer` |
| **Claude Code** | 启动 | seat-runtime ✓ stream-json 长进程；gogoke ✗ | ✓ Claude Agent SDK |
| | 续接 | ✗ | ✓ `resume` |
| | 分叉 | ✗ | ✓ `forkSession`，另可回退对话和文件 |
| | 权限请求 | seat-runtime ✓ `--permission-prompt-tool stdio`（`can_use_tool`） | ✓ `canUseTool` |
| | 用量 | seat-runtime ✓ `rate_limit_event`（订阅窗口与重置时间） | ✓ 每轮用量 + 账号额度读取器 |
| | 中途插话 | seat-runtime ✓（回合进行中写入用户消息） | ✓ `steerActiveTurn` |
| **Grok Build** | 启动 | seat-runtime ✓ ACP `session/new`；gogoke ✗ | ✓ 通用 ACP（目录内置 `grok agent stdio`） |
| | 续接 | ✗ | △ CLI 声明 `loadSession` 时支持 |
| | 分叉 | ✗ | ✗ |
| | 权限请求 | ✗（只在启动时用命令行参数定权限模式） | ✓ ACP `request_permission` |
| | 用量 | ✗ | △ CLI 发 `usage_update` 时有；账号积分由额度读取器取 |
| | 中途插话 | △ 排队，等当前回合结束后作为下一段提示补发 | △ 打断当前回合，换成新提示 |
| **OpenCode** | 全部六项 | ✗ 没有适配 | 启动、续接、权限、用量、插话 ✓；分叉 ✗，但可以回退 |
| **Antigravity** | 全部六项 | ✗ 没有适配 | △ 官方未内置；有第三方插件 `tiezbro/paseo-agy-acp`（Apache-2.0，57★，2026-09 新建），未审计 |

**代码依据**
- 我们：
  - `seat-runtime/src/seat-runtime.ts`（Codex：`thread/start`、`turn/steer`、`approvalPolicy: "never"`，只处理 `requestUserInput`）；
  - `claude-seat.ts`（`--permission-prompt-tool stdio`，`rate_limit_event`，没有 `--resume`）；
  - `grok-acp-seat.ts`（只有 `session/new`、`session/prompt`、`session/cancel`；插话在第 432 行排队补发）；
  - gogoke 的 `src-tauri/src/codex/mod.rs`，以及前端 `ApprovalToasts`。
- Paseo：
  - `packages/server/src/server/agent/providers/` 下的 `codex-app-server-agent.ts`、`claude/agent.ts`、`acp-agent.ts`、`opencode-agent.ts`；
  - 能力开关在各文件的 `capabilities` 对象里；
  - 插话失败时"打断后替换"的逻辑在 `agent-manager.ts` 的 `steerOrReplaceActiveTurn`；
  - Grok 的启动命令在 `packages/app/src/data/acp-provider-catalog.ts`。

## 2b. 会话管理

续接和分叉之外，会话本身怎么管：

| 能力 | 我们 | Paseo |
|---|---|---|
| 列出已有会话 | gogoke Codex ✓ `thread/list`；seat-runtime ✗ | Codex、Claude、OpenCode ✓；ACP 家在 CLI 声明 `sessionCapabilities.list` 时 ✓ |
| 接管在 CLI 里自己开的会话 | gogoke Codex ✓（CodexMonitor 列出本机全部 thread）；其余 ✗ | ✓ 每家都有 `importSession`（Claude 直接读 `~/.claude/projects/*.jsonl`） |
| 读取历史 | gogoke Codex ✓ `thread/read`；seat-runtime 只有自己写的 transcript | ✓ 恢复会话时区分"继续驱动"和"只读历史"两种用途 |
| 重启后接回原会话 | gogoke Codex ✓；seat-runtime △（写了 `state.json` 和 transcript，但没有续接，接不回原生会话） | ✓ 持久化句柄 + 各家续接 |
| 上下文压缩信号 | gogoke Codex ✓（`thread/compact/start`、用量事件）；seat-runtime ✗ | Codex、Claude、OpenCode ✓ |
| 回退（撤回几轮对话或文件改动） | ✗ | Claude 对话和文件都能回退；Codex 能回退对话；OpenCode 能同时回退 |
| 跨厂商转移会话 | ✗ | ✗（这需要 CASR 这类单独的零件） |

会话管理上差距更大。除了 Codex，我们在会话管理上几乎是空白；Paseo 在它原生支持的三家上是完整的。

## 3. 结论

**第一个条件成立：Paseo 在五家上明显更全。**
- 30 格里，Paseo 做到 ✓ 或 △ 的有 28 格（Antigravity 的 6 格全是第三方插件的 △）；我们（两份代码合起来算）只有 12 格，其中 6 格是 Codex。
- 我们在"续接、分叉"上几乎是空白。除 Codex 外，其余四家都没有用量；OpenCode 和 Antigravity 完全没有适配。

**第二个条件，补齐和接入哪个成本高，还不能下结论。**两边的成本如下：
- **补齐我们自己的**：
  - Claude 的续接和分叉，只要加上 CLI 的 `--resume` / `--fork-session` 参数，量小；
  - Grok 补上 ACP 标准里的 `session/load`、`request_permission`、`usage_update`，量中等；
  - OpenCode 要从零写（Paseo 这一个适配就有约 18 万字节）；
  - Antigravity 要从零写；
  - 还要把 `seat-runtime` 从归档里接回 gogoke。
- **接入 Paseo**：
  - 它是 Electron 加 Node 守护进程，gogoke 是 Tauri，宿主边界要重新划；
  - Windows 是它的弱项，上一轮拆解只数到 5 个与 Windows 相关的文件，智能应用控制下的表现也没有验证过；
  - 治理层（权限闸、权威标签）要插进它的守护进程里；
  - 它的每个适配都跟自家的 `AgentSession` 类型和 `agent-manager` 绑得很紧，单个文件就有 13 万到 24 万字节，不能原样摘出来用。

**第三条路：只借它的适配层，不借整个守护进程。**Apache-2.0 允许这样做。gogoke 保留自己的 Tauri 宿主和治理层，把 Paseo 的 provider 适配连同它们依赖的类型一起移植进来，作为执行层的零件。这比评判原先说的"搬零件"范围大，但比整体换底座小。这条路的成本要看适配层和 `agent-manager` 能不能干净地切开，需要先做一次切分评估才能判断。

## 3b. 架构层面：会话在两边分别是什么

上面比的是适配能做什么。这一节比会话在架构里的位置：拿[会话、记忆与做梦架构（候选）](../directions/2026-09-23-session-memory-dream-architecture.md)的 12 个部件，去对照 Paseo 的会话模型。

**Paseo 的会话模型**：
- 守护进程里的 `AgentManager` 管着一个个 agent。一个 agent 就是一个厂商会话，身份和会话是一回事。
- 每个 agent 的记录里有：厂商、原生会话句柄、工作区、标题、标签、最近状态、配置、是否需要人关注、归档时间。父子关系用标签记。
- 每个 agent 有一条规范化的事件时间线。
- 会话之间怎么接，靠三样东西：
  - 用户或 agent 手动调用的 skill：`/paseo-handoff` 让模型写一份交接简报，再开一个零上下文的新 agent；`/paseo-committee` 和 `/paseo-advisor` 用来要第二意见；
  - 对外的 MCP 工具，让 agent 自己去开别的 agent；
  - 工作树隔离。
- 额度只用来显示，不参与调度。

| 我们的部件 | Paseo 有没有 | 说明 |
|---|---|---|
| 1 会话登记簿 | △ | 有 agent 记录和父子标签；没有"席位"这一层：身份和会话绑在一起，没有继承档，也不记接触过的权限等级 |
| 2 健康监测 | △ | 有用量、压缩事件、需要关注的标记；没有健康分级，也没有"建议换或必须换"的输出 |
| 3 切换决策器 | ✗ | 什么时候换、换给谁，由人或 agent 调用 skill 决定，系统本身不做判断 |
| 4 交接包工厂 | △ | handoff skill 让模型按模板写简报；不从提交和检查点机械提取，也不保留死路 |
| 5 交接验证器 | ✗ | |
| 6 前缀与缓存管理 | △ | 能在运行时追加守护进程级指令；没有版本化的固定前缀 |
| 7 写入范围协调 | △ | 工作树隔离；没有写入范围租约 |
| 8 权限闸（管继承） | ✗ | 权限只管单次工具调用，不管"新会话能继承什么" |
| 9 记账与复盘 | ✗ | 只有事件时间线，不记决策和代价 |
| 10 长期记忆库 | ✗ | |
| 11 做梦 | ✗ | |
| 12 用了就回写 | ✗ | |

四档继承方式：
- 原生复制：✓，靠分叉和回退；
- 跨家转录：✗；
- 交接包：△，就是上面那个 skill 模板；
- 零继承：✓，新开一个 agent 就是。

**架构层面的结论**：
- Paseo 管的是"会话本身"，相当于我们架构表里的工作记忆那一层，外加会话的生命周期。我们架构里真正核心的部分它全都没有：会话之上的决策、席位身份、记账、长期记忆、做梦、继承权限闸。
- 所以不管执行层选哪条路，这些都得我们自己做，这一点跟 §12 原本的设想一致：Paseo 做执行底座，gogoke 做上面的治理层。
- 架构上唯一真正的冲突，是**身份和会话的关系**。Paseo 是一个 agent 对应一个会话；我们要的是"席位长期存在，会话只是可以随时丢弃的缓存"。借 Paseo 的话，要在它的 agent 之上再加一层席位登记，把"换会话"做成"同一个席位换了一个新 agent"。它提供的 MCP、标签和父子关系，都可以用来接这一层。
- 如果只借它的适配层（§3 的第三条路），这个冲突就不存在了：会话模型由我们自己定义，适配层只负责把"开、接、分叉、回退、插话"这些动作翻译成各家的协议。

## 4. 待定

- **选哪条路由 Owner 决定：**
  1. 补齐我们自己的；
  2. 整体借 Paseo；
  3. 只借 Paseo 的适配层。
- **不管选哪条，都要先验证两件事：**
  - Paseo 的子进程在 Windows 11 智能应用控制下能不能正常运行；
  - Grok Build 的 ACP 实际声明了哪些能力（`loadSession`、用量）。表里 Grok 这一行的 △ 取决于这一点。
