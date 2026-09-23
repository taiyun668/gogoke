# 全部已看源码的提取矩阵

- 更新日期：2026-08-20 增补 Hermes Studio 与 DeepSeek Harness 参考车道。`PRODUCT-PATTERN` / `SEMANTIC-PORT` 不是生产准入。

## 仓库级结论

| 仓库 | 可复用内核/语义 | 产品壳或专属部分 | 主要等级 |
|---|---|---|---|
| xAI Grok Build | host-agnostic lifecycle、ACP typed channel、ProcessScope、ACP test client | Grok Shell/Pager、认证、workspace 专属状态 | `SOURCE-CANDIDATE` + Grok Adapter |
| OpenAI Codex | PTY/process group、Windows Job、process hardening、官方 schema/tests | app-server 类型仅供 Codex Adapter；完整 sandbox/exec-server 非 MVP | `SOURCE-CANDIDATE` + Adapter-only |
| Grok App | FSM、turn fence、stall/watchdog、host-side ACP、诊断工具 | 整个非官方 App、RPC/IPC/store/native Session | `SOURCE-CANDIDATE` / `REJECT` |
| Gas City | Session≠Work、UNKNOWN、dispatch/retry/quarantine、轮换治理 | tmux/WSL、固定运行壳、prompt policy | `SEMANTIC-PORT` |
| Gas Town | role override、handoff/reclaim、party UI | 固定 Town 角色、tmux heuristics | `SEMANTIC-PORT` / `PRODUCT-PATTERN` |
| Beads | task/dependency、claim/lease/CAS、indeterminate commit、provenance | Dolt/JSONL 具体选型 | `SEMANTIC-PORT` |
| agtx | project-scoped MCP、plugin workflow、allowed actions、条件 claim | tmux 主路径、无 lease/fencing、自动前移无 Gate | `SEMANTIC-PORT` / `PRODUCT-PATTERN` |
| Squad | 事务化 task+message、条件 ACK、role projection、client protocol fields | Prompt receive loop、弱 lease、fail-open token、无中央 project scope | `SEMANTIC-PORT` |
| Omnigent | capability vocabulary、heartbeat/steer/approval/watchdog、prompt precedence | 整体 Meta-Harness、较大 Python scaffold | `SEMANTIC-PORT` |
| Claudexor | 最小 Adapter、ContextPack、terminal receipt、failure taxonomy | 具体宿主/存储选择 | `SOURCE-CANDIDATE` 或 `SEMANTIC-PORT` |
| Clay | YOKE event flatten、不同 transport、instruction exclusion、resume/cancel | browser workspace、Ralph loop、超大 Adapter、memory/chat shell | `SEMANTIC-PORT` / `PRODUCT-PATTERN` |
| Open Harness | manifest payload、path guard、provider projection、幂等 scaffold | Docker/tmux/单 sandbox、Prompt 自动化、覆盖式 update | `SOURCE-CANDIDATE` / `SEMANTIC-PORT` |
| Mission Control | 指挥中心 IA、workspace/task/approval/cost/audit UI | thin adapters、EventEmitter bus、启发式状态 | `PRODUCT-PATTERN` |
| AgentPulse | hook normalizer、event authority、trusted-root prelaunch、host/supervisor UI | 非原子 launch claim、silent-drop hook ACK | `SEMANTIC-PORT` / `PRODUCT-PATTERN` |
| AgentDeck | Adapter event sources、hook-first fallback、协议生成、多表面 session UI | 无锁 registry、诊断 journal、硬件/Apple shell | `SEMANTIC-PORT` / `PRODUCT-PATTERN` |
| CliDeck | project-scoped addressing、lineage boundary、hook coexistence、session deck | PTY ask、idle/transcript completion、内存 authority | `PRODUCT-PATTERN` / `REJECT` |
| EKKOLearnAI/hermes-studio | 审批/工作流/群聊信息架构；coding-agent proxy/run/event 映射思想 | BSL 商业复制、desktop/server 直连、把 Studio 当 Control Plane | `PRODUCT-PATTERN`；传输思想 `SEMANTIC-PORT`；无授权源码/`runtime` `REJECT` |
| deepseek-ai/deepseek-harness | Cordis 可逆插件思想、append-only trajectory、resume/fork/replay、subagent providers、stdio JSON-RPC、测试缝 | 本地 job/goal/schedule/worker 当 Control Plane；DSH seq 当 GOGO PARTY seq；Windows ACL 当完整隔离 | Adapter 目标 `SEMANTIC-PORT`；Control Plane 与权威替换 `REJECT` |

## GOGO 目标模块与优先来源

| GOGO 模块 | 第一来源 | 第二来源 | 不采用 |
|---|---|---|---|
| `harness-runtime-core` | Grok Build lifecycle + ProcessScope | Codex PTY/Windows Job + hardening | 任一完整 App/daemon shell |
| `adapter-sdk` | Claudexor contract/receipt | Omnigent capability vocabulary、AgentDeck source types | 把 orchestration 写进 Adapter |
| `codex-adapter` | Codex official app-server protocol/tests | Clay/AgentDeck event mapping经验 | PTY parser 作为正常主路径 |
| `grok-adapter` | Grok Build ACP/test support | Grok App host-side ACP/turn fence | 连接或依赖 Grok App |
| `task-ledger` | Beads semantics | Squad/agtx 条件 SQL 反例与测试 | Git 锁、内存 Map、select-then-update claim |
| `workflow-engine` | Gas City failure/retry | agtx data-driven phases、Gas Town handoff | artifact/prompt 自动取得 authority；DSH goal/plan/schedule/worker；把 Hermes Studio 进程内 timer 与 trigger identity 上升为 GOGO PARTY lease/fencing authority |
| `context-projector` | Claudexor ContextPack + Open Harness manifest/guard | Omnigent precedence、Clay exclusion、Squad provider paths | 无 manifest 的文件拼接；群聊 transcript 推断当 LoadProof |
| `session-governor` | Gas City | Grok App stall/turn fence、AgentDeck hook-first recovery | token 百分比或 idle 单因素轮换；Hermes Studio 双 Session store 当分权 |
| `event-store` | Claudexor terminal fence + GOGO 自有 sequence/transaction | Beads provenance、AgentPulse source vocabulary | EventEmitter、截断 journal、mtime；DSH SessionEvent 序号 |
| `dashboard` | Mission Control IA | AgentPulse fleet、AgentDeck/CliDeck session wall、agtx board、Hermes Studio 审批/工作流 UX | UI 自己推断权威状态；审批成功覆盖底层失败 |
| `group-chat` | 无第一实现来源；Hermes Studio 仅为 `PRODUCT-PATTERN` | Mission Control / Gas Town party 可视化 | transcript 启发式 mention 路由；relay store 当账本 |
| `desktop-packaging` | 无；Hermes Studio Electron updater/runtime-manager/webui-server 为 `PRODUCT-PATTERN` | Grok App Windows 产品化经验（不接入 App） | 把 Electron 主进程当 Controller |
| `future-dsh-adapter` | deepseek-ai/deepseek-harness transport/session/subagent 语义 | Codex/Claude Code/ACP 官方协议仍走各自 Adapter | 把 DSH 当 Control Plane；subagent finish 当 Receipt/Gate |
| `windows-isolation` | Codex Job/hardening bake-off；GOGO 自有 CapabilityEvidence | DSH Windows ACL 仅 partial 能力证据 | 把 ACL README 写成 PREVENTIVE 完整沙箱 |

## 推荐的源码 bake-off

在写产品功能前，只做三个有限候选实验：

1. **Process runtime bake-off**
   - 候选 A：Grok Build `ProcessScope` + `xai-tty-utils`。
   - 候选 B：Codex `utils/pty` + Windows `JobObject`。
   - 验收：Windows 子孙进程、并发 close/spawn、PID 复用、幂等 cancel、grace→kill、宿主崩溃残留。

2. **Lifecycle/Adapter bake-off**
   - 公共 lifecycle 采用 Grok Build contributor registry 的数据输入/能力注入原则。
   - 外部 Adapter API 采用 Claudexor 的小型 contract/receipt/failure taxonomy。
   - 验收：同一个 mock workflow 同时跑 fake Codex 和 fake Grok，任何 native 类型不得越过 Adapter 边界。

3. **Context projection bake-off**
   - 采用 Open Harness 的 manifest/path guard、Claudexor ContextPack、Omnigent precedence、Squad provider path 表。
   - 验收：项目默认 + 角色 + 成员 + task overlay，强制片段缺失 fail-closed，Windows 无 symlink 路径，输出 included/omitted/conflict/hash，并能由 doctor 证明 Harness 实际加载。

这三个实验结束前，不建立完整 Dashboard，也不复制任何大体量上游 runtime。Hermes Studio 与 DeepSeek Harness 不进入上述 bake-off；它们只提供参考需求和 Adapter 目标边界。

## 硬性拒绝清单

1. 不接入 Grok App，也不让用户安装它才能使用 Grok。
2. 不把 tmux、Dolt、Docker、某一云、某个 Web UI 或单一 Harness 设为 Core 依赖。
3. 不使用 select-then-update claim、无 fencing lease 或无跨进程锁的 JSON 文件承担调度 authority。
4. 不用 terminal idle、spinner、最后几行、mtime、自然语言 PASS 或 judge 文本提升 Task/Fact。
5. 不让 Prompt 承担权限、claim、Gate、重试预算、自动接力或项目隔离。
6. 不把完整 transcript 写入共享仓库；仓库只保存任务、事实、成果、证据、决定和回执投影。
7. 不同时引入多套功能重叠的 PTY/process supervisor；必须用 bake-off 选一个基础并记录未选原因。
8. 无单独商业许可时，不复制 Hermes Studio BSL 1.1 源码，不接入其 desktop/server runtime。
9. 不把 DeepSeek Harness 当作 GOGO PARTY Control Plane；DSH 序号只进 `AdapterEvent.nativeSeq`，subagent finish 只进 `NativeTerminalEvidence`。
10. 不把 DSH 本地 job/goal/schedule/worker 或 Windows ACL 包替换耐久账本、lease/fencing、spool、`TerminalReceipt`/`ReceiptRecord`/Gate、Project 隔离、Context `LoadProof` 或 Transition Engine。

## 设计结论

没有一个仓库可以直接成为 GOGO PARTY。可复用价值集中在小型协议和确定性 primitive；大项目主要贡献失败场景、语义词汇和产品表达。Hermes Studio 与 DeepSeek Harness 只增加参考-only 车道和负向需求，不改变这一结论，也不构成生产准入。

因此下一步不是继续扩充仓库名单，也不是开始画 Dashboard 或接入 DSH/Studio runtime，而是提交一个“通用内核选择 ADR”：固定语言/runtime、选择 process supervisor、确定 Adapter SDK/receipt 和 Context Projector 的最小接口，然后用两个真实公众 CLI 做同一套 conformance。参考负向场景保持 `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`。
