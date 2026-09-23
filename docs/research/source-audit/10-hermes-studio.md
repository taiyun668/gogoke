# EKKOLearnAI/hermes-studio 参考源码审计

- 审计日期：2026-08-20
- 仓库：`EKKOLearnAI/hermes-studio`
- 上游：[GitHub 仓库](https://github.com/EKKOLearnAI/hermes-studio)；[固定 commit](https://github.com/EKKOLearnAI/hermes-studio/commit/d6bed4cc550b8e887e69389d00840ac97407fdd1)
- 固定 commit：`d6bed4cc550b8e887e69389d00840ac97407fdd1`（全文证据边界）
- 许可证：Business Source License 1.1
- 结论性质：GOGO PARTY 参考-only 设计输入。`PRODUCT-PATTERN` 与 `SEMANTIC-PORT` 不是生产准入，也不是复制授权。

本文件不指控上游当前失败。下列六类失败模式是 GOGO PARTY 必须自行守住的 `REFERENCE_REQUIREMENT`，状态一律 `NOT_IMPLEMENTED`。本轮未复制源码、未加入依赖、未接入 Hermes Studio runtime、未执行上游测试，也未改动可执行 Fake 套件。

## 1. 许可与商业边界

控制器核验的 `LICENSE` 结论：

| 项 | 固定事实 |
|---|---|
| 许可证 | BSL 1.1 |
| Additional Use Grant | 仅非商业 |
| 商业使用 | 需要单独许可 |
| Change Date | 2029-05-10 |
| Change License | Apache-2.0 |

因此：

- 在无单独商业授权前，把 Hermes Studio 源码复制进 GOGO PARTY、当作运行时依赖或直接集成其 desktop/server，一律 `REJECT`。
- 非商业阅读、分类和产品/语义参考可以继续；这不改变复制禁令。
- Change Date 之前不得把该仓库当作 Apache-2.0 资产。
- 本判定不是对上游产品质量的评价，只约束 GOGO PARTY 的准入。

## 2. 证据边界与等级

本轮只使用控制器核验锚点。未在本工作区落地上游副本，也未运行任何 shell、安装、构建或测试命令。

| 锚点 | 等级 | 用途 |
|---|---|---|
| `README.md` | `DOC` | 产品定位、桌面/Web/coding-agent 表面 |
| `LICENSE` | `SOURCE` | BSL 1.1、非商业 Additional Use Grant、Change Date |
| `docs/workflow.md` | `DOC` | 定位工作流 UX，不证明实现 |
| `docs/openapi.json` | `DOC` | 定位 HTTP/审批/工作流 API 表面 |
| `packages/server/src/routes/coding-agents.ts` | `SOURCE` | coding-agent HTTP 入口 |
| `packages/server/src/services/coding-agents/runtime/run-manager.ts` | `SOURCE` | run 生命周期 |
| `packages/server/src/services/coding-agents/runtime/event-mapper.ts` | `SOURCE` | 原生事件映射 |
| `packages/server/src/services/coding-agents/codex/proxy.ts` | `SOURCE` | Codex 传输代理 |
| `packages/server/src/services/coding-agents/claude-code/proxy.ts` | `SOURCE` | Claude Code 传输代理 |
| `packages/server/src/services/workflow-manager.ts` | `SOURCE` | 工作流编排表面 |
| `packages/server/src/services/workflow-socket.ts` | `SOURCE` | 工作流实时推送 |
| `packages/server/src/services/workflow-schedule-service.ts` | `SOURCE` | 调度器所有权表面 |
| `packages/server/src/db/hermes/sessions-db.ts` | `SOURCE` | Session 持久化之一 |
| `packages/server/src/db/hermes/session-store.ts` | `SOURCE` | Session 持久化之二 |
| `packages/server/src/db/hermes/workflow-store.ts` | `SOURCE` | 工作流定义存储 |
| `packages/server/src/db/hermes/workflow-run-store.ts` | `SOURCE` | 工作流运行存储 |
| `packages/server/src/db/hermes/workflow-schedule-store.ts` | `SOURCE` | 调度持久化 |
| `packages/server/src/services/hermes/group-chat/runtime.ts` | `SOURCE` | 群聊运行时 |
| `packages/server/src/services/hermes/group-chat/agent-relay.ts` | `SOURCE` | Agent 转发 |
| `packages/server/src/services/hermes/group-chat/agent-relay-store.ts` | `SOURCE` | 转发状态 |
| `packages/server/src/services/hermes/group-chat/mention-routing.ts` | `SOURCE` | mention 路由 |
| `packages/server/src/services/hermes/group-chat/context-projection.ts` | `SOURCE` | 群聊上下文投影 |
| `packages/desktop/src/main/updater.ts` | `SOURCE` | Electron 更新 |
| `packages/desktop/src/main/runtime-manager.ts` | `SOURCE` | 桌面 runtime 托管 |
| `packages/desktop/src/main/webui-server.ts` | `SOURCE` | 桌面 WebUI 托管 |
| `tests/client`、`tests/server`、`tests/desktop`、`tests/e2e` | `TEST-SOURCE` / `NOT-RUN` | 只读测试 seam，本轮未执行 |

`DOC` 锚点只用于定位。`TEST-SOURCE` 不写成“测试已通过”。

名称注意：GOGO PARTY 既有文档中的 “Hermes” 指可替换高层助理或未来 Harness 名；本文件只讨论 `EKKOLearnAI/hermes-studio`，下文写 **Hermes Studio**。

## 3. 分层判定

| 层 | 观察 | 等级 |
|---|---|---|
| Dashboard / 审批与任务板 UX | 指挥面板、审批成功态、工作流可视化 | `PRODUCT-PATTERN` |
| Workflow UX | 阶段、运行、调度的交互与信息架构 | `PRODUCT-PATTERN` |
| Group Chat | 多 Agent 群聊、mention、relay、上下文投影的产品表达 | `PRODUCT-PATTERN` |
| Electron 打包与更新 | updater、runtime-manager、webui-server | `PRODUCT-PATTERN` |
| coding-agent 传输与 run 思想 | Codex/Claude Code proxy、run-manager、event-mapper | `SEMANTIC-PORT` only |
| 源码复制、desktop/server 直连、把 Studio 当 Control Plane | BSL 商业边界 + 权威层冲突 | `REJECT`（无单独授权） |

`PRODUCT-PATTERN` 只允许借信息架构后由 GOGO PARTY 重写。`SEMANTIC-PORT` 只允许移植不变量和失败语义，不照搬实现。两者都不是 `SOURCE-CANDIDATE`，更不是生产准入。

## 4. 产品壳：Dashboard、Workflow UX、Group Chat、Electron

Hermes Studio 把本地 Studio、coding-agent 会话、工作流和群聊放在同一产品里。对 GOGO PARTY 有用的是“一个面板同时看到任务、审批、会话和多人对话”，而不是它的进程、SQLite 表或 Electron 主进程。

应借的产品表达：

- 审批、运行、失败必须能在同一注意力面展开；卡片颜色不能代替 Gate。
- 工作流运行与 Session 是不同对象；完成一条任务不等于原生会话仍可解析。
- 群聊需要显式收件人，而不是从 transcript 猜 `@` 目标。
- 桌面包更新、本地 runtime 托管和 WebUI 托管是打包问题，不能变成 Controller。

不应进入 GOGO PARTY 权威层：

- `workflow-socket` 或任何 WebSocket/SSE 作为状态推进总线。
- Electron `runtime-manager` / `webui-server` 作为 Coordination Plane。
- 桌面 updater 作为能力或事实来源。
- OpenAPI 文档中的成功字段直接投影为 `GateEvaluation=PASS`。

## 5. coding-agent 传输：只移植思想

`coding-agents` 路由、`codex/proxy`、`claude-code/proxy`、`run-manager` 和 `event-mapper` 证明：每个 Harness 需要独立传输代理，原生事件必须映射后再展示。这与 GOGO PARTY Adapter 契约同方向。

可移植思想（`SEMANTIC-PORT`）：

- Codex 与 Claude Code 分 proxy，不把两套原生协议写进同一 Controller。
- run-manager 管一次 run 的启动/观察/结束，不签发项目事实。
- event-mapper 把原生事件变成 Studio 事件；GOGO PARTY 对应物是 Adapter Observation，且必须带 `source_strength`。
- 原生终态最多成为 `NativeTerminalEvidence` 的输入，不能直接成为 `TerminalReceipt`、`ReceiptRecord` 或 Gate。

不可移植：

- 复制 proxy/run-manager/event-mapper 源码。
- 把 Hermes Studio 的 run 身份当成 GOGO PARTY `Attempt`。
- 把 Studio 事件序号当成 Coordination Plane sequence。
- 在无商业授权时把 Studio server 当 Adapter 宿主。

## 6. 双 Session 存储与工作流存储是参考缝，不是缺陷指控

控制器核验锚点同时包含读取 Hermes `state.db` 的 `sessions-db.ts` 与 Web UI 自建存储 `session-store.ts`，以及 `workflow-store` / `workflow-run-store` / `workflow-schedule-store`。这给 GOGO PARTY 的教训是：Session、WorkflowRun 和 Schedule 若分多套存储，必须有唯一权威平面和 reconciliation。本轮**不**断言上游此刻已经分叉或丢所有权。

GOGO PARTY 对应规则：

- Session 身份只在 Coordination Plane 权威。
- Native Session Plane 只保存 Harness 引用与 transcript 指针。
- WorkflowRun/Task/Assignment 必须能解析 origin Session；解析失败则 Attempt 为 `OUTCOME_UNKNOWN`，不得标完成。
- 调度所有权必须是带 fencing 的耐久 lease，不能只活在 `workflow-schedule-service` 的进程内存里。

## 7. 群聊路由与上下文投影

`mention-routing`、`agent-relay`、`agent-relay-store` 与 `context-projection` 说明群聊有独立运行时：转发、提及和上下文投影可以和 coding-agent run 分开。产品上值得看；权威上必须 fail-closed。

GOGO PARTY 约束：

- 收件人必须是结构化 Agent/Session 引用，不能从 transcript 推断。
- 群聊上下文投影不能代替 `ContextSnapshot`、`EffectivePromptManifest` 或 `LoadProof`。
- relay 日志不是 Task Ledger，也不是 ReceiptRecord。
- 错投必须成为可审计 Attention，而不是静默成功。

## 8. 六条 GOGO PARTY 参考需求

下列每条都是 `REFERENCE_REQUIREMENT` 且 `NOT_IMPLEMENTED`。它们不是“Hermes Studio 当前失败”的主张，而是 Studio 产品缝给 GOGO PARTY 的必测负向场景。完整矩阵见 [12-reference-negative-test-matrix.md](12-reference-negative-test-matrix.md)。

### HS-01 底层失败而审批 UI 显示成功

`REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`

Approval/Dashboard 成功态不得覆盖 run-manager 失败、mapper 未知或缺失 `ReceiptRecord`。GOGO PARTY 的 UI 只能投影 `ReceiptRecord` 提交之后的 `GateEvaluation`。没有终态回执时状态是 `OUTCOME_UNKNOWN` 或 `APPROVAL_REQUIRED`，不是成功。

### HS-02 审批超时后的迟到 ACK

`REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`

审批超时必须消耗该次 approval generation，并使迟到 ACK 按旧 fencing token 拒绝。超时后的 ACK 不得补写 Task/Assignment，不得把已关闭 Attempt 改成成功。重复投递同一 payload 返回原结果；不同 payload 冲突拒绝。

### HS-03 任务完成无法解析 origin Session

`REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`

Task/Workflow 完成路径必须解析 origin `session_id`、native session ref 和 `attempt_id`。任一缺失则不得写 `SUCCEEDED`，不得组装 `TerminalReceipt`，不得进入 Gate。工作流 store 与 Session store 对不上时记 `OUTCOME_UNKNOWN` 并进 Attention。

### HS-04 两套 Session 存储分叉

`REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`

GOGO PARTY 只允许一个权威 Session 身份。Live Observation 与 Native Session 投影可以落后，但不得各自写成权威。分叉时 Controller 先 reconciliation，再决定是否重派；UI 不得选“更新鲜的那份”当事实。

### HS-05 重启丢失调度器所有权

`REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`

调度所有权必须是 Coordination Plane 上带 `controller_epoch` 与 fencing 的 lease。进程重启后先续租或接管，再触发到期 workflow。内存中的 `workflow-schedule-service` 身份不能在无 lease 时继续派活；双主必须零写拒绝。

### HS-06 transcript 推断路由到错误收件人

`REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`

mention 与 relay 只接受结构化收件人。transcript、最后几行、自然语言 `@名字` 或启发式投影最多作为低强度 Observation。错投不得形成对错误 Agent 的 Assignment，也不得把错误上下文写入 `LoadProof`。

## 9. 测试源码

`tests/client`、`tests/server`、`tests/desktop`、`tests/e2e` 证明 Studio 把 UI、server、桌面和端到端分成可测缝。等级：`TEST-SOURCE` / `NOT-RUN`。存在测试文件不等于测试通过，也不构成 GOGO PARTY 对 Studio 行为的回归证明。

不得把这些测试复制进可执行 Fake 套件。对应负向场景只记录在参考矩阵中，状态保持 `NOT_IMPLEMENTED`。

## 10. 结论

Hermes Studio 是完整产品壳：面板、工作流 UX、群聊和 Electron 打包值得看，coding-agent proxy/run/event 映射值得作为 Adapter 思想对照。它不能成为 GOGO PARTY Control Plane，也不能在无商业授权时成为源码或 runtime 依赖。

对 GOGO PARTY 最有价值的不是“把 Studio 接进来”，而是把审批成功、迟到 ACK、origin Session、双存储、调度所有权和群聊路由写成自己的负向不变量，并由 Controller、ReceiptRecord 和 Gate 执行。
