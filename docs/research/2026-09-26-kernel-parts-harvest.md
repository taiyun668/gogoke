# 零件拆解：能直接搬、要改、只借思路

- 日期：2026-09-26
- 背景：Owner 已定"拼 + 补全"，并且继续用 gogoke 的 Tauri 壳（界面加 Rust 宿主）。本文把[内核候选对照](2026-09-26-kernel-candidates-architecture.md)里要借的零件读到文件和函数这一级，判断三件事：依赖什么、能不能干净地拆出来、放进 Tauri 壳时 Windows 11 智能应用控制会不会拦。
- 方法：用 GitHub API 只读源码，没有运行任何代码。智能应用控制能不能过，这里只做静态判断：看有没有原生二进制或原生插件、有没有平台相关的进程操作。真正的结论要在 Owner 的机器上实测。

---

## 1. 结论一览

| 零件 | 来源 | 判断 | 理由 |
|---|---|---|---|
| **旁聊（Codex 这一家）** | Codex app-server 协议 | **直接可做** | 协议本身就支持，gogoke 本来就在调 app-server，不需要新增任何二进制。详见 §2 |
| 收件箱（对方空闲立即投递，正忙排队） | claw-orchestrator `src/inbox-manager.ts`（178 行） | **直接搬** | 只依赖自己的类型和常量，纯 TS。里面有防伪造发件人的转义，一并保留 |
| 跨引擎交接（文字重放） | claw-orchestrator `src/handoff.ts`（189 行） | **直接搬** | 只从 `openai-compat.js` 引了一个 `fenceHistoryTags` 函数，纯 TS |
| 按名字寻址的会话管理 | claw-orchestrator `src/session-manager.ts`（3,885 行） | **要改，只取骨架** | 跟工作流、自动循环、ultraapp、council 等十几个模块耦合。用了原生插件 `re2`（只在 `grepSession` 里用，可以去掉） |
| 各家 CLI 会话协议（stream-json、app-server、ACP、agy、OpenCode 等） | claw-orchestrator `src/persistent-*-session.ts` | **要改：协议解析搬，进程管理换掉** | 进程管理是按 POSIX 写的：`spawn` 时 `detached: true`（在 Windows 上会新开控制台窗口），`process.kill(-pid)` 整组杀进程（Windows 不支持），`spawn` 不走 shell（Windows 上起不了 npm 的 `.cmd` 包装）。进程的启动和托管应交给我们 Rust 宿主已有的 Job 托管 |
| 会话日志（epoch、重启对账） | Orca `src/main/native-chat/agent-session-journal/*` | **要改** | 用的是 Node 内置的 `node:sqlite`，没有原生插件，对智能应用控制友好。但它依赖 Orca 自己的数据库封装和一整套模块，要连依赖一起摘 |
| 人工决策闸 | Orca `coordinator-decision-gates.ts` | **借思路** | 规则很清楚：工人发消息开闸，任务被阻塞，协调器不自动解，只由人解。代码绑死在它的协调器数据库上，自己写更省事 |
| 持久信箱（按角色投递、消费者代号、重放） | Orca `db/messages/*` | **借思路** | 同上，跟它的 SQLite 表结构绑定 |
| 读各家额度（OAuth 用量接口） | Orca `src/main/rate-limits/*-fetcher.ts` 等 | **要改** | 走 HTTP 接口的部分是纯 TS，可以搬。靠读终端屏幕的那部分依赖 `node-pty` 原生插件，不要 |
| 多账号托管家目录 | Orca `src/main/codex-accounts/*`、`claude-accounts/*` | **借思路** | 文件很多，大量是 WSL 迁移之类的历史包袱 |
| 额度到了自动切账号 | —— | **不存在** | **更正**：之前说 Orca 能在额度快到时自动切账号，这是错的。那是一个**还开着的功能请求**（stablyai/orca #20512），代码里没有。真正做了轮换的是 Gas Town，但它靠读终端屏幕 |
| Profile / Bot 名册（长期身份加永久对话） | Hermes（Python） | **借思路** | 语言不同，搬不了代码，照它的设计自己实现 |
| Curator（空闲时整理记忆） | Hermes（Python） | **借思路** | 同上。确定性清理默认开、LLM 合并手动开、钉住的不动，这几条规则直接沿用 |
| 调用权限矩阵加封驳 | 三省六部 Edict | **借思路** | 本质是一张"谁能调谁"的表，加上"必须经过审议"和"打回最多几轮"的规则。依赖 OpenClaw，不搬代码 |
| 两层作用域、角色模板和身份分开、升级链 | Gas Town（Go） | **借思路** | 理念和数据模型，执行层（tmux）不要 |

## 2. 旁聊：Codex 这一家已经能实现 Owner 定的形态

出处：`openai/codex` 里的 `codex-rs/app-server-protocol/src/protocol/v2/thread.rs`。

- **fork 主控上下文**：`thread/fork` 的参数 `ThreadForkParams` 包括：
  - 要 fork 的线程；
  - 从哪一回合截断（`last_turn_id` / `before_turn_id`）；
  - 模型；
  - `sandbox`、`approval_policy`，用来设成只读；
  - `developer_instructions`，用来放"这是旁聊、之前的内容只作参考"的说明；
  - **`ephemeral`**。
- **可以持久，也可以丢弃**：`ephemeral = true` 就是 Codex 自己旁聊的做法，不落盘，关掉即丢；设成 `false`，fork 出来的就是一条正常保存的线程，可以持久。
- **持续同步主控上下文**：协议里有 `ThreadInjectItemsParams`，作用是"把原始条目追加到这个线程模型可见的历史里"。主控每走一步，就把新的回合作为参考材料注入旁聊，同时补一段"以上只作参考、不执行"的边界说明。
- 参考边界和"不修改"的具体措辞，可以直接沿用 Codex TUI（`codex-rs/tui/src/app/side.rs`）里的写法。

**其他厂商**：
- Claude 可以用 `--resume` 加 `--fork-session` 原生 fork；持续同步只能在下一回合把主控的新进展作为参考消息带进去。
- 其他家先用 claw-orchestrator 的文字重放来 fork。

这部分要逐家实测。

## 3. 放进 Tauri 壳的方式

- **Rust 宿主**继续负责进程的启动、托管和停止，也就是 R2-06a 的 Job 托管。借来的会话协议代码不自己管进程，只负责解析各家 CLI 的输入输出。
- **借来的 TS 零件**（收件箱、交接、协议解析）放进一个 Node 进程，挂在宿主下面运行。Node 是官方签名的程序，纯 JS 包不涉及原生二进制。
- **要避开的原生插件**：`re2`、`node-pty`、`better-sqlite3` 这类。它们的预编译 `.node` 文件没有签名，智能应用控制可能会拦。存储用 Node 内置的 `node:sqlite`，或者放在 Rust 宿主这一侧。
- Rust 这一侧的新代码照现有规矩，在云端构建。

## 4. 许可证

搬代码的几家（claw-orchestrator、Orca）自有代码都是 MIT。搬的时候保留原版权声明，记进 `THIRD_PARTY_NOTICES.md`。Codex 是 Apache-2.0，这次只调用它的协议，不搬它的代码。

## 5. 还没做的

- 以上是静态判断，真正能不能过智能应用控制，要在 Owner 的 Win11 机器上实测。
- claw-orchestrator 各家会话里的协议解析部分，要逐家拆开，确认跟进程管理代码能切干净。
- `ThreadInjectItems` 对应的 JSON-RPC 方法名，以及注入的条目格式，要在本机 Codex 版本上实测确认。
