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

## 4. 待定

- **选哪条路由 Owner 决定：**
  1. 补齐我们自己的；
  2. 整体借 Paseo；
  3. 只借 Paseo 的适配层。
- **不管选哪条，都要先验证两件事：**
  - Paseo 的子进程在 Windows 11 智能应用控制下能不能正常运行；
  - Grok Build 的 ACP 实际声明了哪些能力（`loadSession`、用量）。表里 Grok 这一行的 △ 取决于这一点。
