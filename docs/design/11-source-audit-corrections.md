# 源码审计校正账本

- 状态：v0.2 正式设计候选
- 输入：`docs/research/source-audit/04-cross-project-findings.md`、`05`—`09` 扩展审计

## 1. 逐项闭环

| 审计发现 | 设计处置 | 规范位置 |
|---|---|---|
| Git 可作信号/事实投影，不能作 claim/lease 队列 | 引入持久 Coordination Plane；Repository 只接收 Proposal/物化事实 | 02 §2/5，03 §1/4/5，ADR-0005 |
| Agent 持久、Session 可替换 | 明确 Agent/Session 身份、并行边界和三种连续性模式 | 01 §2，06 §5，ADR-0002 |
| “SQLite + Git 双层”不足 | 拆为 Canonical、Coordination、Live Observation、Native Session 四平面 | 01 §4，02 §2，ADR-0005 |
| 单 Controller 需 HA/fencing | 单一逻辑 authority + leader epoch + Assignment fencing token | 02 §3，04 §3/5，ADR-0004 |
| Adapter 不能抹平能力 | 定义 support/evidence/enforcement/freshness 与 Conformance Suite | 05 §4/9，ADR-0006 |
| 创建 Agent 自动生成 Harness 文档 | Context Compiler + EffectivePromptManifest + Projector load verification | 05 §7，06 §1/2 |
| 换窗不能只看 Token | 使用 drift/boundary/stall/wait/age/crash/churn/pressure 综合状态 | 06 §4/5 |
| 自动接力必须加事务 | receipt -> Gate -> Git materialization -> claimNext -> fenced dispatch | 02 §5，04 §6 |
| Auditor 数量应是 Workflow fan-out | Assignment fan-out + 声明式 fan-in；MVP 先证明一个 Auditor | 04 §7，08 §2 |
| 隔离需落到所有层 | DB、event、cache、command、filesystem、credential、session、artifact 全部 Project-scoped | 01 §6，07 §1/2 |
| Windows 能力需分层 | Native Control Plane、Native headless Runner、WSL、Remote Linux profiles | 02 §9，07 §8，ADR-0007 |
| 原子重命名不等于并发账本；select-then-update 不能 claim | Task Ledger 必须使用带 expected state/owner 的单语句 CAS，并返回单调 fencing token | 04 §3/5，ADR-0004 |
| Hook、PTY parser、transcript 的证据强度不同 | AdapterEvent 必须携带 source/authority/confidence；结构化 hook/protocol 新鲜时压过 parser，降级必须可见 | 05 §4/9，06 §4 |
| Provider 文档路径不同但 Prompt 不能承担控制权 | Context Projector 只投影角色/项目/任务上下文；claim、Gate、接力仍由 Controller Command 执行 | 04 §6，05 §7，06 §1/2 |
| 多套 PTY/process runtime 均有可复用实现 | 实现前进行 Grok Build 与 Codex process runtime bake-off，最终只保留一套公共基础 | 02 §7/9，07 §8 |

## 2. 源码未证明项的设计补齐

| 原缺口 | 当前处置 |
|---|---|
| Controller split-brain | leader lease、monotonic epoch、Runner/DB 双侧 fencing；接管先 reconciliation |
| Git promotion 事务边界 | `PROMOTION_PENDING` + transactional outbox + deterministic manifest + commit ack；indeterminate 不盲重放 |
| Blind Audit 信息流证明 | 独立 Snapshot/result namespace/event filter/visibility manifest/Reveal Gate |
| 凭据与 Remote Runner | Host-local handles、outbound typed Runner Protocol、enrollment/revoke/spool/reconcile |
| resume 与 cold rebuild 一致性 | 三种模式分名，统一 Snapshot/Prompt digest 和 continuity handshake |
| Artifact 保留/删除 | 独立内容状态机、classification、HOLD、反向引用、两阶段删除与 tombstone |
| Adapter 版本漂移 | CapabilityEvidence expires；Harness/Adapter/OS 变化触发 reprobe/conformance |
| Windows/WSL/remote 表达 | execution profile 是显式产品字段，不以“Windows 支持”一个布尔值概括 |
| Grok 专属运行时已有成熟公开实现 | 官方 CLI 直连保持主路径；只允许选择性复用 Grok App 源码；禁止接入已安装 Grok App |
| 官方 Grok Build 含 host-agnostic lifecycle/ACP/ProcessScope | 将其与 Claudexor Adapter contract 分层评估；不得让 Grok native type 泄漏进 Controller |
| AgentPulse/AgentDeck/CliDeck 的面板能力强于权威协调 | 仅借 Fleet/Session/Attention UI 与事件来源表达；账本和完成判定使用 GOGO 自有实现 |
| xAI ProcessScope 与 Codex PTY/Job 各自覆盖不同竞态 | 只保留一个公共 Supervisor；用 close/spawn、PID reuse、Windows suspended spawn、输出 loss 和 host crash Gate 后选型（ADR-0009 §3/Required Gates） |
| Adapter 原生终态不等于 GOGO 回执，Gate 也不能倒造运行回执 | Adapter 产出 NativeTerminalEvidence；Runner 耐久组装 TerminalReceipt；Controller 提交 ReceiptRecord 后再执行 Gate（05 §6，ADR-0009 §4） |
| Task 可能 fan-out 多个并行审计 Assignment | claim/lease/fencing 绑定 Project + Assignment + Attempt，不以 Task 级单 lease 串行化合法 fan-out（01 §3.3/3.4，04 §5/7，ADR-0009 §2） |
| bounded channel 或 broadcast lag 可能静默丢输出 | 公共 Supervisor 必须记录 sequence、截断/丢失/落盘证据；terminal boundary 不依赖普通输出通道（ADR-0009 §3/Required Gates） |
| JSON wire 的大整数与摘要投影容易跨语言分叉 | counter 在 wire 使用十进制字符串；JCS digest 绑定 producer/scope/sequence/业务时间/payload，排除 delivery metadata（12 §2/3） |
| SQLite WAL 容易被误解为多 writer 或可放同步盘 | 本地可靠卷、逐连接 FK、FULL、短 BEGIN IMMEDIATE、项目复合 FK、commit outcome reconciliation（13 §1/9） |
| Fake Harness 的绿色测试可能被误报为真实兼容 | 独立标签 `FAKE_CONFORMANCE_ONLY`；真实 Codex/Grok 必须按相同 test ID 重新取证（14 §1/8） |

## 3. 仍然开放但不阻塞当前模型的问题

1. Codex + Grok CLI 已由产品所有者确定为首发 Spike 组合；它们的具体能力等级仍需当前版本证据，Spike 失败不得静默换 Harness。
   Grok 的证据必须来自公众可下载的官方 Grok Build CLI；本机私有 Provider 仅可作为非规范对照，不得成为产品路径。
   2026-08-11 公众版 Spike 已完成，主通道固定为 Codex app-server stdio 与 Grok ACP stdio；当前状态为 `ADMIT_FOR_ADAPTER_IMPLEMENTATION`，权限、取消、恢复、分叉和跨项目负测完成前不得标记 Conformance PASS。证据见 `docs/research/adapter-spike/`。
2. ADR-0009 已提出 Rust + SQLite + React/TypeScript 与 schema-first Adapter wire 作为实现主干；在产品所有者接受和 Required Gates 完成前仍是候选，不得当作已冻结依赖选型。
3. Local artifact store 的具体磁盘布局、加密实现和默认保留天数待实现设计，但生命周期已冻结为候选。
4. Controller leader lease 在本地单实例 MVP 中可以退化为进程/数据库 lease；多主部署不是 MVP，但 epoch/fencing 字段不能删除。
5. Remote push/PR/CI 是否为某 Project 的完成 Gate 由 Policy 配置，不设全局答案。

## 4. 评审结论

审计中指出的架构缺口均已在 v0.2 候选中获得明确语义位置；尚无上游实现证据的部分被标记为 GOGO PARTY 自有设计，而非伪称“行业已有答案”。在产品所有者接受 ADR 前，当前文档仍是实现前候选，不授权进入产品编码。
