# 扩展源码审计：范围、固定版本与统一判尺

- 审计日期：2026-08-11；参考车道补充：2026-08-20
- 目的：把此前看过但只做过产品层判断的仓库，按与 Grok App 相同的源码复用方法重新审视；2026-08-20 增加 Hermes Studio 与 DeepSeek Harness 两条 reference-only 车道
- 结论边界：这是实现前的候选筛选，不代表已复制源码、已通过测试或已选择技术栈。`PRODUCT-PATTERN` / `SEMANTIC-PORT` 不是生产准入

## 统一判尺

每个仓库都拆成四层：

1. **产品壳**：品牌、完整 UI、安装器、特定设备、托管平台或单一工作流。
2. **通用核心**：可以服务多个 Harness 的协议、生命周期、进程、账本、上下文、事件或测试部件。
3. **Harness 专属实现**：只能放进某一个 Adapter 的协议、事件映射、配置探测和恢复逻辑。
4. **拒绝项**：不能成为 GOGO PARTY 权威层的启发式状态、Prompt 权限、非原子 claim、单进程事件总线或产品壳依赖。

复用等级保持四档：

| 等级 | 含义 |
|---|---|
| `SOURCE-CANDIDATE` | 许可证已确认，源码边界相对清楚；仍须完成依赖、NOTICE、平台和测试审计后才能成为 `SOURCE-DERIVED` |
| `SEMANTIC-PORT` | 移植不变量、状态机、失败语义和测试思想，不照搬整体实现 |
| `PRODUCT-PATTERN` | 仅借信息架构、交互和产品表达，重新实现 |
| `REJECT` | 明确不进入 GOGO 的依赖、权威模型或运行路径 |

`SOURCE-CANDIDATE` 不是“可以马上复制”。任何派生文件仍必须记录上游 URL、commit、原始路径、许可证、修改说明和对应测试。

## 本轮新增固定版本

| 仓库 | Commit | 分支 | 许可证 | 证据位置 |
|---|---|---|---|---|
| xAI Grok Build | `b13fa526f5112c0b20dad5f1f2300d3d3b127895` | `main` | Apache-2.0 + notices | `research/upstream/grok-build/` |
| OpenAI Codex | `279b93242cfef379e65da97e87e44b83c5934fd7` | `main` | Apache-2.0 + NOTICE | `research/upstream/codex/` |
| agtx | `6f0d8dec975b4f62ff9a48ec52dbf8cdff92bb04` | `main` | Apache-2.0 | 临时只读审计副本 |
| Clay | `c955515907ca593a8e90f6e56acf90f7fe2663d2` | `main` | MIT | 临时只读审计副本 |
| Open Harness | `b52c82d3af120d0157d60f4a5c36b1abe46285eb` | `main` | Apache-2.0 + NOTICE | 临时只读审计副本 |
| AgentPulse | `2d4bb8a8169f2953e1f10ecb50a2775ae4921aa8` | `main` | MIT | 临时只读审计副本 |
| AgentDeck | `43fff4579c3694bc73b0824580f6aa67217b65ae` | `master` | MIT | 临时只读审计副本 |
| CliDeck | `a88ba92a01295d1626f7c7d19bd9f5052d239728` | `main` | MIT | 临时只读审计副本 |
| Squad | `8146bcc1c38c439aedaf3ff44548c830654c8621` | `main` | MIT | 临时只读审计副本 |

原六个深审仓库的固定版本继续以 [00-scope-and-pins.md](00-scope-and-pins.md) 为准。非官方 Grok App 的两个固定版本和公众 Grok CLI 对照版本继续以 [Grok App 复用审计](../grok-app-reuse-audit.md) 为准。

## 2026-08-20 参考-only 固定版本

本轮只增加两条参考车道。它们不进入 bake-off，不成为依赖，不复制源码，也不改变 P05/CT3 或生产权威。`PRODUCT-PATTERN` / `SEMANTIC-PORT` 不是生产准入。

| 仓库 | Commit | 发行/分支标注 | 许可证 | 证据位置 |
|---|---|---|---|---|
| EKKOLearnAI/hermes-studio | `d6bed4cc550b8e887e69389d00840ac97407fdd1` | 控制器核验全文 commit | BSL 1.1；Additional Use Grant 仅非商业；商业使用需单独许可；Change Date 2029-05-10 → Apache-2.0 | 控制器核验只读锚点；本工作区未落地上游副本。详见 [10-hermes-studio.md](10-hermes-studio.md) |
| deepseek-ai/deepseek-harness | `141eb6fef83422698aef7a981029e843e8161534` | 该 commit 发行标注 `dsh@0.1.0-rc.8`；官方 developer preview，警告破坏性变更 | MIT | 控制器核验只读锚点；本工作区未落地上游副本。npm 证据（2026-08-20，控制器提供）：`latest=0.1.0-rc.7`，`next=0.1.0-rc.8`。详见 [11-deepseek-harness.md](11-deepseek-harness.md) |

Hermes Studio 核验锚点：`README.md`；`LICENSE`；`docs/workflow.md`；`docs/openapi.json`；`packages/server/src/routes/coding-agents.ts`；`packages/server/src/services/coding-agents/runtime/run-manager.ts` 与 `event-mapper.ts`；`packages/server/src/services/coding-agents/{codex,claude-code}/proxy.ts`；`packages/server/src/services/workflow-{manager,socket,schedule-service}.ts`；`packages/server/src/db/hermes/{sessions-db,session-store,workflow-store,workflow-run-store,workflow-schedule-store}.ts`；`packages/server/src/services/hermes/group-chat/{runtime,agent-relay,agent-relay-store,mention-routing,context-projection}.ts`；`packages/desktop/src/main/{updater,runtime-manager,webui-server}.ts`；`tests/client`、`tests/server`、`tests/desktop`、`tests/e2e`。

DeepSeek Harness 核验锚点：`README.md`；`LICENSE`；`docs/architecture.md`；`docs/subsystems/{session,subagent}.md`；`docs/api-gateway.md`；`docs/testing.md`；`packages/subagent/subagent-{codex,claude-code,acp}/README.md`；`packages/workflow/workflow-worker-thread/README.md`；`packages/sandbox/sandbox-windows-acl/README.md`；`packages/sdk/server/README.md`；`python/sdk-runtime/README.md`。

本轮分层补充：

- Hermes Studio：Dashboard、Workflow UX、Group Chat、Electron 打包/更新为 `PRODUCT-PATTERN`；coding-agent 传输/runtime 思想为 `SEMANTIC-PORT` only；无单独授权时商业源码复制与 runtime 直连为 `REJECT`。
- DeepSeek Harness：候选 Harness 执行层或 Adapter 目标，永远不是 GOGO PARTY Control Plane；DSH 序号只映射到 `AdapterEvent.nativeSeq`；subagent finish 只映射到 `NativeTerminalEvidence`；本地 job/goal/schedule/worker 不得替换耐久账本、lease/fencing、spool、`TerminalReceipt`/`ReceiptRecord`/Gate、Project 隔离、Context `LoadProof` 或 Transition Engine；Windows ACL 声明保持 capability-scoped 且 partial。
- 参考负向场景一律 `REFERENCE_REQUIREMENT` / `NOT_IMPLEMENTED`，见 [12-reference-negative-test-matrix.md](12-reference-negative-test-matrix.md)。不得改可执行 Fake 套件，不得声称测试已通过。上游测试为 `TEST-SOURCE` / `NOT-RUN`。

## 证据等级

- `SOURCE`：读取了实际实现和关键条件。
- `TEST-SOURCE`：读取了测试或测试 seam，但未在本机执行。
- `DOC`：只用于定位，不能证明实现成立。
- `NOT-RUN`：本轮没有安装依赖、构建或执行上游测试。

本轮没有修改任何上游副本，也没有连接、检测、启动或调用 Grok App。`<local>\Grok UI` 继续保持冻结且未触碰。2026-08-20 参考车道同样未安装依赖、未复制源码、未执行上游测试、未改可执行 Fake 套件；npm 版本证据由控制器提供，本工作区未查询 registry。

## 本轮最重要的校正

1. **官方 Grok Build 不只是 Grok Adapter 参考。** 它包含明确标注为 host-agnostic 的 lifecycle crate、通用 ACP typed channel 和跨平台进程树 scope，应优先于非官方 App 中的同类代码进入候选比较。
2. **“原子文件写”不等于并发账本。** AgentDeck 的 `sessions.json` 用临时文件重命名避免半写，但没有跨进程锁，不能承担 claim/lease authority。
3. **“有 claim token 字段”不等于原子 claim。** AgentPulse 当前实现先查询 `validated` 行，再仅按 ID 更新；竞争者可覆盖 claim，不可直接借为任务账本。
4. **“有 lease 字段”不等于 fencing。** Squad 的任务 ACK 使用条件更新，但完成时没有校验租约是否过期，也没有单调 fencing token。
5. **Hook 优先、PTY 降级是正确方向。** AgentDeck 的 Codex turn manager 明确让新鲜 hook 压过 PTY parser，并为漏失 stop 建立恢复边界；这适合移植为事件权威规则，但其时间窗口不能成为跨 Harness 的统一常量。
6. **跨 Harness 指令投影必须比文件拼接更严格。** Clay 的实现证明统一扫描有产品价值，但其根目录文件表和固定优先序不足以处理祖先/子目录、必选片段、冲突和哈希证明。

## 2026-08-20 参考车道校正

1. **BSL 1.1 不是 Apache-2.0 预支。** Hermes Studio Additional Use Grant 仅非商业，商业使用需单独许可，Change Date 2029-05-10 才转为 Apache-2.0。无授权复制或直连 runtime 为 `REJECT`。
2. **产品壳可以参考，不能变成 Control Plane。** Hermes Studio 的 Dashboard、Workflow UX、Group Chat 和 Electron 更新只借信息架构；审批 UI 成功不等于 Gate PASS。
3. **双 Session 模块是 GOGO PARTY 参考缝。** `sessions-db` 与 `session-store` 并存，要求 Coordination Plane 只有一个 Session 权威身份。这是参考需求，不是对上游当前失败的指控。
4. **DSH 是执行层候选，不是指挥面。** DeepSeek Harness 的 goal/plan/schedule/worker、Cordis 插件和 SDK 不得替换 GOGO PARTY 账本、lease、spool、Receipt/Gate、LoadProof 或 Transition Engine。
5. **原生序号不是账本序号。** DSH SessionEvent 只进 `AdapterEvent.nativeSeq`；subagent 结束只进 `NativeTerminalEvidence`。
6. **Windows ACL 包不等于隔离完成。** `sandbox-windows-acl` 保持 capability-scoped 且 partial；不支持或不完整时 fail-closed。
7. **developer preview 的包漂移是调度拒绝项。** 源码 pin `dsh@0.1.0-rc.8` 与 npm `latest=0.1.0-rc.7`、`next=0.1.0-rc.8` 并存；若部署解析 `latest` 或版本未知，不得套用 `rc.8` 证据，只有精确解析并验证 `rc.8` 后才可继续对应 Gate。
