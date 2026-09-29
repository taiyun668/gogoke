# 执行底座源码拆解（第二批 B，第一组）

- 日期：2026-09-25
- 性质：事实底稿，不评判、不给建议。只记"它实际怎么做"，出处精确到仓库内路径。
- 方法：通过 GitHub API 读取默认分支的完整文件树与关键文件，只读，没有下载或运行任何代码。
- 对象（按星数）：Orca、Paseo、Superset、OpenMausBot、Happier、awslabs/cli-agent-orchestrator（CAO）、microsoft/conductor。
- 星数、许可证、日期见 [第一批底稿 §3.1](2026-09-25-next-stage-research-base.md)。

---

## 1. 一览

| 项目 | 规模（文件数 / 主语言） | 怎样驱动各家 agent | 多账号 / 额度 | 许可证（读 LICENSE 原文） | Windows 相关文件 |
|---|---|---|---|---|---|
| Orca | 29,449 / TypeScript（Electron，另有移动端、Swift、HCL 云） | 以本地 PTY 为主（`src/main/providers/local-pty-*`）；为 Claude、Codex 各有专门模块（`src/main/claude`、`src/main/codex`）；往各 CLI 安装 hook 以获取状态（`src/main/agent-hooks`、`src/shared/agent-hook-listener/providers`）；带守护进程（`src/main/daemon`，含冷恢复与检查点） | **有**：`src/main/claude-accounts`、`src/main/codex-accounts`（按账号托管独立的 home 与凭证、登录会话、`codex-reset-credit-coordinator`/`-ledger`）；`src/main/rate-limits`（按账号取用量，既走 OAuth 接口，也解析 PTY 输出）、`src/main/usage`、`src/main/codex-usage` | MIT | 335 |
| Paseo | 4,876 / TypeScript（服务端守护进程、App、CLI） | **Claude 走 `@anthropic-ai/claude-agent-sdk`**（`packages/server/src/server/agent/providers/claude/agent.ts`）；**Codex 走 app-server JSON-RPC**（`codex-app-server-agent.ts`，含 `approvalPolicy`、`sandbox`、`approvalsReviewer: auto_review`）；Copilot、Cursor、Kimi、Kiro、Trae 等走 ACP（`*-acp-agent.ts`、`generic-acp-agent.ts`）；另有 OpenCode、Pi | **有**：`packages/server/src/services/quota-fetcher/providers/{claude,codex,copilot,cursor,grok,kimi,minimax,zai}.ts`，见 §3 | 自有代码 Apache-2.0，第三方组件各随原许可 | 5 |
| Superset | 10,167 / TypeScript（Bun） | `packages/pty-daemon`（PTY 守护进程，带进程树）；`packages/host-service/src/terminal-agents`（agent 绑定、持久化、子 agent 转录、守护进程丢失后的清扫） | 用量历史：`packages/host-service/src/trpc/router/usage` | **Elastic License 2.0**（源码可见，非 OSI 开源） | — |
| OpenMausBot | 2,733 / TypeScript（Electron，另有 Kotlin/Swift 移动端） | **Claude 走 CLI 的 stream-json 双向通信，用 `--resume <sessionId>` 续接**，另有权限代理（`server/drivers/claude.ts`）；**Codex 走 app-server JSON-RPC**（`server/drivers/codex.ts`，无人应答的权限请求超时后跳过）；Cursor、Droid、Gemini、Grok、Hermes、Kimi、Qwen 等走 ACP（`server/drivers/acp/*`）；有 `codex-device-auth.ts`、`claude-login-auth.ts` | 未见专门的额度模块 | Apache-2.0 | 12 |
| Happier | 21,359 / TypeScript（CLI + UI + 守护进程） | 每家一个后端（`apps/cli/src/backends/`：agy、auggie、claude、codex、copilot、cursor、devin、droid、gemini、grok、kilo、kimi、kiro、opencode、pi、qwen 等）；Claude 分本地与远程两种启动方式（`claudeLocalLauncher.ts`、`claudeRemoteLauncher.ts`），带上下文压缩事件；有 `forking`、`directSessions` | 未见专门的额度模块 | MIT | 96 |
| CAO（awslabs） | 1,634 / Python（另有少量 Rust、TypeScript） | **tmux**：每家 CLI 在 tmux 会话里运行，经 send-keys 输入、读屏幕快照解析（`providers/claude_code.py`）；Claude 默认带 `--dangerously-skip-permissions`，并自动处理"Yes, I accept"确认；支持 `resume` | — | Apache-2.0 | — |
| microsoft/conductor | 793 / Python | **SDK 与 API**：GitHub Copilot SDK、Anthropic、Claude Agent SDK（实验）、OpenAI、Hermes、Azure Container Apps 沙箱（`src/conductor/providers/*`）；**编排路由是 YAML + Jinja2，决策环节不用 LLM**；有 Script、Set、MCP、Terminate 步骤，人工闸门，最大迭代与超时，OpenTelemetry | Fleet Manager TUI 显示 token 与成本 | MIT | 50 |

---

## 2. 驱动方式归类（事实）

同一家 CLI，在不同底座里被用不同方式接入：

| 接入方式 | 谁在用 | 拿到的是什么 |
|---|---|---|
| **官方结构化协议**：Claude Agent SDK / stream-json、Codex app-server JSON-RPC、ACP | Paseo、OpenMausBot、Happier，Orca 部分使用 | 结构化事件：工具调用、权限请求、用量、会话 ID；权限请求可以程序化应答 |
| **PTY 终端 + hook** | Orca、Superset | 完整的交互终端，状态靠往 CLI 里安装 hook、解析终端输出 |
| **tmux + 读屏** | CAO | 靠屏幕快照和正则识别状态，权限通常直接绕过 |
| **SDK / API** | microsoft/conductor | 不经 CLI 的会话；计费走 API 或 Copilot 认证 |

- Paseo 的 Codex 接入内置了"auto_review"：符合条件的 `on-request` 批准会交给一个自动审查子 agent（`codex-app-server-agent.ts` 第 230–283 行）。
- OpenMausBot 的 Codex 接入对无人应答的权限请求会超时，并回话让 agent"跳过这个动作，尽量完成其余部分"（`server/drivers/codex.ts` 第 169 行）。

---

## 3. 额度读取：Paseo 实际调用的接口

`packages/server/src/services/quota-fetcher/providers/`：从各 CLI 本地的登录凭证读出 OAuth token，调各家的用量接口，得到窗口使用率与重置时间。

| 厂商 | 凭证来源（代码所读位置） | 用量接口 | 取到的字段 |
|---|---|---|---|
| Claude | macOS 钥匙串"Claude Code-credentials"，或本地凭证文件；OAuth beta 头 `oauth-2025-04-20` | `https://api.anthropic.com/api/oauth/usage` | `five_hour`、`seven_day`、`seven_day_opus`、`limits[]`（按模型或界面的周上限）、`utilization`、`resets_at`、`extra_usage` |
| Codex | `CODEX_HOME/auth.json` 或 `~/.config/codex/auth.json` 中的 `tokens.access_token` 与 `account_id` | `https://chatgpt.com/backend-api/wham/usage` | `rate_limit.primary_window`（会话）、`secondary_window`（周）、`code_review_rate_limit`、`used_percent`、`reset_at` |
| Grok | `~/.grok/auth.json` | `https://cli-chat-proxy.grok.com/v1/billing?format=credits`（注释说明：不带 `format=credits` 时，统一计费账号拿到的是清零的旧格式） | 积分用量 |
| Copilot | — | `https://api.github.com/copilot_internal/user` | — |
| Cursor | — | `https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage` | 当期用量 |

- Orca 另有两条路：一是按账号调 OAuth 用量接口（`rate-limits/claude-oauth-usage-request.ts`）；二是在 PTY 里运行 CLI 自己的用量命令并解析输出（`claude-pty-usage-parser.ts`、`claude-pty-reset-parser.ts`）。同时维护 Codex 的"重置额度"账本（`codex-accounts/codex-reset-credit-ledger.ts`）。
- 这些接口没有在任何厂商的公开文档里出现过（见第一批底稿 §2）：它们是 CLI 自己在用的内部接口。

---

## 4. 多账号：Orca 的做法（文件名层面的事实）

- 每个账号一个托管 home：`codex-managed-home-path.ts`、`codex-managed-home-lifecycle.ts`、`host-codex-managed-home-ownership.ts`；Claude 侧对应 `managed-auth-path.ts`、`claude-managed-auth-storage.ts`、`keychain.ts`。
- 登录流程由它托管：`claude-login-session.ts`、`codex-login-session.ts`、`codex-login-auth-url.ts`、`oauth-refresh.ts`。
- 旧版"共享凭证"迁移：`legacy-shared-auth-migration.ts`。这与第一批底稿 §1.4 记录的"两个程序共用同一账号，一次性 refresh token 会互相作废"是同一类问题。
- 账号与运行目标同步：`rate-limits/account-runtime-target-sync.ts`。
- Windows 下调用命令有专门处理：`claude-accounts/windows-command-invocation.ts`。

---

## 5. 与 gogoke 已有设计相关的对照点（只列对应关系，不作取舍）

| gogoke 已有设计或方向 | 底座中的对应实现 |
|---|---|
| 额度作为调度资源（方向候选 §4） | Paseo 的额度读取器；Orca 的按账号用量与重置账本 |
| 跨订阅多账号 | Orca 的按账号托管 home 与登录 |
| 权限闸 / 分阶段放权 | Codex app-server 的 `approvalPolicy`、`sandbox`、`approvalsReviewer`；ACP 的 `session/request_permission`；OpenMausBot 的权限代理与超时跳过 |
| 编排决策不交给 LLM | microsoft/conductor 用 YAML + Jinja2 做确定性路由 |
| 会话续接与分叉 | stream-json 的 `--resume`；Happier 的 `forking`；Grok Build 的 `--fork-session`（第一批 §2.4） |
| 本机守护进程与重启恢复 | Orca 的 `daemon`（冷恢复、检查点）；Superset 的 `pty-daemon` 与守护进程丢失清扫 |

---

## 6. 本组未覆盖

- 只读了文件树和关键入口文件，没有逐文件审计安全边界、凭证处理细节和遥测内容。
  - Orca 有 `install-telemetry.ts`，代码里出现 posthog 字样。
  - OpenMausBot 也出现 posthog 字样。
- 没有核实各底座在 Windows 智能应用控制下的实际表现。它们都是 Electron / Node 或 Python 应用，是否带原生子进程未逐一核对。
- 第二组待拆：额度通道（codex-chatgpt-web、Chat On Steroids、Oracle）、会话转移（CASR、cass）、记忆与复核零件。
