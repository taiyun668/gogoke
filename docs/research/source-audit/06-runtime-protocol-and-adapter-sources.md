# Runtime、协议与 Adapter 源码候选

本组回答两个问题：哪些代码可以成为多 Harness 公共运行时，哪些必须封装在单一 Adapter 内。

## xAI Grok Build：官方源码镜像

### 可进入通用内核候选池

| 源码边界 | 已观察实现 | 判定 |
|---|---|---|
| `xai-agent-lifecycle` | 模块注释明确为 host-agnostic；按 session lifecycle、turn lifecycle、turn input、command contributor 分离；安装时注入能力，不拥有宿主循环；registry 构建后不可变 | `SOURCE-CANDIDATE`，优先评估 `Send + Sync` 的 `send` 变体 |
| `xai-acp-lib` | typed client/agent channel、request/response oneshot、gateway、message、line/stdin reader；对 peer close 返回结构化 channel failure | `SOURCE-CANDIDATE`，作为 ACP protocol-driver 基础，不直接定义 GOGO 领域事件 |
| `xai-tty-utils::ProcessScope` | scope 位于 worker 之外；Weak registry 防 PID 复用误杀；close/spawn 竞态下即时杀死晚到子进程；`kill_all` 幂等；Unix process group 与 Windows Job 由下层封装 | `SOURCE-CANDIDATE`，目前最完整的公共 Runner 进程 scope 候选 |
| `xai-grok-test-support::acp_client` | 初始化、认证、new/load/prompt、超时、raw wire、子进程诊断等测试客户端 | `SOURCE-CANDIDATE` 仅限测试工具；自动批准逻辑不得成为产品默认 |

### 只能留在 Grok Adapter 或安全子系统

- Grok workspace checkpoint、recovery、foreign session discovery 只贡献 Grok Adapter 恢复语义，不能定义 GOGO Agent/Attempt 身份。
- permission rules、exec risk、preflight 和 auto mode 的源码资产丰富，但属于高风险安全边界；MVP 只能做独立审计后的局部移植，不能因为存在测试就直接采用。
- Grok Pager、Shell、品牌 UI、认证产品流程和内部 telemetry 都属于产品壳或 Grok 专属实现。

### 结论

官方 Grok Build 应成为 Grok 协议和公共 Rust runtime 的第一参考。非官方 Grok App 仍有宿主侧多 Session、Windows 产品化和 UI 编排价值，但不再默认拥有通用内核的首选权。

## OpenAI Codex

### 可进入通用内核候选池

| 源码边界 | 已观察实现 | 判定 |
|---|---|---|
| `codex-rs/utils/pty` | Unix process group、Linux parent-death signal、macOS group member fallback；Windows Job 用 suspended spawn 避免子进程在加入 Job 前逃逸，并持有 process handle 防 PID 复用 | `SOURCE-CANDIDATE`，与 `xai-tty-utils` 做 bake-off，不同时引入两套抽象 |
| `codex-rs/process-hardening` | 启动前禁 core dump/ptrace、清理 preload 类危险环境变量，覆盖 Unix 与 Windows seam | `SOURCE-CANDIDATE`，作为可选宿主 hardening，不与项目权限模型混用 |
| `exec-server` / `exec-server-protocol` | 远端执行、环境、能力发现、恢复、process tree 和网络策略拥有大量类型与测试 | `SEMANTIC-PORT` 起步；范围过大且与 Codex 产品架构耦合，不进入 MVP 直接源码池 |

### 只能留在 Codex Adapter

- `app-server-protocol` 的 JSON-RPC 类型、schema export、thread/turn/item/approval 映射是 Codex Adapter 的优质官方协议源。
- schema fixture、event mapping 和 protocol tests 应随 Adapter 使用，但 Codex 的 Thread/Turn 类型不能泄漏成 GOGO 的 Agent/Assignment/Attempt 类型。
- Windows sandbox 是大体量安全子系统，涉及 ACL、WFP、身份、helper materialization 和 elevated backend；在单独 threat model、打包和兼容性审计之前，不列为 MVP 直接复用项。

## Grok App：只复用源码，不接入产品壳

Grok App 仍贡献以下实现资产：

- `session_fsm`、`turn_complete`、`stream_stall`、`process_limits`、`store_lock`、`path_scope` 等低耦合 Rust primitive；
- host-side ACP reverse RPC、事件解码、多 Session 路由、resume、usage 和迟到事件处理；
- Windows 本地应用对 watchdog、诊断、原子写和支持包的产品化经验。

重新分类后：

| 部分 | 判定 |
|---|---|
| 低耦合 runtime primitive | `SOURCE-CANDIDATE`，但先与 Grok Build/Codex 同类实现比较 |
| Grok ACP host 与 golden fixtures | `SOURCE-CANDIDATE`，封装在 Grok Adapter |
| 多 Session/上下文/诊断算法 | `SEMANTIC-PORT` 或局部源码候选 |
| Mirror UI/Host 的网络结构 | `PRODUCT-PATTERN`；如复用源码也必须改成 GOGO 自有协议和身份模型 |
| Grok App 进程、RPC、IPC、store、原生 Session | `REJECT` |

GOGO 不探测、不启动、不连接、不兼容 Grok App。这个硬边界由 ADR-0008 约束。

## Claudexor

Claudexor 仍是最小严格 Adapter 语义的最佳参考：

- 小型 `Adapter` 接口分离 discover/doctor/run/review/cancel；
- `ContextPack` 明确上下文输入而非让 Adapter 自行抓取所有文件；
- append-only event log 和 terminal receipt fence 把“进程结束”与“任务回执已提交”分开；
- failure taxonomy 为 Controller 提供可恢复、需人工、未知等结构化结果。

若 GOGO 公共内核选 TypeScript，小型接口和 ContextPack 可进入 `SOURCE-CANDIDATE`；若选 Rust，则为 `SEMANTIC-PORT`，不能为了复制几十行接口而引入第二运行时。

## Omnigent

Omnigent 的最大价值不是复制 scaffold，而是能力词汇和长期运行失败场景：

- single-turn、heartbeat、interrupt、steer、approval、resume/fork、watchdog；
- prompt precedence 和 session metadata；
- 多 Harness 能力差异显式化。

其 Python scaffold/process manager 较大，且 Windows 能力存在降级，判定为 `SEMANTIC-PORT`。GOGO 应把这些能力变成带证据和 freshness 的 registry，而不是照搬整个 Meta-Harness。

## Adapter 与公共内核的目标分界

```text
Controller domain
  └─ Harness-neutral Adapter Contract
       ├─ Codex Adapter → official app-server protocol
       ├─ Grok Adapter  → official ACP + selected host logic
       ├─ Pi Adapter    → future protocol/hook implementation
       └─ Generic PTY   → explicitly degraded evidence

Common Runner Runtime
  ├─ lifecycle contributor registry
  ├─ process scope / PTY / Windows Job
  ├─ timeout, cancellation, bounded diagnostics
  └─ typed event envelope
```

公共内核不得识别 Grok thread、Codex item 或 Claude SDK message；这些只能由 Adapter 映射成 GOGO 的 AttemptEvent、AttentionRequest、UsageEvidence 和 TerminalReceipt。

