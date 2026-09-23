# 横向结论与设计校正矩阵

## 推荐的 GOGO PARTY 内核分层

```text
Project / Party / Role / Policy
              |
        Workflow Engine
              |
 Task Ledger + Claim/Lease/CAS + Provenance
              |
       Runner Command Protocol
              |
  Harness Adapter Kernel + Conformance Bench
              |
 Claude / Codex / Pi / Z Code / OpenCode / Hermes / ACP / ...

Repository Fact Protocol <-> Task/Result/Fact/Decision projections
Runtime Store            <-> heartbeat/process/interaction/stream cursors
Native Harness Store     <-> transcripts/native session ids/vendor caches
```

仓库是可审阅协作账本，不是高频调度队列；Runtime Store 是可重建观察层，不提升项目事实；Harness 自己继续拥有完整对话。

## 设计草案校正矩阵

| 既有主张 | 审计结论 | 校正 |
|---|---|---|
| 仓库保存事实和成果，不保存完整对话 | **保留** | 多个上游都把 Transcript/native session 留在 Harness；仓库协议应只保存 Result/Fact/Decision/Revision 引用。 |
| Git 仓库可以直接作为消息总线 | **修改** | 可作为信号来源和持久投影，但 claim、lease、dedupe、offset、retry 必须在结构化协调层完成。 |
| Agent 是持久身份，Session 可替换 | **强支持** | Gas City 与 Omnigent 均有直接实现证据。 |
| SQLite + Git 双层状态 | **保留但改名** | 改为 Canonical Fact Plane、Coordination Plane、Live Observation Plane、Native Session Plane；不要把 SQLite 误称全部 runtime truth。 |
| Controller 是唯一权威提升者 | **方向支持，机制待定** | 上游支持 guarded/CAS promotion，但“唯一 Controller”是我们的治理选择；必须设计 HA/lease/fencing，避免单点与双主。 |
| Adapter 统一所有 Harness | **保留并加强** | 统一的是最小生命周期与事件语义，不是抹平能力。Capability 必须带 support level、evidence、freshness、enforcement strength。 |
| 创建 Agent 自动生成 Harness 项目文档 | **支持** | 使用单一 Context Compiler，定义 prompt precedence、生成清单、hash、敏感内容过滤、mandatory input fail-closed。 |
| 状态到一定程度自动换窗口 | **强支持** | 采用 task boundary、context mismatch、progress stall、interaction wait、age、rate limit、crash/churn 和 token pressure 的组合决策。 |
| 自动接力由仓库信号触发 | **保留但加事务** | Signal → dedupe → validate → atomic claim → dispatch → structured receipt → promote；人工按钮调用同一 Command。 |
| 一个或十个 Auditor 动态组织 | **支持** | Workflow graph/fan-out 是正确抽象；人数不应硬编码为角色字段。 |
| Project 与 Agent 隔离 | **强支持但需落到每层** | DB row、event、runner command、filesystem root、credential scope、session native ID、cache key、SSE subscription 均必须含 project/workspace scope。 |
| Windows 是首等本地平台 | **需明确能力矩阵** | Control Plane 可原生；tmux/native PTY 可能只能 WSL/remote。Windows Job Object 只提供进程树和资源约束，不等于文件系统/网络隔离。 |

## Adapter Capability 最小字段

MVP 不应只记录 `supported: true/false`，至少需要：

- integration mode：structured server / ACP / headless JSON / PTY；
- lifecycle：discover、start、resume、fork、interrupt、stop、attach；
- event quality：structured、native-observed、heuristic、unknown；
- interaction：approval、question、steering、live queue；
- context：usage、compaction、resume identity、fork history；
- output：structured result、diff、test、artifact、usage/cost；
- policy enforcement：preventive、approval-capable、post-hoc、none；
- isolation：cwd、filesystem, process tree、network、credential scope；
- evidence：probe/test/source、verified version、verified time；
- failure behavior：fail-closed、fail-open、unsupported、unknown。

## MVP 优先级校正

第一条垂直切片应是：

1. 单 Project，两个真实 CLI Harness；
2. Task ledger + dependency + atomic claim；
3. 一个 Implementer → 一个 Auditor 的自动接力；
4. Repository Result Capsule 与 revision evidence；
5. structured state/attention，不用终端文本宣称完成；
6. Session rotation recommendation，先人工确认，再开放自动轮换；
7. 同一协议下的第二 Project 隔离测试。

首批 Adapter 应优先选择能提供结构化事件和原生 Session ID 的 Harness。PTY-only 适配器可以同时开发，但只能以较低置信度进入面板，不得阻塞核心契约演进。

## 尚未被源码证明、必须在正式设计中补齐

- Controller 多实例的 leader lease、fencing token 和 split-brain 恢复；
- Git remote 冲突下 Canonical Fact promotion 的确定事务边界；
- Project 内 blind audit 的信息流证明，而不只是目录隔离；
- 凭据引用、轮换和 Runner 远端代理协议；
- Session rotation 时 native resume 与 cold rebuild 的一致性标准；
- Artifact 存储、保留、删除和敏感数据生命周期；
- 适配器版本漂移后的 probe 失效与自动降级；
- Windows 原生、WSL、remote runner 三种执行面的产品表达。

在这些问题完成设计与测试前，已有 ADR 应视为“方向已提出、机制未正式接受”。
