# 状态观测、面板与产品壳源码审计

## Mission Control：运营信息架构，不是运行时内核

继续保留的价值：Projects、Agents、Tasks、Activity、Cost、Audit、Security、Approvals 的整体信息架构，workspace scope 和 task dispatch UX，以及对 adapter capability 的诚实展示。

不能采用的部分：framework adapter 较薄；进程内 EventEmitter 不是持久消息总线；部分运行状态来自启发式推断；Windows 能力不完整。判定：`PRODUCT-PATTERN` 为主，task dispatch 失败语义可 `SEMANTIC-PORT`。

## AgentPulse：多主机摄取、Supervisor 与安全预启动

### 值得移植

- hook → normalizer → event processor → DB → WebSocket 的观测链清楚，Claude/Codex 原始事件被映射为统一 category/source/providerEventType。
- `event-authority.ts` 显式定义来源优先级、近时重复窗口和 prompt mirror pair，说明同一事件的多来源去重不能靠 UI 临时判断。
- managed session 与 observed hook 分开，Supervisor 有注册、heartbeat、stale/offline、host routing 和 provider launch metadata。
- prelaunch action 对 trusted roots、祖先 symlink escape、deepest existing realpath、clone collision、seed hash 和非覆盖写做了大量检查。
- UI 将可观察 Session、托管 Session、Host、Launch、Inbox 和 Templates 放在同一面板，符合 GOGO 的主视图需求。

### 关键缺陷

`claimNextLaunchRequest` 先 SELECT 第一条 `validated` 行，再执行仅含 `WHERE id = row.id` 的 UPDATE；没有把 `status = validated`、空 claimant 或 claim token 放入条件。两个竞争者可能都更新并返回同一 launch。`updateLaunchDispatchStatus` 也只检查 supervisor ID，没有在 UPDATE 中带 claim token/旧状态 CAS。

因此：

- event normalizer、authority vocabulary、prelaunch path safety 为 `SEMANTIC-PORT`，小型纯函数可进入 `SOURCE-CANDIDATE`；
- Supervisor/Host/Inbox UI 为 `PRODUCT-PATTERN`；
- launch claim 实现为 `REJECT`，必须由 GOGO 账本重新实现并做并发测试。

此外，“hook ingestion 总返回 200 且限流丢弃为 silent”适合低影响 telemetry，不适合任务回执；GOGO receipt endpoint 必须明确 ACK、duplicate、conflict、retryable failure。

## AgentDeck：多表面协议、Adapter 归一与 hook 优先

### 值得移植

- `AgentAdapter` 将 terminal、mode switching、diff review、options、suggestions、usage、model catalog 等能力显式化。
- AdapterEvent 分 hook/parser/metadata/activity/connection/timeline 来源，给“证据等级”提供了实际结构。
- gateway protocol 具有 request ID、event sequence、stateVersion、idempotency key、subscribe 和 approval resolve，且生成 Swift/Kotlin binding，适合借鉴多前端协议治理。
- Codex turn manager 明确把新鲜 hook 设为权威，PTY parser 只在 hook 失联后降级；漏失 stop 时，新 prompt 是权威边界并关闭陈旧 turn。
- 事件 journal、bounded diagnostics、PTY ring buffer、session registry、remote attach 和多设备状态展示为本地指挥中心提供产品素材。

### 限制与拒绝

- `AgentCapabilities` 主要是 UI 布尔值，缺少 enforcement、evidence source、tested version 和 freshness；不能直接成为 GOGO CapabilityEvidence。
- `sessions.json` 的临时文件重命名只防半写；read-filter-write 没有跨进程锁或事务，不能承担 registry/claim authority。
- event journal 是截断诊断日志，没有 commit fence、sequence 或 projection checkpoint，不能作为 durable event bus。
- PTY fallback 的 1.5s/15s/30s 等窗口是 Codex 适配经验，不是跨 Harness 统一真理。
- hardware plugin、Apple app、设备协议和品牌 shell 为 `REJECT`。

判定：协议生成、hook/parser authority 规则、turn recovery 为 `SEMANTIC-PORT`；面板和跨设备 attach 为 `PRODUCT-PATTERN`；小型协议类型可在依赖清理后进入 `SOURCE-CANDIDATE`。

## CliDeck：Windows 友好的 Session Deck，但控制证据偏弱

### 值得移植

- Session 明确归属 project，重名检查在项目 scope 内，跨项目 ask 必须使用显式 `@project/session`。
- lineage 只复用 transcript/menu parser，不抹掉真实 Harness identity；这个边界适合 Generic PTY Adapter。
- Codex hook 安装会保留第三方 hook，并可健康检查和卸载自身配置。
- Pi/OpenCode bridge 优先使用供应商 session ID/event，再把结果映射到统一 working/idle/preview。
- 可恢复 Session 列表、快速 attach、项目筛选和多 CLI preset 是良好的产品体验。

### 限制与拒绝

- Session 主要由内存 Map + `sessions.json` 管理，状态 authority 不足。
- `ask` 向目标 PTY 注入 bracketed paste 和 Enter，再等待 working→idle/转录文本；它拒绝 busy 而不排队，也没有 task ID、claim、receipt 或 fencing。
- Generic PTY 状态仍依赖 terminal/menu/idle heuristic；只能标为 degraded/unknown。
- transcript parser/normalizer 可以用于只读展示和检索，不得提升 Fact 或完成 Task。

判定：`PRODUCT-PATTERN` 为主；project-scoped address、lineage boundary 和 hook coexistence 为 `SEMANTIC-PORT`；PTY ask 自动派活为 `REJECT`。

## 面板设计的合并结论

GOGO 面板不应复制任一完整 UI，而应组合四种视图：

| 视图 | 主要借鉴 | GOGO 权威数据 |
|---|---|---|
| Project / Party / Task board | Mission Control、agtx、Gas Town | Task Ledger + Workflow projection |
| Fleet / Host / Session wall | AgentPulse、AgentDeck、CliDeck | Runner heartbeat + Adapter evidence |
| Attention Inbox | Mission Control、AgentPulse、AgentDeck | typed AttentionRequest + Gate decision |
| Native Session attach / timeline | AgentDeck、CliDeck、Clay | Native Session Plane；只读 transcript 不进入 Fact Plane |

任何状态卡片都必须能展开显示：`source`、`observed_at`、`confidence`、`native identity`、`capability digest` 和 `why unknown`。面板不得用颜色把启发式 idle 包装成确定完成。

