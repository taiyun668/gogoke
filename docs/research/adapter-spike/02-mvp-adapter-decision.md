# 首发适配决策与设计影响

## 1. Transport 选择

| Harness | 主通道 | 降级通道 | PTY 定位 |
|---|---|---|---|
| Codex | app-server stdio | `exec --json --ephemeral` | 只用于人工 attach，不作自动完成证据 |
| Grok Build | ACP stdio | headless `streaming-json` | 只用于人工 attach，不作自动完成证据 |

主通道用于持久 Session、实时状态、审批、用量和取消；降级通道只用于明确声明为 one-shot 的 Assignment。一次性通道不得伪装成可 steering、可 resume 的持久 Session。

## 2. 新增的 Adapter 不变量

1. **Protocol pinning**：每个安装版本生成/固定 schema 或 capability digest；版本变化立即失效并 reprobe。
2. **Terminal event fence**：Codex 只认 `turn/completed`；Grok ACP 只认 prompt response 的 stop reason，headless 只认最终 `end`。进程 exit 0 不能单独让 Task 变绿。
3. **Native ID mapping**：保存 Project/Agent/Session/Attempt 与 native thread/session/turn/request ID 的一对一映射，不把它们暴露成跨项目可猜句柄。
4. **Privacy firewall**：认证、账号、限额、reasoning 和 transcript 先过滤，再进入 Observation；Canonical Fact Plane 只接收验证后的 ResultCapsule。
5. **Warning channel**：用户级 hook/plugin/MCP/config 告警归一化为 `adapter.warning`，不得混成任务失败，也不得静默忽略。
6. **Credential/config split**：CredentialHandle、HarnessProfile 和 Project Workspace 是三个对象，不能把整个用户 Home 当作 Agent 隔离边界。

## 3. Harness Profile 模式

首版支持两种显式模式：

- `SHARED_USER_PROFILE`：复用用户已登录的公众 CLI Profile。优点是零重复登录；缺点是会继承用户级配置。必须 preflight、显示继承源、过滤隐私事件，并以独立 Workspace/Session/Project scope 隔离。
- `DEDICATED_PROFILE`：每个 Project 或安全域使用独立 Harness Home，需要用户单独完成官方 CLI 登录。它提供更强配置和会话隔离，不允许 GOGO 复制 token 文件来“自动克隆”身份。

Grok 可用 `GROK_AUTH_PATH` 在不复制凭证的情况下进一步拆开认证和 Home；Codex 当前公众 OAuth Profile 与 `CODEX_HOME` 耦合更强，因此严格隔离时应选择 dedicated login，而不是读取或复制 `auth.json`。

## 4. Context Governor 映射

- Codex：消费 `thread/tokenUsage/updated`、`thread/compacted`、thread status、turn/item lifecycle 和 warning/error。
- Grok：消费模型 `totalContextTokens`、update `totalTokens`、compaction/update 信号、session lifecycle 和 prompt stop reason。

这些原生信号只构成 `context_pressure` 和 `runtime_health`。是否换窗仍由 revision drift、task boundary、stall、interaction state、instruction integrity 和 handoff readiness 联合决定，不能把 80% token 使用率直接写成自动换窗命令。

## 5. 首发判定

Codex 与公众版 Grok Build 都进入首发 Adapter 实现队列。实现顺序建议：

1. 公共 Runner stdio supervisor、schema/version pin、Privacy Firewall。
2. Codex app-server Adapter + exec fallback。
3. Grok ACP Adapter + streaming-json fallback。
4. 两端共用 Conformance Suite，优先权限拒绝、终态、取消、恢复和跨项目负测。

在上述测试完成前，UI 可以显示 `Spike verified`，不得显示 `Production ready` 或 `Conformance PASS`。

## 6. Grok App 复用路线

Grok Adapter 采用两层路线：

1. **规范主路径**：GOGO Runner 直接启动公众版 `grok agent stdio`。
2. **选择性源码复用**：从固定版本的 MIT-licensed `RongleCat/grok-app` 移植 ACP、Session FSM、watchdog、terminal-event fence、权限解析和诊断等 Grok 专属模块及测试，封装为 Adapter 私有 runtime。
Grok App 是非官方社区产品。GOGO PARTY 不得检测、启动或连接 Grok App，不得接入其 Mirror RPC、IPC、store 或原生 Session。完整审计见 `../grok-app-reuse-audit.md`，已接受决策见 ADR-0008。
