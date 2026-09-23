# CapabilityEvidence：Codex 0.147.0 与 Grok Build 1.0.0

证据等级：`OBSERVED` 表示公众发行二进制真实观察；`SOURCE` 表示公开源码/官方文档已证明但本轮未做动态负向测试；`UNKNOWN` 表示不可据此调度。

## 1. Codex

| 能力 | 判定 | 证据 | 限制 |
|---|---|---|---|
| 机器协议 | SUPPORTED | OBSERVED | `app-server` stdio JSONL；WebSocket 官方标为 experimental/unsupported，不进入 MVP |
| Session/Thread create | SUPPORTED | OBSERVED | `thread/start` 返回 ephemeral thread 和 idle 状态 |
| resume/fork/archive | SUPPORTED | SOURCE | `thread/resume`、`thread/fork`、`thread/archive`；仍需动态隔离测试 |
| 结构化流与终态 | SUPPORTED | OBSERVED | `turn/started`、`item/*`、`turn/completed`；真实请求终态 completed |
| 一次性执行 | SUPPORTED | OBSERVED | `exec --json --ephemeral` 返回 JSONL 终态与 usage |
| 中途 steering | SUPPORTED | SOURCE | `turn/steer`；review/manual compaction turn 不接受 |
| interrupt | SUPPORTED | SOURCE | `turn/interrupt`，最终应以 `turn/completed: interrupted` 确认 |
| Usage/context pressure | SUPPORTED | OBSERVED | `thread/tokenUsage/updated` 包含 total/last usage 和 model context window |
| compaction event | SUPPORTED | SOURCE | `thread/compacted` |
| 权限与审批 | DEGRADED | OBSERVED + SOURCE | read-only/never 策略被回显；尚未执行 command/file approval 的允许与拒绝负测 |
| 项目指令 | DEGRADED | SOURCE | 返回 `instructionSources`；本轮未验证生成的 AGENTS.md precedence/hash |
| 认证与配置隔离 | DEGRADED | OBSERVED | 隔离 `CODEX_HOME` 可启动但没有用户登录态；标准 Profile 会同时继承 hooks/plugins/MCP/config 告警 |
| Windows Native | SUPPORTED | OBSERVED | 公开签名 x64 binary 在 Windows 成功握手和真实请求 |

## 2. Grok Build

| 能力 | 判定 | 证据 | 限制 |
|---|---|---|---|
| 机器协议 | SUPPORTED | OBSERVED | `agent stdio` ACP v1；另有 `agent serve` WebSocket |
| Session create | SUPPORTED | OBSERVED | 认证后 `session/new` 成功；未认证明确返回 `Authentication required` |
| load/resume/close/list | SUPPORTED | SOURCE + HANDSHAKE | initialize 公布 load/list/resume/close；仍需动态重连测试 |
| 结构化流与终态 | SUPPORTED | OBSERVED | ACP `session/update` + prompt response；headless `streaming-json` 最终固定 `end` 事件 |
| 一次性执行 | SUPPORTED | OBSERVED | `-p --output-format streaming-json`，真实请求返回 stopReason/sessionId/requestId/usage/cost |
| 中途 steering | UNKNOWN | SOURCE | ACP 可排队 prompt 并有扩展输入机制，但未证明与 GOGO `steer` 语义等价 |
| cancel | DEGRADED | SOURCE | `session/cancel` 与 `cancelRewind` 已声明；尚未验证副作用停止和最终确认 |
| Usage/context pressure | SUPPORTED | OBSERVED | init 给出模型总窗口；update `_meta.totalTokens` 和终态 usage 可用于原生压力信号 |
| compaction | SUPPORTED | SOURCE | `/compact`、auto-compact 阈值和 compaction 信号存在；尚未触发动态试验 |
| 权限与审批 | DEGRADED | SOURCE | mode、allow/ask/deny、deny 优先和 sandbox 完整；本轮只执行无副作用请求 |
| 项目指令 | DEGRADED | SOURCE | 原生读取 AGENTS.md 等文件；本轮未验证生成文件的实际加载顺序 |
| 认证与配置隔离 | SUPPORTED | OBSERVED | `GROK_AUTH_PATH` 可复用公众登录态，同时使用隔离 `GROK_HOME` 和 Workspace |
| Windows Native | SUPPORTED | OBSERVED | 官方签名 x64 binary 在 Windows 成功 ACP 和 headless 真实请求 |

## 3. 尚未获得 PASS 的 Conformance 项

- 恶意/越界工具调用在两端是否真正 DENY，且拒绝事件能否可靠归一化。
- 执行中 cancel/interrupt 后是否仍有后台进程或文件副作用。
- 进程崩溃、Controller 重启、协议断线后的 resume/reconcile。
- 相同 Session ID、跨 Project/Agent 误用时是否 fail closed。
- AGENTS.md/项目规则的真实加载路径、precedence、漂移检测和 hash 握手。
- native fork 与 GOGO Snapshot/Revision 的一致性。
- Grok steering 的等价语义；未证明前 Scheduler 必须按 `UNKNOWN` 处理。

因此本 Spike 的结论是 `ADMIT_FOR_ADAPTER_IMPLEMENTATION`，不是 `CONFORMANCE_PASS`。
