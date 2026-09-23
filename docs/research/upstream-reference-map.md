# Upstream Reference Map

- 调研日期：2026-08-11
- 用途：记录设计借鉴方向，不表示采用或复制其实现

| GOGO PARTY 模块 | 主要参考 |
|---|---|
| Host-agnostic lifecycle and ACP primitives | xAI Grok Build |
| Cross-platform PTY, Windows Job and official Codex protocol | OpenAI Codex |
| Project/Agent/workflow organization | Gas City, Gas Town |
| Repository-backed task graph | Beads |
| Harness Adapter and capability tests | Omnigent |
| Typed run facts and artifact layout | Claudexor |
| Operations dashboard and approvals | builderz-labs Mission Control |
| One-click phase workflow | agtx |
| Cross-Harness instruction loading | Clay |
| Sandbox and instruction projection | Open Harness |
| Attention and multi-host UI | AgentPulse, AgentDeck |
| Windows-friendly session dashboard | CliDeck, Squad |
| Grok host-side session and context source patterns only | RongleCat/grok-app |

## Links

- https://github.com/xai-org/grok-build
- https://github.com/openai/codex
- https://github.com/gastownhall/gascity
- https://github.com/gastownhall/gastown
- https://github.com/gastownhall/beads
- https://github.com/omnigent-ai/omnigent
- https://github.com/razzant/claudexor
- https://github.com/builderz-labs/mission-control
- https://github.com/fynnfluegge/agtx
- https://github.com/chadbyte/clay
- https://github.com/mifunedev/openharness
- https://github.com/jstuart0/agentpulse
- https://github.com/puritysb/AgentDeck
- https://github.com/rustykuntz/clideck
- https://github.com/mco-org/squad
- https://github.com/RongleCat/grok-app

## GOGO PARTY 不照搬的部分

- 不采用 Gas Town 的固定角色命名和完整 Town 体系。
- 不要求 Dolt 或 tmux 成为 Core 依赖。
- 不以完整 Transcript 为共享上下文。
- 不让任何单一 Harness 成为 Controller。
- 不把 Token 百分比作为 Session 轮换的唯一标准。
- 不把自然语言完成声明当作权威 Event。
- Grok App 仅作 MIT 源码参考；不检测、启动、连接或兼容其 App、RPC、IPC、store 和原生 Session。
- 不把 AgentPulse 的非原子 launch claim、Squad 的无 fencing lease、AgentDeck 的无锁 registry 或 CliDeck 的 PTY ask 当作控制面权威实现。
