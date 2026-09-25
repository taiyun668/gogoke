# 零件源码拆解（第二批 B，第二组）

- 日期：2026-09-25
- 性质：事实底稿，不评判、不给建议。
- 方法：通过 GitHub API 只读默认分支的文件树、依赖清单（package.json）、关键文件与 README，没有下载或运行任何代码。
- 星数与许可证见 [第一批底稿 §3.1](2026-09-25-next-stage-research-base.md)。

---

## 1. 网页额度通道

| 项目 | 规模 / 语言 | 浏览器怎么驱动（依赖清单与代码） | 并发与额度 | 与本机工具的连接 | 其他 |
|---|---|---|---|---|---|
| miuuyy/codex-chatgpt-web | 270 文件 / TypeScript（Electron 启动器） | `playwright-core` + `chromium-bidi`，内置浏览器，不依赖系统 Chrome；适配层在 `src/adapters/chatgpt-web/`（`browser-worker.ts`、`model-selection.ts`、`output-validation.ts`、`retry-policy.ts`） | `concurrency.ts`：`MAX_CHATGPT_BROWSER_TABS = 5`；`limits.ts` 从页面识别套餐（`pro_100` / `pro_200`）并读取 GPT-6 Pro、5.6 Pro 的用量，注释写明"只导出稳定的账号标识，不导出会话凭证" | MCP（`mcp-server.ts`、`mcp-main.ts`）；按其 README，经 OpenAI tunnel-client 接回 Codex，并改写 Codex 的 `openai_base_url` | 有专门的压缩交接模块：`compaction-handoff.ts`、`compaction-transaction.ts`、`compaction-continuation.ts`、`native-compaction-control.ts` |
| steipete/oracle | 521 文件 / TypeScript | `chrome-launcher` + `chrome-remote-interface`（CDP）；`@steipete/sweet-cookie`（从浏览器读取 cookie）；另有 `src/gemini-web` | 按其文档：默认 3 个标签页的槽位排队 | MCP server（`@modelcontextprotocol/server`）；API 模式另依赖 `openai`、`@google/genai` | 带 token 计数（`gpt-tokenizer`、`@anthropic-ai/tokenizer`） |
| totec448-spec/chat-on-steroids | 857 文件 / TypeScript（Electron） | 页面扩展配合桌面端；`node-pty` 终端 | 多个 worker 标签页（第一批 §4） | MCP server 与 client 两侧都有（`src/main/mcp/`）；`code-mode-runtime.ts` 用 **QuickJS（WASM）** 在沙箱里跑代码；`tools-desktop-windows.ts` / `-macos.ts` 负责桌面控制 | 依赖 `tree-sitter-bash`（能解析 shell 命令）；有 `tool-approval.ts`、`src/main/codex/command-batch.ts` |
| XiaoDuoYa/codex-with-chatgpt | 70 文件 / TypeScript | 不驱动页面；网页 ChatGPT 通过 MCP 连接器调进来 | — | `express` 本地服务 + `cloudflared` 隧道（`src/tunnel/`：临时与命名两种）；只读 MCP（`src/mcp`）；认证在 `src/auth` | 规模最小 |

**共性（事实）**
- 四个项目里有三个直接驱动网页界面，靠的是浏览器自动化或页面扩展。
- 它们都通过 MCP 把本机能力暴露给网页端，或把网页端接回本机。
- 读额度这件事，codex-chatgpt-web 是从页面上解析出来的，不走接口。

---

## 2. 会话转移与检索

| 项目 | 规模 / 语言 | 做什么 | 覆盖的来源 | 其他 |
|---|---|---|---|---|
| CASR（cross_agent_session_resumer） | 150 文件 / Rust | 读出某一家的会话，转成统一的中间表示，再写成目标家原生的会话文件；**写完回读校验**，然后打印续接命令 | `src/providers/`：aider、amp、antigravity、chatgpt、claude_code、clawdbot、cline、codex、cursor、factory、gemini、grok、kiro、openclaw、opencode、pi_agent、vibe，共 17 家 | 安装方式为 `curl … \| bash` |
| cass（coding_agent_session_search） | 3,901 文件 / Rust | 本机多家会话的统一索引与检索 TUI；提供给 agent 使用的 `--robot` 模式 | README 列出 25 家以上 | **模糊测试语料很大**，其中 `fuzz_redact_secrets` 有 118 份，说明做了密钥脱敏的模糊测试；Windows 有 PowerShell 安装脚本 |

---

## 3. 决策与需求制品

**asdecided/core（原 rac-core），1,208 文件，Rust**
- README 原意：把需求、决策、设计、路线图、提示词都存成仓库里的**有类型的 Markdown**；原生 Rust 引擎校验这些知识，**确定性地**检索相关决策，并通过 MCP **只读**提供给 agent。"不需要 embedding、模型调用、托管索引或 Python 运行时"；"同一仓库状态给出同一答案"。
- `rust/decided-mcp/src/` 下有 `provenance.rs`、`audit.rs`、`graph.rs`、`sidecar.rs`、`tools.rs`。
- 这一组对应方向候选 §4 的"Owner 决定登记册"，以及第一批 §0.1 提到的"授权来源不能丢"。

---

## 4. 过程与复核

**boshu2/agentops，3,187 文件，Go + Shell + Python**
- CLI `ao` 加 38 个技能。
- 流程：一次改动走 Plan → Implement → Validate（RPI），大的工作由多个 RPI 组成目标，用 Beads（带依赖的议题跟踪）管理。
- **每次改动都由一个没写过这段代码的新会话来判定。**
- 同一套 `SKILL.md` 可用于 Claude Code、Codex、Cursor、OpenCode、Gemini CLI、Pi 等，也可用于 OpenClaw、Grok Bot。
- 相关模块：`cli/internal/goals`、`cli/internal/evalsubstrate`、`cli/internal/doctor`。

---

## 5. agent 之间的通信

**aannoo/hcom，220 文件，Rust**
- 单个二进制，**没有后台服务**。在 agent 命令前加上 `hcom` 启动即可，让不同终端里的 agent 互相发消息、观察对方、互相拉起。
- 往各家 CLI 装 hook 来实现：`src/hooks/` 下有 claude、codex、codex_file_edits、copilot、cursor、gemini、opencode、pi、antigravity。
- 支持 Claude Code、Codex、OpenCode、Kilo Code、Pi、Oh My Pi、Antigravity、Cursor、Kimi、Copilot，任意组合。

---

## 6. 与 gogoke 在 Windows / 智能应用控制下相关的事实

以下只描述形态，没有做实测。

| 形态 | 项目 |
|---|---|
| Rust 编译出的原生可执行文件，通过 `curl \| bash` 或 PowerShell 脚本安装 | CASR、cass、hcom、AsDecided |
| Go 编译出的可执行文件 | AgentOps（`ao`） |
| Electron 应用，带原生依赖 | codex-chatgpt-web（其 README 写明未做平台签名）、Chat On Steroids（`node-pty`、`sharp`、`tree-sitter`） |
| 纯 Node 脚本或 npm 包，可以用已签名的 node 运行 | Oracle、codex-with-chatgpt |

---

## 7. 本组未覆盖

- 没有逐文件审计凭证处理。例如 Oracle 的 cookie 读取、codex-chatgpt-web 的会话存储。
- 没有核对各项目遥测的具体内容。
- 没有做在本机运行时的实测。
