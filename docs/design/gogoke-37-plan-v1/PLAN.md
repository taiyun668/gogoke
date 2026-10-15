# 设计 37 施工计划 v1：按分布式施工切分

**状态：DESIGN_PROPOSAL / NOT_AUTHORIZED。**本计划不恢复、也不改变 S1-R4 的施工和授权。

- **权威定义在 `PLAN.json`**：线和阶段、写入范围、共享文件、跨线约定、验收项、证据沿用规则、Owner 触点，都以它为准。本文是给人读的说明。
- **校验**：`python docs/design/gogoke-37-plan-v1/verify_plan.py --self-test --manifest --sources-ref origin/main`。
  - 它按脚本里**写死的必需清单**来检查计划，不依赖计划自己列了什么：设计 37 的全部需求 ID、10 组约定、必需的依据文件、授权顺序。
  - 它只证明这些**具名的结构检查**通过。它不读自然语言，也无法核实某个验收项的证据是不是真实的，更不证明任何代码能跑。
  - 自检会对计划做 30 种人为破坏，每一种都必须被抓到，其中包括复审构造的那些反例。
- **对应设计**：[37 席位与会话架构](../37-seat-session-architecture-draft.md)。
- **两轮审查的回应**：对照表见文末。

## 1. 基线与授权顺序

- **映射快照**：main `5b638c73`。R2-05 那次交付经 Owner 接受后，由 PR #27 合入 main，所以现在交付的产品代码和最新的治理、设计都在同一棵树上。旧快照（main `d0e739be` 加施工分支 `df63d8b7` 两棵树）已作废。主控的 T00 记录 `artifacts/gogoke-37/t00/T00-RECHECK-161448a4.json` 查出了它的偏差，本次修订已经吸收。
- **施工基线**就是映射快照这棵 main。授权前再做一次 **T00：在授权所依据的提交上重新核对**全部共享文件和资产，并把带条件的共享文件逐个登记为"要改"或"核对过不用改"。跟映射快照相比有任何变动，就修订本计划。
- **顺序**（校验脚本强制，必须完全一致）：
  1. 草案审查；
  2. T00：在授权所依据的提交上重新核对，并跟映射快照比较；
  3. 机器计划和 MANIFEST 定稿，合入 main；
  4. 另开一个单独的授权 PR；
  5. 合入后按实际读回的内容核对授权；
  6. 第 0 步（L0）；
  7. 各线并行。
- **授权**照搬 S1-R4 的形式，换一个独立路径：`artifacts/gogoke-37/intake/PUBLIC_AUTHORIZATION_RECEIPT.json`。授权绑定的是**范围摘要**（`verify_plan.py --scope-digest` 算出），覆盖需求、线及其写入范围和依赖、阶段所属里程碑、里程碑范围、共享文件的归属和路径及条件规则、约定的持久写方归属、不变量、Owner 触点、治理、授权、不承诺事项。**只改细节**（交付、约定及验收项的描述文字、资产、依据文件版本）不改变摘要；阶段归属、里程碑范围、条件写域和持久写方归属属于范围。独立审计仍须核对需求、不变量、安全边界和已记录的 Owner 决定未被改写：经独立审计通过后由主控合并，不需要新回执，也不需要 Owner 合并。**改到范围**时，在同一个 PR 里更新回执，由 Owner 合并这一个 PR。S1-R4 的授权不覆盖本计划，也不改动。**合并本计划，就等于 Owner 接受用这套授权方式来管设计 37。**
- **施工派活**依据 `AGENTS.md` 和 `docs/model-routing.md`，不拿设计 37 里的产品规则来授权。计划只切线、不定人。

## 2. 现有资产

分成"已有的数据约定 / 已接通的实际行为 / 本轮新做"三列登记（见 `PLAN.json` 的 `assets`）。本轮更正的几处：

- **事件**：`store/orchestration.rs` 里已经有消息、活动、会话、回合、审批事件。A 线做厂商中立的席位账本，由 L0 定下旧事件怎么并入，只留一份权威记录。
- **工作树**：Tauri 已经能建 Git 工作树。F 线把这部分逻辑移进宿主；旧入口只留给旧的 Codex 线程。
- **问题卡**：`RequestUserInputMessage.tsx` 已经有选项、自由补充和按问题回答。所以 G 线包装复用它，C 线只新做厂商中立的约定、席位和回合的绑定、持久状态和恢复，不从零画一张新卡。
- **额度**：首版不接用量查询或模型消耗探测；只如实显示 CLI 自己报告的额度耗尽、错误和明确给出的恢复时间，未知就显示未知。

## 3. 第 0 步（L0）：共享接线和跨线约定

- **共享文件 61 个**，唯一的写入者是汇合席位（只有 L0 这一条线拥有它们，校验脚本检查）。
  - 本轮补上了设置页的四个接入文件，因为实例管理页放在设置里。
  - Owner 新指定的右侧席位入口再精确加入 7 个接线文件：`PanelTabs.tsx`、`layoutNodes/buildGitNodes.tsx`、`layoutNodes/types.ts`、`useGitPanelController.ts`、`useMainAppComposerWorkspaceState.ts`、`useWorkspaceFileListing.ts`、`useMainAppLayoutSurfaces.ts`。它们只扩展四选一 tab、布局 source 传递和文件模式类型；`MainApp.tsx` 与 i18n 已在清单中。`PanelShell` 以及 Git/文件/提示词面板从 `PanelTabId` 继承，不另开写域。
  - 另外 5 个带条件的文件：问题卡组件、`Cargo.lock`、卫生检查脚本和它的测试、服务打包脚本。这几个由 T00 或 L0 登记为"要改"或"核对过不用改"；哪条线都不能为了够着它们而扩大自己的写入范围。
- **10 组约定**，每组写明：
  - 持久状态归谁；
  - 谁来用；
  - 在哪个汇合点用真实实现结算（M3 时全部在候选上再结算一次）；
  - 跟别的线怎么接。

  本轮补上的接缝：
  - **准入只有一个边界**：由 H 在现有的协调域里做预约、提交、释放。E 的同时运行上限、F 的实例并发，都只是这个边界的输入，不各自另记一份。
  - **压缩和换会话**：E 通过会话约定向 H 和 B 发请求，拿回"能力不够"或失败的回执，以及新旧会话代次的切换。E 不自己拼各家私有的命令。
  - **收件箱投递**：走 H 现有的"准备 → 开始提交 → 完成"那条链，并在投递时重新核对当前授权。
  - **旁聊的持久化**：D 有自己的原生存储，放在同一个协调域里；旁聊自己的回合作为旁聊层事件记进账本。
  - **秘书长的触发器**：放在现有的协调器里，能登记、恢复、取消、去重，不另建第二个调度器。
  - **手动升级之后**：重新固定程序身份，重新探测能力。
  - **清理工作树**：只在停止事实到达、并且没有占用中的预约之后才做，不凭目录名判断。
  - **界面**：每个读取模型和操作都对应到上面某个约定的操作和回执，G 不自己加操作或状态来源。

## 4. 各条线（按阶段交付）

| 线 | 阶段与内容 |
|---|---|
| **H 宿主会话、准入与权限** | H.1（M1）：固定程序身份 → 授权启动 → 输入输出事件 → 停止事实 → 恢复；统一准入；实例目录和按会话的临时目录；显式环境变量；权限档位按实际可用的机制强制执行，保证不了的档位直接拒绝；撤权；压缩和换会话命令 |
| **A 账本与上下文** | A.1（M1）：统一事件，三个标签加旁聊层；重启对账；按作用域查询和实时订阅；交接材料；会话用途和可读来源的规则 |
| **B1 Codex** | B1.1（M1）：协议、续接、插话、只追加不回答、原生问题卡、压缩、能力清单、关闭记忆的配置 |
| **B2–B5 其余四家** | B2–B5.1（M2）：同上，再加各家的记忆和指令文件行为。每家单独实测 |
| **C 收件箱与问题卡** | C.1（M1） |
| **D 旁聊** | D.1（M2）：包括原生存储 |
| **E 席位、编排与规则原语** | E.1（M1）：登记、分层、绑定；E.2（M2）：编排范围、上限、模板、接手问答、状态卡、健康信号、调用权限表、关口和阶段、升级链；E.3（M3）：秘书长的触发器 |
| **F 实例与工作树** | F.1（M1）：登记、独立目录（关闭记忆）、登录状态；F.2（M2）：版本、升级后重新固定、并发、工作树、图谱、合并、清理。席位只在宿主创建并授权的工作树里改文件；停止事实与无占用条件满足后，由 F 用原注册 Git pin 封装席位改动并执行已有合并。 |
| **G 界面** | G.0（M1）：浏览器预览（本机 vite，Tauri 桥背后接 K-UI 假实现，改界面几秒可见，不用原生构建、签名、装机）；最小实例页，含实例列表和 Codex 测试实例的四种状态（未安装、未登录、已登录、出错），从实例页一键登录；新版本提示和通用额度耗尽仍在 M2；G.1（M3）：右侧第 4 个席位 tab 与其余页面和卡片，复用现有的问题卡组件 |

### K-SEAT 席位期限创建细节变更请求（2026-10-09；仅文档请求，未施工）

在既有 `create-from-template` 操作中，Owner 直接创建的 USER 席位保持 LONG；由已认证的 H/LEAD 为单项任务新建的直属 LEAD 子席位默认 SHORT。请求字段和操作家族不变，模型不能提交或选择席位期限。Lead 需要保留长期岗位时，使用现有 `short-to-long` 操作把已空闲的 SHORT 子席位转为 LONG；原有权限、作用域、并发、StopFact、回收条件和状态转换约束不变。

这项细节不改写既有数据：更新前创建的 LONG/SHORT 席位保持原期限；旧操作请求仍按原始 kind、指纹和回执恢复，不能按新默认重解释或重写；不迁移、补写或回填席位。V06 的真实检查必须从原 H/LEAD 创建回执和 state-card 读到新子席位为 SHORT，再用原生 `short-to-long` 收据和读回确认其变为 LONG。这里是 K-SEAT 冻结合同的细节变更请求，不表示产品代码、测试、云端或真机已经实现或通过。

### H 的原生会话恢复细节

原生绑定（NativeV2）把 E 席位授权与 H 单个会话的物理进程代次分开。首次准入必须有 H 自己记录的原生关联和选定实例，实际启动时封存原授权、席位 incarnation、实例和 home；缺少旧绑定记录不是原生授权。恢复只允许同一份未撤销授权、同一选定实例和原有作用域，逐项核对原记录及当前 E 授权；不自动采用后来的席位配置。符合这些条件时，保留 E 的 generation 和 revision，只推进该 H 会话的物理代次、home 与进程 episode。另一个仍在运行的主会话或旁聊继续使用自己的当前 H 证明，不因同席位另一个会话恢复而失效。

原 continuation 仍要求已确认的 STOPPED、原协议回执和原绑定；持有者消失不能代替 StopFact 或授权续接。Job/custody、pid 与创建时间、ticket、nonce 和数据库事务核对仍沿用现有 H 边界；旧物理代次不能发起新的模型操作。UNKNOWN 对账也必须走相同的原绑定核对。所有未释放的同席位占用（包括已停止但尚未释放的会话）都阻止调整作用域；只释放其中一个会话不能把仍有占用的 E 席位置为空闲，最后一个占用释放后才解除占用。已释放或已撤销的会话不得恢复。旧绑定（LegacyV1）的兼容行为保持原样，不把旧记录升级解释为原生授权。

验证覆盖同席位主会话与旁聊同时存在、各自停止并恢复、另一个会话继续有效，原授权不变而 H 物理代次改变；错实例、错 home、错 incarnation、授权变动、旧物理证明和缺原绑定均拒绝。这里细化既有 H 恢复和统一准入，不新增权限、持久写方、写入范围或 Owner 触点；原生验证只在云端执行，稳定点再在已安装候选上实测。

### Claude 冷恢复的 ACK 与 init 时点细节（2026-10-11；审查草案）

固定 Claude Code 2.1.196 的原实际协议结构显示：首次合法 USER 前只有 initialize ACK，`system/init` 在 USER 后出现；后续 USER 可再次收到同一厂商会话 ID 的 init。冷恢复不能要求 ACK 后、首 USER 前已取得 init，也不能把 ACK、旧代 init 或 `--resume` 参数当作新代 init 来源。此修订只纠正证明时点，不把原 ACK-only UNKNOWN 请求升级成成功。

新合法冷恢复仍先核原 STOPPED/StopFact、未撤销绑定与当前授权、实例/F 私有历史、原已完成 A/H 厂商会话目标和新代 custody，再核新代 initialize 原命令与同 request ID 的真实成功 ACK。此时仅认 transport READY；回执 `readinessBasis=ORIGINAL_CLAUDE_INITIALIZE_ACK`，恢复身份仍未确认，对外观测 `threadId` 尚未确认时为 null。原目标 ID 只供受约束的恢复选择，不冒充已观察的新代身份。UNKNOWN 旧请求、旧 ACK 或旧物理证明不能借此成为新的 USER 写入依据。

首次真实 USER 必须重新经过原 H prepare/begin、当前授权与物理调用证明，并且只写一次；其准入使用上述原 ACK 与封存目标，不能再把尚未发生的新 init 当作首 USER 前置。实际同代、同进程 custody 的 init 必须匹配原 STOPPED 会话已完成 A/H 的厂商 ID，才填写观测 threadId；多条相同 ID 的 init 保留第一条原 source，并逐条核一致，异 ID 拒绝。持久 completion 和成功回复读回都核同一原 ACK/绑定、原目标、同代 init、实际 USER 回显、result 与 A 来源关系；到结算或成功读回仍缺 init，或异 ID、错代/错 custody、来源不完整、关系不一致时，保留原错误和已写 USER 的 UNKNOWN，禁止成功交付，不重发、不换会话、不伪造历史。

这里允许的是既有合法 USER 在已准入 transport 上的一次调用；实例、模型、effort、权限档位、F 历史可达域、LPAC/capabilities、CLI 固定字节及模型调用授权均保持。READY 不声明恢复上下文已证实，也不使模型回复或 `turn/start` 自动成为 completion；不加入等待、重试或 dummy 输入。验证沿原黄金协议与安全边界覆盖 ACK-only 就绪但身份未确认、原 STOPPED/ACK 拒绝、首次 USER 的同 ID/异 ID/缺 init、重复同 ID 首 source 保全以及完成/读回一致性；固定 CLI 无模型 ACK 对照不证明真实上下文恢复，原两轮与停止后恢复的上下文用例仍须实际端到端结算。

参照：gogo-party `packages/seat-runtime/src/claude-seat.ts` 在真实事件流中由 init 更新厂商会话 ID，启动回执不充当 init；NaveHQ 的 host 证据/原请求恢复原则和 LoomOS 的已有轮子记录未提供可直接复用的 Claude 冷 ACK 身份实现；仓库 `docs/research/2026-09-25-substrate-source-teardown.md` 的 SDK 路径仅作线索。官方 [Python SDK Query.initialize/control_response](https://github.com/anthropics/claude-agent-sdk-python/blob/b6e9d12fe1cc98dde988ab7b7713c1feeee50c6c/src/claude_agent_sdk/_internal/query.py#L339) 与 [Client 的 initialize 后 USER 写入](https://github.com/anthropics/claude-agent-sdk-python/blob/b6e9d12fe1cc98dde988ab7b7713c1feeee50c6c/src/claude_agent_sdk/client.py#L242) 采用分离的 ACK 与数据消息；采用此顺序，不采用 SDK 的额外等待、后台任务或权限机制。固定 CLI 行为依据其原实际帧，SDK 源码和 mock 测试不替代本体。

此草案仅细化既有 H/B2/A 恢复与 completion 约定，不增加写域、共享文件、持久写方、权限或 Owner 触点，scope digest 与原 v2 回执不变。按 amendment_rule，由新鲜独立审计确认边界后，Controller 合并细节修订才生效；本包停在产品施工前。后续开发遵守 [构建与发布治理](../../governance/gogoke-build-and-release.md)：本机开发先实际读 SAC=0，独立数据与产物/临时置于 D；正式原生检查在云端，安装、真实模型及验收结果本包均 NOT_RUN。

### OpenCode 自有 uv 管道的 LOCAL 接线细节（2026-10-11；审查草案）

固定 OpenCode 1.18.32 的开发原件在 LPAC、真实 F 路径别名与特定实验环境下取得 initialize ACK，原 Stop/Job 清零；它没有发送 session/new 或模型，清理校验原失败仍保留，后续临时 profile 清理成功不改整例结论。无兼容模块的原对照也有路径别名和不存在的 GIT_DIR，但 initialize 超时；这只支持验证自有 uv 管道的 LOCAL 名称机制，不能证明完整产品或模型已通过。

仅在 H 既有 process/session/compat/build.rs、compat/native/shim.cpp、compat/src/lib.rs、process/session/compat_module.rs 与 store/session_transport/launch.rs 接入正式固定 OpenCode mode。沿用已有模块生成、实际悬挂进程映像和嵌入字节核验，限定上述固定 CLI 字节、当前进程及精确 uv 数字叶名的成对 CreateNamedPipe/CreateFile 映射；其他名称和参数直通，原安全参数与 Windows 错误保持。映射只供同一 AppContainer 内该进程的内部 IPC，不授予其他身份、路径或通用管道访问。

产品保留真实已登记 Git F 与现有 H 自有路径别名的意图、物理 FileID、宿主登录命名空间、原 StopFact 及精确撤销。不得加入实验用不存在的 GIT_DIR，不隐藏、替换或伪造真实仓库；不加 PATH、Git common 目录许可、卷根 ACL、能力、登录或权限档位，不改固定 CLI 字节，不搬入开发 ACL/清理夹具，不忽略产品 DACL control 或有序 ACE 的原校验。若实际结果仍需要这些边界外行为，保留原错误，回到已有 Owner 条件触点，不自动扩展。

先在上述唯一变动下验证真实产品 H 的 initialize、session/new 与模型/强度设置原 ACK；缺其中一项便不算 open，更不算模型通过。随后沿既有真实端到端用例核已授权实例的一次 USER、原 A/H 完成、同代身份、Stop、冷恢复与正式保护；未知不重发旧输入。安全边界仍核固定字节与 mode 拒绝、其他管道直通、原隔离/权限、模块来源和路径别名生命周期。开发结果、源码、单独 ACK 和云检不能替代真实安装结果。

参照：先核 gogo-party 的 accounts.ts 与 ACP 席位运行时，其收口环境和真实 cwd 原则沿用，但没有 LPAC uv 管道实现；NaveHQ 的 cao_runtime_adapter/agent_runtime_port 与 LoomOS 事实源、施工图未提供此实现。仓库 parts/substrate teardown、reuse-blueprint、upstream-reference-map 与 execution-layer-capability-table 没有可直接搬入的该兼容路径。采用本仓库既有 Claude H 自有兼容模块和路径别名生命周期，差异仅为固定 Bun/OpenCode 的 uv 管道模式。固定 OpenCode 官方 core project/git 与 ACP/effect-cmd 实现没有公开的保留 Git 身份并跳过发现的参数；Bun 的 --no-git 只控制 bun create，不作为产品启动方案。

此草案只细化既有 H/B3，五路径已在 H 写域；PLAN.json、scope digest、v2 回执及三处固定值均不变，不新增 Owner 步骤。独立审计通过、Controller 合并并精确读回后生效。正式原生检查在云端，本机开发遵守实际 SAC=0、独立数据、D 产物及并行上限；稳定点仍须重新开启 SAC 后实测，不自行验收。

## 5. 汇合点

汇合点包含哪些线，由各线的阶段推导出来，不另外手写名单。

- **M1 一个真席位**：一个 Codex 席位在自己的实例目录里跑通。
- **M2 一个项目跑起来**：五家都接上，接手、编排、隔开写和合并、旁聊、规则原语、隔离测试。"接手 → 派生 → 写 → 合并"只是一组可配置的验收场景，不是产品唯一的流程。
- **M3 候选**：通过真实界面、从安装好的候选版本走一遍，加上秘书长；主控在 Owner 的 Win11 上实装实测，Owner 看证据决定是否接受。

## 6. 验收

一共 21 个验收项，每项写明要观察到什么、在哪个汇合点用真实实现结算。

- **约定分三次结算**：V00a（M1）、V00b（M2）、V00c（M3，全部 10 组）。每一组都要在它自己的结算点被结算一次，并且在 M3 再结算一次，由校验脚本检查。
- **问题卡**：Codex 的原生卡在 M1（V03）。"没有原生卡的厂商用我们的卡"放到 M2 的真实厂商上验（V03b）；如果五家都有原生卡，记为没跑（NOT_RUN），不记通过。
- **记忆关闭分开验**：
  - 独立目录和凭证不被读取（V04，M1）；
  - 每家实际生效的关闭记忆配置，以及同一个实例同时服务两个项目时互不串（V04b，M2，五家逐个验）；
  - 零继承的正式审查，也按厂商逐个验（V10）；
  - 某家做不到的，记为没跑，不能拿"启动成功"来顶。
- **旁聊的注入测试**：在旁聊自己有权写的工作树里放一个目标。没有 Owner 新的明确要求，就不能有任何工具调用、文件哈希不变；有明确要求时，写入能成功（V12）。
- **补上的子断言**：
  - 模板复制后独立修改，另一个项目不受影响；用户层能调整和回收主控层的席位；真实 H/LEAD 新建的任务子席位为 SHORT，并用原生 `short-to-long` 操作和回执转为 LONG（V06）；
  - 关口"通过"和"打回附理由"、合法的阶段流转（V08）；
  - 旁聊保留后重新打开、归档后恢复、主控上下文不存第二份（V12）；
  - 手动升级后重新固定身份（V13）；
  - 保证不了的权限档位被拒绝、主树不能直接写（V11）；V11 同时核对席位只改宿主工作树文件、不要求 CLI 调用 Git 或访问 `.git`，以及 F 在原 StopFact/无占用约束下封装的本地提交身份和绑定原 MERGE request 的封装 intent、child commit 原 hash 读回。
  - 界面验收明确列出全部 25 个状态，每个状态下的禁用操作和恢复都要验，**缺一个就记为没跑，整项不能算通过**（V15）。
- **证据沿用**：每个结果都绑定它所用的源码提交和组件哈希。M1、M2 的结果，只有在候选上涉及的组件和配置逐字节不变时，才能沿用到 M3，否则在候选上重跑。V16 给出逐项的来源表。结果只有三种：通过、失败、没跑。
- **故障注入**：在真实的边界上注入故障，不用替身后端，也不为了测试去耗尽真实的订阅额度。
- **模型自己说"没看到""没执行"，不算证据。**

## 7. 构建与发布

照现行的[构建与发布治理](../../governance/gogoke-build-and-release.md)办，不另立规矩：
- 原生代码和测试在云端跑，CI 任务有超时，新推送取消旧运行；
- JS/TS 测试在本机只用签名的 Node 跑；
- 不以代码签名为前提，不改安全设置，通过的结果不外推；
- 更新链不变；`SHA256SUMS.windows` 用 Owner 私钥签署，主控按 Owner 2026-09-25 的授权可以在本机签，公开发布仍由 Owner 决定；
- 证据公开前先过卫生检查；
- 五家 CLI 由 gogoke 在自己的受管目录安装固定官方版本，绝不读取或修改用户全局 npm/PATH 中的 CLI；逐个在受信目录锁定官方来源和预期制品哈希，核对下载完整性与实际可执行文件哈希及 Win11 智能应用控制下的启动结果。

## 8. Owner 出面

- **计划内的**：
  - OT1：合并计划；
  - OT2：合并授权；
  - OT3：设定测试项目的用户层配置和测试仓库（包括项目并行上限和每个测试实例的并发上限），可以跟 OT2 一起回答；
  - OT4：各实例登录。M1 前登录 Codex，M2 前登录其余四家；都在实例页上一键发起，浏览器自动打开，进度自动刷新；
  - OT5：根据证据决定是否接受 M3 候选版本。在 Win11 上安装和实测由主控按治理文档里的普通视图做法完成，Owner 不用自己动安装包。候选字节在 Owner 决定之后又变了：由主控对新候选重跑受影响的实机检查，只有"接不接受新候选"再交给 Owner，也就是再走一次 OT5。
- **条件触发的**：
  - 登录失效；
  - 智能应用控制拦下了新组件；
  - 跟设计或已定的决定冲突。
- **不在本计划内**：公开发布。

## 9. 不在本计划里的

- 第二批：决策登记册、额度、长期记忆。
- 用法：岗位怎么设、合并权限给谁、流程怎么走。

## 10. 两轮审查的回应

**第一轮（review 5327521026）**：13 项，复审判定 8 项已解决、5 项部分解决；部分解决的，已并入下表的第二轮各项。

**第二轮（review 5327655159）**：

| 复审项 | 本版怎么处理 |
|---|---|
| F01 P1 校验器会放过缺内容的计划 | 脚本内置必需的需求 ID、约定、依据文件和授权顺序；ID 必须唯一；共享文件只能有一个拥有者；实现线必须排在 L0 之后；复审构造的反例全部加进自检；输出改称"具名结构检查通过"，并写明 REAL 只是声明 |
| F02 P1 V00 在 M1 要求全部约定到齐；V03 藏着跨阶段依赖 | 线改成按阶段交付，脚本检查每个验收项引用的阶段不晚于它的汇合点；约定分 V00a/b/c 三次结算；V03 拆出 V03b 放在 M2 |
| F03 P1 旁聊没有原生持久落点，接缝未定 | D 加了自己的原生存储，放在同一个协调域；10 组约定都写明持久归属、消费者、结算点和接缝，包括准入、压缩、投递链、触发器、升级后重新固定、清理条件 |
| F04 P1 V04 证明不了记忆已关 | V04 只验独立目录和实际配置；新增 V04b，五家逐个验关闭记忆和同实例多项目；V10 按厂商逐个验；做不到的记为没跑 |
| F05 P2 写入范围可以用路径别名绕过 | 只接受规范的仓库相对路径，拒绝绝对路径、反斜杠、`.` 和 `..`；比较时不分大小写；加进自检 |
| F06 P2 共享文件还漏了设置入口 | 补上设置页四个文件；另列 5 个带条件的文件，由 T00 或 L0 登记"要改"或"核对过不用改" |
| F07 P2 V12 的注入测试有歧义 | 改成在有权写的范围内做注入：没有明确要求就不能动，有明确要求才能写 |
| F08 P2 子断言缺失；M3 怎么结清没写 | 补上各项子断言；V15 必须覆盖全部 25 个状态，缺一个就算没跑；新增证据沿用规则和 V16 的逐项来源表 |
| F09 P2 问题卡被写成"没有" | 资产表更正：复用现有组件，只新做厂商中立的部分 |

## 按 R2-05 交付重新对齐（2026-09-29）

| 变动 | 处理 |
|---|---|
| 交付代码原来只在施工分支上，映射要跨两棵树 | PR #27 合入后，映射快照改为同一棵 main `5b638c73`；T00 改为授权前在授权所依据的提交上重新核对 |
| 共享文件有 9 个随 R2-06 修复变了 | 逐个看过：都是在原文件里加功能，计划写的扩展点仍然成立，共享文件清单不变 |
| 依据文件有 3 个变了（`AGENTS.md`、构建与发布治理、S1-R4 计划） | 固定的 blob 改为 main 上的新值；`AGENTS.md` 新加的两节（先查逻辑硬伤、取直接证据）直接适用于本计划的施工 |
| 签名方式已改为主控经授权在本机签 | 治理条款和不变量的说法跟着改；更新链本身不变 |
| OT5 原来要 Owner 在 Win11 上自己验收 | R2-06 已经证明主控能用普通视图在 Owner 机器上装卸实测；OT5 缩成"看证据决定是否接受"，少了一次 Owner 亲自动手 |

## 第三轮审计的回应（2026-09-29）

| 发现 | 处理 |
|---|---|
| P1 设置里的实例入口缺接线文件 | `SettingsNav.tsx`（写死的导航列表）和 `i18n/index.tsx`（英文与中文标签键）登记为共享文件，由 L0 开扩展点；共享文件增加到 52 个 |
| P2 T00 的机器顺序还指向旧快照 | T00 的标题和校验脚本里写死的授权顺序，都统一为"在授权所依据的提交上重新核对，并跟映射快照比较"；其余顺序不变 |
| P2 CT5 不是只有 Owner 能做的事 | 从条件触发里删掉；写进 OT5：字节变了由主控重跑受影响的实机检查，只有接受新候选这一步再交给 Owner |

## M1 准入的两个上限（2026-09-29）

| 发现 | 处理 |
|---|---|
| 主控在 L0 施工中发现：H.1 要在 M1 完成准入，准入要用 E 的项目并行上限和 F 的实例并发上限，但计划把这两项排在 E.2、F.2（M2），M1 没有权威来源 | E.1、F.1 在 M1 各交付一个经校验、持久存储在原生库里的用户层字段，由 H 的准入直接读取；缺值时拒绝准入，不编默认值。E.2 改为宿主按本机资源算出一个上限（机械事实，连同输入一起记录），准入取它和 Owner 设的项目上限中较小的一个；宿主从不改 Owner 的字段；F.2 去掉并发一项。K-SEAT、K-INSTANCE 的结清说明同步修改。OT3 写明这两个上限由 Owner 设定 |

| PR #55 聚焦复核：M1 验收没覆盖两个上限 | V00a 写明 K-SEAT、K-INSTANCE 的上限用例，假实现和 M1 真实实现都要跑：任一上限缺值就拒绝准入；两个上限都设了之后，H 的同一个准入边界超过任一上限就拒绝、释放后能重新准入；重启后值还在；写入非法值被拒 |
| PR #55 聚焦复核：E.2 的"建议值"怎么生效没说清 | 不再用"建议值"：宿主算出的机器资源上限是机械事实，准入取较小值，Owner 的字段只有 Owner 能改；V06 同步改为验证负载下不超过这个较小值、且 Owner 字段没被宿主改动。不新增 Owner 触点 |

## 实例页和登录提前到 M1（2026-10-01）

| 发现 | 处理 |
|---|---|
| OT4 要求在 M1 登录，但实例页排在 M3 的 G.1。主控只能把登录塞进旧的 Codex 设置页：请求 ID 存在页面组件里、关页就丢，只显示设备码原文，要 Owner 自己补流程 | F.1 加一项"登录会话由宿主持有"：开始、进度、结果、取消，在关页、重启界面后都还在。新增 G.0（M1）：通过已开好的设置插槽做最小实例页，从实例页一键登录，宿主打开浏览器，设备码可以复制，进度自动刷新；旧 Codex 设置页不再放登录入口。K-UI 的实例读模型和登录动作提前到 M1 结清，其余仍在 M3。V00a、V04 写明一键登录，以及登录到一半时关页、重启界面后能接上；任何一步都不让 Owner 手动复制请求 ID 或改设置 |
| PR #58 聚焦复核："A4 状态"会把 M2 的内容拉进 M1 | G.0 和 K-UI 在 M1 只结清四种状态：未安装、未登录、已登录、出错（含登录失败的原始原因）；新版本提示和通用额度耗尽仍按 F.2、V13 在 M2 结清 |
| PR #58 聚焦复核：登录的两种结果没有验收 | F.1、K-INSTANCE 写明宿主保留结果：失败存 CLI 的原始原因；成功后由宿主自己检测登录状态，转为已登录。V00a 增加登录用例（假实现和真实实现都跑）：失败读回为出错并带原因，成功读回为已登录，界面重启后读到的是同一会话、同一结果。V04 增加：在真实 CLI 边界上制造一次失败，显示原始原因；真实登录在浏览器授权后，无需任何操作就转为已登录；关页或重启界面后两种结果都能原样读回 |

## 更高效的施工过程（2026-10-02）

| 变动 | 处理 |
|---|---|
| 授权绑定计划文件字节：改一个细节就要重新授权，签名链还要再改绑一次，Owner 每次合并两到三个 PR | 授权改为绑定范围摘要。只改细节由主控在独立审计通过后合并；改到范围时回执在同一个 PR 里更新，Owner 只合并一次。回执升级为 v2 |
| 每改一处都签名、装机、上 Owner 机器实测（例如为验证一个想法出 0.1.16） | `AGENTS.md` 新增"阶段内轻，稳定后重"：阶段内每次推送只跑云端检查；只有 Owner 机器才看得到的事攒到稳定点一次实测，卡住阶段时才提前单测；完整更新链只在阶段稳定后走一次 |
| 小事也找 Owner | `AGENTS.md` 新增"谁来决定"，列出只归 Owner 的几类，其余主控决定并记理由 |
| 计划没写到的细节按自己的想法做 | `AGENTS.md` 新增"先查现成做法" |
| 到了界面和交互阶段，每改一处细节都走完整链不现实 | G.0 在 M1 交付浏览器预览：本机 vite 加 K-UI 假实现，改完几秒就能看到，Owner 也可以直接看；真实数据和真实应用窗口到稳定点再验 |

### PR #61 独立复核修订

范围摘要补入阶段归属、里程碑范围、条件共享文件规则和持久写方归属；描述性交付、约定及验收文字继续作为细节。计划的回执 schema 统一为 v2，自检覆盖范围变动拒绝及描述细节可改。三处候选签名链固定回执 blob，以可信 main 的校验器只读候选计划数据，并从 main 第一父链机械定位引入回执的 Owner 合并提交。阶段内撤掉自动签名入口，保留稳定点显式派发。

## M1 期间攒下的规则（2026-10-03）

| 变动 | 处理 |
|---|---|
| 两次计划缺陷（PR #55 准入上限、PR #58 实例页）都是时间错位：某时刻要用的东西排在后面的阶段 | 每个计划内 Owner 触点写明所属里程碑和依赖的阶段；阶段可写 `needs`（H.1 需要 E.1、F.1，G.0 需要 F.1）。校验脚本检查依赖的阶段在该时刻之前或同时交付。OT4 拆成 M1 的 Codex 登录和 M2 的其余四家登录 |
| 计划没写到的细节被按自己的想法做 | 每条线（T00 除外）写明 `precedents`：参照的现成做法，或查过哪些；校验脚本检查 |
| M1 卡住的 bug 全是在真机上发现的，大量轮次花在测试夹具上 | 治理规则和 `AGENTS.md` 写入：端到端为主、各家 CLI 录黄金样本、单元和变异测试只留安全边界 |
| 检查点越写越重 | 阶段内检查点只写改了什么、结果、下一步、参照和失败原文；完整证据在稳定点整理 |
| 286 个提交全由主控串行完成，没有并行 | 治理规则和 `AGENTS.md` 写入：约定和假实现就绪后，各线由工人席位并行，集成席位合并并跑端到端 |
| 挪动阶段也算改范围，要 Owner 合并 | Owner 决定放开：范围摘要不再包含阶段归属和里程碑说明；挪动阶段属于细节，审计通过后由主控合并。授权回执随本 PR 更新为新的范围摘要 |

M2、M3 的时间重估和 M1 的其余教训，在 M1 完成后以细节修订补入。

## CLI 与实例管理修订（2026-10-06 Owner 决定）

Owner 已决定由 gogoke 自己安装和管理五家固定官方 CLI，提供安装、显式点击升级或退回、自检、查更新和零实例卸载；用户点击后由宿主自行下载、核验和安装，无需 Owner 手工下载安装 CLI；版本切换遇活跃或 UNKNOWN holder/session 一律拒绝，不触碰现有 home 或凭据。这个决定取代 F.2、A4、V13 和 CT2 的“Owner 手动安装/升级”条款，不增加登录触点。原实例只有在原物理 home 与认证对象仍可验证、没有未结 intent 或活跃/UNKNOWN holder、受管官方可执行字节满足原 pin 或完成静止 repin 与重探后才能沿用；原 UNKNOWN 和停止事实不被重写。B5 的 Antigravity 登录/隔离资格仍为 UNSUPPORTED/NOT_RUN。

实例页以宿主能力为准：创建具备实例登录资格的实例后直接开始其 login-only 流程，并提供显示名、改名、启停、使用中拒绝的删除、并发上限、检测及打开受控 home。CLI 区域区分操作状态、进度、失败原话、当前版本、已验证新版本、未验证的官方新版本提示、可退回的前一已验证版本和上次检查时间；实例区域区分上次确认的遮挡账号/订阅与 OpenCode 认证来源、当前登录状态检测错误、最近记录、所服务席位与会话数、所选模型与另有来源证据的可用模型、过期、CLI 明确报告的额度恢复时间、遮挡的错账号未采用及异常收尾记录；未验证版本提示不能直接变成可安装版本。检测动作只读该 CLI 已证实的 login-status 路径，不调用泛化 doctor 或模型/额度请求；doctor 是单独的显式操作，没有合格登录状态路径就隐藏检测；来源没有给出或不能可靠归属实例时显示未知，不能用模型或额度请求补测。没有宿主能力的操作不展示；界面不展示内部 ID、路径、哈希和修订号。真实登录仍只由 Owner 在可见授权页面完成，宿主负责启动和结果回读。

右侧 Git、文件、提示词旁新增 lucide users 席位图标，选中渲染 `<SeatsPanel source={...}/>`。source 只读当前项目；真实桥未接或 `source.read` 返回 null 时显示“席位数据还没接上”，不能用 fake 行代替。已接桥时展示在跑席位数、本机并发上限；每席位的名字、直属下属、主控标记、长短期、IDLE/WORKING/SWITCHING/STOP_REQUESTED/STUCK/REMOVING/REMOVED、当前与换绑前实例名字/厂商、模型、effort、权限、一句话 doing、卡住时长、最近活动、状态卡 goal/pending/reclaim 条件与最近动作。候选实例只列已登录且启用者，模型绑定实例，effort、权限档位和模板取宿主能力；主控范围展示可用实例、最高权限及席位上限。创建直属下属、tune、忙时拒绝换实例、拒绝回收主控、忙席位先获真实 H stop 并读回 StopFact 后保留记录地回收、设置编排范围分别走原 K-SEAT 操作。界面不展示会话号、PID 等内部编号。现有 PanelTabs/Git 控制器测试若因第四项确实失败，由汇合席位按真实失败决定适配；本修订不新增测试路径。

这次修订改变计划范围摘要，因此修订回执与计划在同一个 PR 中，由 Owner 依既有 v2 流程合并一次；计划和结构自检不代表产品实现、Win11 验证或候选验收。

## M2 原厂历史目录的实现细节（2026-10-04）

同一 OS 用户之内，R-CLEAN 仍要求投射产品的项目、席位及用途边界。给每个模型进程开放整个实例 HOME 不能证明这项要求。共享 HOME 的目录 ACL 候选在固定 Codex 第二个原生会话启动时返回访问拒绝；该具体候选未获资格，不能推广为所有目录 ACL 方案均不可行。

Codex/File 后端的下一候选采用实例持久根内的私有原厂历史子目录。F 持有目录与认证对象元数据，H 从原始原生回执、STOPPED episode 和当前绑定确定 continuation，A 仍是用途的唯一权威。SESSION/CALL 目录继续是每代独立的临时目录；同一可信原生 continuation 跨代恢复历史目录，全新原生会话创建空目录。formal 的零继承及 continuation 拒绝保持不变。旧历史不得扫描内容猜归属、归给当前席位或静默丢弃；具名恢复需求必须另有可信原始身份及物理对象证明。

认证复用只限原实例已确认的 File 后端及同一认证物理对象：宿主可持有仅元数据的句柄，登记确切 file ID、受控名字及 link count，建立同卷原生别名；不读取、解析、复制、映射、哈希或记录凭据内容，不请求 READ_DATA/WRITE_DATA。需要的 DELETE 命名空间权限必须明确记录，活跃期不删除认证对象。原厂 CLI 自己读取及原位保存认证内容。普通文件仍要求单链接；只有登记对象及完整受控别名集合可走精确例外，任何未知链接、替换、重解析或不确定结果都拒绝并保留现场。模型不取得 DELETE、WRITE_DAC、WRITE_OWNER 或父目录 DELETE_CHILD 来删除共享认证对象。

同一认证对象使用共享元数据 custody；停止一代撤销该代权限，不能在其他活跃代仍持有对象时强行删除别名。实例已确认静止、无未知 intent 时才结算别名清理与登录重绑定，不能靠等待、重试、自动停止其他席位或降低 Owner 并发上限补偿。重启先恢复原意图、episode 和登记对象身份，再允许启动。普通用户登录、模型 LPAC、lpacIdentityServices、固定 CLI、权限档位及授权范围不变。

V04 的“不打开凭据文件”明确为“不为内容访问打开”，仍禁止宿主读取或复制任何凭据字节；只说明完成身份与命名空间 custody 所需的元数据权限，不授予内容权限。V04b/V10 要在候选本体上分别验证同实例两项目、worker/Owner-lead、formal、同 continuation 跨代恢复及重启；云端合成认证对象只证明机制，真实登录复用与原厂 refresh 未执行不得写成通过。其他厂商按其固定版本的实际配置独立验证，不套用 Codex 的文件布局。

参照：既有 F 注册根与临时目录身份、H episode/RPC journal、A session purpose、compat_module 的 held-root custody，以及固定 Codex `a956835d020762cb2b570053af06f643a11c0ecc` 的 FileAuthStorage 原位保存方式；不是重新登录、凭据副本或替换 CLI。以上仅细化 K-SESSION/K-INSTANCE 和 V04/V04b/V10；不改需求、持久写方、wire family、共享文件或写入范围。必要的同 DB 产品入口接线由 Controller 持有，沿用已撤回多余 Owner 触点的持久组合裁定。

## 旧隔离权限的系统重启后恢复细节（2026-10-04）

真实 Win11 候选的首次模型启动被 `legacy account scope requires original stopped observer custody` 拒绝：旧登录 custody 是 UNKNOWN、原停止证明为空，实例根仍有旧可继承 AppContainer 授权。新根的云端通过不能作为旧根迁移证明。恢复必须保留原 UNKNOWN、原停止证明、原实例 home 和凭据字节；不以进程缺席、时间差、旧版本或名称过滤推造 STOPPED，不新增 capability，不修改 CLI，不将系统重启事实用于旧会话 continuation、worktree 删除或 admission 释放。

F/H 在原注册根的既有排他协调和元数据 custody 下，沿用已持久提交、不可扩充的旧权限迁移栅栏及原 schema。第一次遇到这项旧权限阻断时，登记原物理数据库、注册根、实例 home、已登记认证物理对象及旧 custody 的确切 ticket、nonce、每个 PID 与 `creation_time_100ns`、绑定和原始状态摘要。只登记当时确切存在的旧对象，不请求凭据内容权限或修改 ACL。栅栏提交前后原元数据和旧记录必须一致；已有活跃 holder、新 intent 或新的 UNKNOWN 继续拒绝。栅栏所列的旧 UNKNOWN 自身是待保留的遗留对象，不被抹去。原栅栏一旦提交就只读回、不扩充、不重写 capture 字段或 `boot_identity`；旧 `boot_identity` 若是 BCD GUID，仅作原始记录保留，不作恢复资格。新栅栏或步骤若需记录启动值，只以 `boot-start:<16hexFILETIME>` 命名空间标记元数据，不凭字符串变化授权。

已发生真实 Windows 系统重启后，在原注册根排他协调内逐项证明旧持有者消失：(a) 对原栅栏的每个 custody PID，按原 `creation_time_100ns` 精确核对原进程均不存活；(b) 当前内核启动 FILETIME 严格晚于栅栏中每个旧进程的创建 FILETIME。两项都满足，且原 root/home/认证物理对象、栅栏记录与旧 ACL 来源原样匹配，所有未列或后来新增的 holder/intent 均按既有规则结算，才允许迁移栅栏所列旧身份的精确权限。每个步骤的 begin、prewrite、finish 均在既有 revision CAS 与两步骤权限 journal 下，核对封存的原 PID/创建时间对仍与原栅栏 custody 完全匹配，重读启动 FILETIME 为同一值并重查原持有者；任一条件漂移即拒绝，不改原栅栏。启动 FILETIME 是直接读取的启动事实，不是用当前时间减运行时长推断。该证明只为旧 ACL 迁移和下述 source baseline 准备授权；它不证明旧业务调用成功或产生原 Job/writer 停止证明。原 custody 的 UNKNOWN、NULL proof 和历史记录保持不变，旧 continuation 仍按原 STOPPED 契约拒绝。原有历史与工作树不删除，COMMITTED 预留不因此释放。迁移只清理已证明属于被列旧身份的精确授权，保留 Owner/系统以及仍有原始有效 custody 的权限；同一对象的不同用途不获得新的读取权限。

权限更新逐个记录意图和结果；崩溃后用原栅栏、原物理对象和实际 ACL 元数据对账，全部精确完成才记为已迁移。未完成时不启动模型、不删除认证名字或内容，不用等待、重试、宽限或全树 ACL 重置补偿。新 model/account-observer 的权限仍由原启动路径授予。没有栅栏、任一旧 PID/创建时间无法鉴定或仍存活、启动 FILETIME 查询失败或不严格晚于每个旧创建时间、根或对象替换、记录变化、未列 UNKNOWN、未知链接、重解析或当前活跃 holder 均拒绝并保留原始错误。恢复不能借旧栅栏让后来新增的记录失效。本次使用已经发生的真实重启；不要求 Owner 再重启或登录，产品也不执行系统重启命令。

原认证对象若是未保护的 DACL，仅清理 HOME 的旧 grant 还不足以建立现有模型启动需要的 source baseline。栅栏因此分开登记两个确切步骤：旧身份 grant 迁移，以及原认证物理对象的 baseline DACL 准备。后者只在上述逐 PID 消失与启动 FILETIME 两项证明均成立、原对象不变、完整旧 holder/权限集合有来源绑定、没有未列或当前 holder/intent、没有别名且 source link count 为 1 时，采用既有 `prepare_quiescent_owner_account_source` 的 Owner/SYSTEM/Administrators 精确保护基线；任何未登记 AppContainer 权限都拒绝，不盲目重置。它仍只请求元数据 READ_CONTROL/WRITE_DAC，不请求内容权限；意图、实际 protected DACL 和结果逐项对账后才能记录本次 holder 的 prepared witness。此独立资格只送入这两个恢复 ACL 步骤，不把整个实例标为 quiescent，不改一般 quiescence 检查，也不用于启动之后的活跃认证对象重置、登录重绑定、别名清理、STOPPED、continuation 或预留释放。未保护基线、旧进程仍存活或启动 FILETIME 未晚于旧创建时间、只完成 HOME 步骤以及 baseline 部分失败必须作为独立的拒绝/崩溃回读用例；安全集合仍需实际 Win11 证明。

`SystemBootEnvironmentInformation.BootIdentifier` 是 BCD 条目标识，重启不变是预期行为，不能再当作重启证明。启动 FILETIME 优先由受信系统 ntdll 的 `NtQuerySystemInformation(SystemTimeOfDayInformation, class 3)` 读取 `BootTime`，参照 [System Informer 的既有调用](https://github.com/winsiderss/systeminformer/blob/master/phlib/include/phnativeinl.h) 和 [固定 phnt 结构定义](https://github.com/winsiderss/systeminformer/blob/ef43345b48301ada8f01b7ce194ddbf45b15bc28/phnt/include/ntexapi.h)。该信息类的成员为 Windows 私有实现，[Microsoft 文档](https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntquerysysteminformation) 不承诺稳定布局；动态解析、核实状态码、返回长度、结构及正值，失败保留 NTSTATUS 并拒绝，不用 WMI、elapsed 或调用者传值代替。宿主的 `NativeLegacyHoldersGone::observe(exact[(pid, creation)])` 对每个旧 PID 用 `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE)` 取得并保持句柄，`GetProcessTimes` 精确比对原创建 FILETIME，再以 `WaitForSingleObject(handle, 0)` 判断该原进程是否已结束；PID 已不存在或已复用且创建时间不同，也只在内核查询明确证明后判为原进程消失，AccessDenied、查询失败、`WAIT_FAILED` 或不确定状态一律拒绝，保留 Win32 原始错误。云端 Windows 验证真实 API 返回、重复读取和权限/失败路径，再覆盖逐 PID 同一活进程拒绝、PID 复用、缺失及错误、启动 FILETIME 严格比较、UNKNOWN/NULL proof 与旧 continuation 拒绝、未列/变化/新 intent/活跃 holder 拒绝和部分迁移崩溃对账。`for_test` 仅覆盖事务边界，云端合成旧进程与启动值只验证机制，不证明实机 boot 语义；已发生重启的 Owner Win11 本体须一次核对真实旧进程消失、内核启动 FILETIME、实际 ACL、原实例模型会话及正式版保护快照，没跑不算通过。

参照：先查 gogo-party 的 accounts.ts、席位运行时，以及 NaveHQ、LoomOS 的 boot/fence 实现，未发现可照搬的旧持有者迁移；再查仓库 `process/windows.rs` 的 `GetProcessTimes`/`WaitForSingleObject`、`process_custody.rs` 的 PID 与创建时间记录，以及 parts/substrate teardown、execution-layer-capability-table、kernel-parts-harvest、reuse-blueprint、upstream-reference-map，沿用“原 custody、单写方、未知保留、按物理对象迁移”的规则。最后核对上述 Microsoft 文档与 System Informer 的直接 BootTime 调用。不采用 BCD GUID、lease、等待时长或 elapsed 推断。本次只修订一轮实现细节；独立审计须复核真实来源、恢复前提、云端与实机证据边界。若仍无法干净恢复，停止此恢复路径并保全现场，先报告，再把新增实例登录并入 OT4b 同一次 Owner 触点；不把 fallback 自动视作本轮通过。不替换 ProcessCustodian 停止事实，不改变范围摘要、权限档位、持久写方、写入范围或既有 Owner 决定。


## F.2 席位工作树提交封装（2026-10-05）

M2 的“隔开写和合并”由宿主封住 Git 边界：席位仅在授权的宿主工作树内修改文件，不需要模型 CLI 执行 Git 命令或访问 `.git`；不向 H 的 LPAC profile 增加 Git 可执行文件、PATH 或 common Git 目录权限。E.2 当前调用者仍须授权既有 `K-WORKTREE merge`；F 仅在原 F worktree/repository 物理绑定有效、原 H StopFact 已确认且没有占用中的 admission reservation 后，使用 Owner 注册的固定 Git pin，将停止后的 child 改动封装为本地提交，再执行原 source merge。提交保留 project、seat、instance 和授权该次执行的 native turn 身份；source clean 约束、原 incoming tree 重解析与属性/外部 driver 拒绝、正式数据保护、固定 CLI、凭据不读均保持。

F 在原 MERGE intent 下先持久化与原请求绑定的 child-seal intent；提交成功后记录 child commit 原 hash。child commit 已发生但后续 merge 或数据库回执无法证明时，原请求进入 `MERGE_UNKNOWN`，原请求和新请求都不得重做该效果。历史 `APPLIED` merge receipt 继续按原记录只读回读，不补造 child receipt。这里补的是 F.2/K-WORKTREE merge 的描述性细节：wire family、写入范围、阶段、Owner 触点和持久写方归属不变。

参照：gogo-party 的 `packages/room/src/server.ts:411-425,1848` 由 room 的 `commitForSeat` 代表席位提交，席位只改文件、不触碰 `.git`；当前 Rust F2 已持有固定 Git pin 并负责 merge commit，但还没有 child 改动的宿主封装步骤。

## 同次开机内的原持有者消失与资源恢复（2026-10-05）

Owner 已裁定：正常退出或崩溃丢失原 custodian 后，产品应在同一已登录实例自行恢复，不要求重新登录。本路径是 F/H 资源结算，独立于前述 PR #66 旧权限迁移的系统重启资格；原 `NativeLegacyHoldersGone` 的启动时间条件、旧栅栏和旧 UNKNOWN 集合保持原样。新 native-only 证明复用同一 `OpenProcess`、创建 FILETIME 和已结束句柄检查，确认捕获集合中每个确切 PID/创建时间身份已消失；PID 复用须核对原创建时间，AccessDenied、查询失败、不确定或任一原身份仍活均拒绝并保留原始错误，不用等待时长推断。

父进程消失不能单独授权恢复。当前 verified DB 必须仍绑定原物理 root，并在同根排他协调下确认原 custodian 不属于当前 live runtime。原 H 的唯一启动/ACTIVE 写方在受检 suspended launch 中完成非继承 Job 绑定、无 breakaway、kill-on-close；原 `ProductDatabase` 在关闭 DB、释放 RootLock 前丢弃全部 custodian/Job。由同一物理根重新取得排他 custody 和原封存 operation/ticket/nonce、episode/F generation 组合原持有者已退出的资格。它是代码与原实际安装来源绑定后的结构依据，不是旧记录中不存在的逐 Job 实测字段，也不是调用者传入的 `hostGone`。云端须实测父进程先退而后代仍活、原宿主关闭后同根重开时后代不再活；撤销 kill-on-close 的生产变异必须使断言失败。同权限 Owner 主动保留句柄不被当作跨主体攻击阻断此路径。

F 先持久保存不可扩充的独立 holder-disappearance capture 和 revision CAS：原物理 DB/root/home/认证 FileID、完整受控别名及 link count、原 profile/history/SID、H claim/episode/custody 绑定、状态与修订号，以及 ACL 原值和精确目标。该记录不能构造 StopFact。后续只撤销列明的原 SID 精确 ACE，保留 Owner、系统和其他合法 SID；不重置整个 DACL、不删除别名或历史、不读写凭据内容。每个 begin、prewrite、finish 都重验原捕获、内核消失事实及物理对象；未知 holder/intent、对象替换、未知 link/SID、重解析或原值漂移均拒绝。崩溃后的同一原 intent 只允许两种确定分支：实际 ACL 为封存原值时执行尚未完成的原精确步骤；实际 ACL 为目标时仅读回完成。其他状态保留 UNKNOWN，不用新请求重做副作用。第二条 SID 的原值必须匹配第一条精确步骤完成后的值。

实际撤权回执与独立恢复记录齐全后，由 H 在同一事务核当前 Owner、原 claim 修订号/绑定/seat incarnation、无未决 generation change，将确切原资源占用结算为 RELEASED；只在该席位 incarnation 的所有占用均已释放后才置为 IDLE，并写明独立 holder-gone 依据。多成员逐项结算期间只要还存在一个未释放占用，席位继续 BUSY；不经过 STOPPED、不填写停止 hash。原 custody、episode、RPC 和业务历史保留，新宿主既有 ACTIVE/PREPARED→UNKNOWN 初始化也不得冒充停止事实。F 的冷 source holder 仅对精确 APPLIED 恢复集合核实既有 protected DACL 残余并取得只读 adoption witness，不修改通用 `stopped_instance`；后来新 holder/intent 仍按原规则拒绝。旧 continuation、worktree merge/delete 和任何需要原 StopFact 的操作仍拒绝。三个更早的 UNKNOWN 和原 legacy fences 不在此恢复集合中。

在 Owner Win11 本体用原实例及原认证物理对象验证两条恢复、原并发占用释放和新真实模型会话；保留原失败、旧三条 UNKNOWN、认证元数据及正式版五组保护快照。每次关闭候选前必须先对本次所有 live 会话执行原 H stop，并读回确切 StopFact，再正常关闭；测量失败先保全原请求，不能仅关闭窗口后把进程缺席记为停止。本次限定一轮恢复施工和实测；若仍不能干净恢复，保全现场、报告后才进入 Owner 允许的一次新 Codex 登录。

参照：gogo-party 席位 close 的 PID/创建时间约束与 P07 supervisor 实验（仅行为参考，不作为本体证明）、仓库 runtime/parts 调研、PR #66 的既有内核查询、原 H Job/RootLock 生命周期、F metadata/journal/精确撤权及 H admission CAS。采用这些既有做法，不另建探测框架；新通道仅区分“持有者消失后的资源释放”与“真实停止事实”。本节只修订 F/H 已授权实现细节，不改范围摘要、wire family、持久写方、权限档位、固定 CLI 或 Owner 触点。

Owner 逐字段接入补充：新建输入为厂商、显示名及 OpenCode 所连模型来源；模型来源与凭据后端/认证来源分别记录。实例读回上次成功确认登录/账号的时间；异常退出仅在宿主持有真实自动收尾结果及依据时显示“已自动收尾”，未结算保持 UNKNOWN，并提供最近记录。CLI 的安装失败与已安装但启动被拦分为不同状态，分别保留原话；安装/升级进度与各自失败也分别读回。V13 对这些输入、时间、收尾结果和状态分别验证，不从泛化异常记录推断成功。

席位的“直属/下属”是 USER/LEAD 两层的显示归属，新建直属席位按用户层原授权执行；并非让主控越界新建 USER 席位。内部席位/实例标识只可作宿主动作目标，不渲染给用户。计划 PR 的 Browser job 在原 main 夹具读取终态回执后复用目录，原卸载子进程仍持有生命周期锁而返回 Windows 32；同 PR 携入施工分支已有的确切锁释放观测及非空持有负例，只改云端夹具，不改产品删除规则，明确列为集成席位共享文件。

## Owner 2026-10-07：正在进行的最小挂接范围（待本 PR 合入生效）

G 新增 `apps/desktop/src/features/now/`，沿用 Owner 已认可的样稿与 Claude `bce2990b`。现有 MainApp worktree 说明没有 Messages 历史锚点、ThreadRow 状态字或 ComposerInput 停止控件插槽；外层包装无法保证旧批次留在原轮次，且原组件仍自行渲染状态与停止键。只新增以下三个 INTEGRATOR 共享文件：

- `apps/desktop/src/features/messages/components/Messages.tsx`：按真实主控输出锚点挂 NowBlock，永远是该输出的最后一块，已 closed 批次保留历史原位。
- `apps/desktop/src/features/app/components/ThreadRow.tsx`：从 G context 读确切项目/对话的 batchWord，沿用 thread-state-chip 的 unread / processing / ready。
- `apps/desktop/src/features/composer/components/ComposerInput.tsx`：有宿主真实操作时换为 StopMenu，区分停这一轮与停这件事，以及停止请求与停止确认。

G 在自己的新目录提供 context，已授权 MainApp 包裹其现有页面节点并在 composerNode 上方挂 NowPin；因此不新增 Composer、Sidebar、ConversationItem 类型、消息分组 hooks 或 layout builder 的写范围。宿主没有的数据不猜，操作做不到就不传；断线重连完整读回，派活无回执为 unknown，审计之后被审文件变更才是 stale，删除席位为 gone。批次事实来自原主控轮次/派发记录，closed 与改动合入来自宿主原回执；拒绝、超时、出错、投递失败保留各自原因。

参照：现有 Messages 末尾与历史分组、ThreadRow 状态字、ComposerInput 原停止键，MainApp/shared_files 的既有节点与插槽；采用小型 G context 传递，避免为穿透 Sidebar/Composer 扩大文件清单。Owner 样稿：<https://claude.ai/artifact/KT15K79wwMhLeWQ2PzMBMk>。本次未能通过工具读出样稿内容，不据此宣称画面一致；以 Owner 认可和分支实际代码为审阅依据。

这是范围摘要变更，同一个 PR 更新 v2 回执及签名链三处固定回执 blob，Owner 合并一次后精确读回才可改这三个产品文件。不改变既有需求、权限、契约持久写方、里程碑或 Owner 触点，不构建候选、不签名、不装机、不自行验收。

G context 必须覆盖左栏所有可见 pinned/recent/对话行的确切项目/对话查找，不只 active 行；legacy workspace/thread 标识不能直接当作 native 身份。每批另由宿主提供 native 主控轮次与具体输出 item 的确切关联，现有 NowBatch.id 与 ConversationItem.id 都不自行充当 turn 锚点。身份或锚点缺失明确未接上，不按顺序或最后一条消息猜位置；该关联类型仍在 G 新目录，Messages 消费现有 item id，故不增加 types.ts 范围。

## 2026-10-07：可见对话与 H 共用真实会话的最小范围修订（提案，待 Owner 合并）

直接读回发现：现有可见对话经 `backend/app_server.rs` 自行启动 CLI，使用 workspace 配置的 home；设计 37 的 H 另持 native session、实例 home、process ticket 和原始协议来源。两者是不同的进程和传输，编号相同不能证明同一身份。PR #73 的三个 UI 消费文件已经挂接；缺少上述同一性时明确未接上，预览通过不能结算本体。

新增六个 INTEGRATOR 共享文件，产品代码只在本 PR 由 Owner 合并并精确读回后施工：

- `apps/desktop/src-tauri/src/backend/app_server.rs`：设计 37 对话使用同一个 H 会话及其原始事件，保存宿主身份、原主控轮次与输出 item 的确切关联；会话存活和停止通过 transport 的统一方法执行。既有非设计 37 路径保留原行为。
- `apps/desktop/src-tauri/src/codex/mod.rs`：现有启动、恢复、读取、列表、发送、插话、停止、审批和输入响应接到同一会话；沿用原 Tauri 命令和事件形状，不把 legacy ID 当 native 身份。
- `apps/desktop/src-tauri/src/shared/workspaces_core/connect.rs`：连接、取消连接时调用上述存活/停止方法。
- `apps/desktop/src-tauri/src/shared/workspaces_core/crud_persistence.rs`：原创建/克隆失败回滚时调用上述确切停止方法。
- `apps/desktop/src-tauri/src/shared/workspaces_core/runtime_codex_args.rs`：原运行参数调整前调用上述确切停止方法。
- `apps/desktop/src-tauri/src/bin/gogoke_daemon.rs`：原过期清理、回收和关闭调用上述生命周期方法；不支持的 native 模式在启动前明确拒绝，不退到普通用户模型进程。

后四个文件是现有 `WorkspaceSession.child` 的直接生产消费者，改 transport 后必须一并接上，否则关闭与回滚仍会停止错误对象。沿用同一个 session registry，避免另建 map 扩大 `state.rs`；原始协议和事件形状不变，不新增前端 hooks、ConversationItem 类型或任意文件范围。实际发现需要其他文件时，保全未接上状态，重新列出确切依赖，不能借本节扩大施工范围。

原 H 是唯一启动、准入、隔离、权限和停止事实写方；同一固定 CLI、实例 home、LPAC 与 `lpacIdentityServices` 保持不变。模型不在普通用户进程执行。停止请求与确认分开，原始失败保留，不能从 `Child` 缺席补造 STOPPED。登录普通用户阶段沿用 Owner 已裁决的范围。本修订不改需求、契约持久写方、权限、里程碑、验收或 Owner 触点。

验证针对实际可见对话：同一个 H session 的原始输出到达 Messages，历史批次留在原输出 item，左栏每行读到其确切身份；发送、插话、恢复和审批仍作用于该会话。连接取消、创建失败、参数修改、过期回收与关闭均走确切 H stop/readback；断线完整读回，身份或锚点缺失拒绝关联。稳定候选在 Owner Win11 的已登录测试实例上跑真实端到端；K-UI 预览只验证 UI 消费语义。

参照：直接读取现有 `WorkspaceSession`、四处 `Child` 消费者、`codex/mod.rs` 与 `product_entry.rs` 的通用 USER 桥、原 H/A source 和 Claude 已复核的 Now context。采用已有 session registry、原始协议与宿主准入，未另造编号映射或第二条模型通道。T00 列出 main 与既有映射快照的全部共享文件 blob；既有卸载夹具的差异由已合入修订承接，其余不借机修改。

范围摘要因此改变：同一 PR 更新 v2 回执、MANIFEST 和三处固定回执 blob。Owner 合并一次后，主控精确读回并用 main 的 verifier 重算摘要，再接这六个产品文件；本提案不构建候选、不签名、不装机，不宣称完成或接受。

## 2026-10-07：秘书长界面与宿主（同一范围提案，待 Owner 合并）

依据 main `33a7a9e5` 的《GOGO 要做的和不做的》§八、设计 37 §7/§8c 和 Owner 本次指令，秘书长在 M3 的 E.3/G.1 结算；不把新增秘书长交付倒排为 M1/M2 的收尾条件。沿用 Claude `claude/g37-secretary` 的 `9aa6e76a`，基于已复核的 `b94a4ebd`，尚不集成未授权目录。现有主控输出同一性缺口与本次秘书长共用 PR #74，一次 Owner 合并更新回执。

### 最小写入范围与挂接

- G 新增一个目录：`apps/desktop/src/features/secretary/`，含 Claude 已提供的组件、类型及后续 K-UI 适配，不另建一套对话渲染器。
- INTEGRATOR 只再新增一个共享文件：`apps/desktop/src/features/app/components/Sidebar.tsx`。给固定 `SecretaryEntry` 留一个入口槽，置于全部项目、置顶和最近对话行之前，位于这些行的滚动区之外。现有 Sidebar props 由组件类型推导，不新增通用 types/hooks。
- 已授权 `MainApp.tsx` 提供入口、真实全局会话选择和返回原项目；打开秘书长沿用普通 Messages/Composer，秘书长自有的一条长期对话可搜索，项目选择与历史不得被改写。右侧用 `SecretaryPanel` 替换项目面板，关掉项目 plan 分区，只保留定时任务和设置两个标签；不改 DesktopLayout。
- 已授权 `Messages.tsx` 在秘书长真实输出中挂 `SecretaryActionLine`，每项动作按原事件/输出 item 的确切关联留在历史；原文、结果、原因和导航由宿主提供。G context 只传类型与事实，长对话和动作来源不塞进 panel-only 的假字段，不借显示名或 legacy ID 伪造 native 绑定。缺事实明确未接上，做不到的操作不传。

### 宿主分工与边界

- **E.3**：一个用户层配置的全局席位，实例、模型、强度、权限档位逐项读回，有明确“没设置”；只用已登录实例的已验证模型和宿主能保证的档位。全局身份来自 H 核实的席位目的与绑定，不能由模型传一个 global/secretary 字段取得。复用既有实例、准入、并发、LPAC、真实停止和恢复，不新增登录触点。
- **A**：秘书长自己的全局账本、记忆和决策登记册，与项目结构隔离。订阅所有项目所有席位的原账本，按 epoch/cursor 补齐断线，再完整读回；项目和旁聊查询不能取到全局或别的项目。全局决定保存 Owner 原话、独立解释、日期、作用域、账本出处与取代关系；冲突或不确定先标记，不覆盖旧决定、不冒充已定，界面不另设登记册页。
- **C/E/H**：按调用权限表给项目主控递任务、追进度，向审计要结论，或为 Owner 在项目开新工作。每个投递在落到项目之前按目标作用域过滤，经过原 H prepare/begin/completion 与当前授权复核；动作行保留实际原文和 `steered / new-turn / failed / unknown`，失败保留原话。未知不算成功，不因缺回执再发一次。新工作不等于新席位；秘书长不直接改项目文件、不建席位，配置只能在这些已定边界内选择接收者和调用范围。项目问题留在项目，只在秘书长对话给指向，不替换成新的全局问题。
- **E.3/现有 coordinator**：自然语言经已准入秘书长的工具创建定时任务，暂停、恢复、删除保留历史，持久记录原时间规则/时区、下次运行、上次结果及原因。现有 coordinator 只有恢复协调，尚没有完成秘书长定时任务；E.3 在它和 E 原有持久域内补齐，不声称现成 API 已存在、不另起调度服务。用唯一 occurrence 与原 H 送达日志去重，重启不能重复执行未知副作用。暂停针对后续投递，不能充当停止确认。
- **离开事实**：最近一次可归属的用户前台打开/输入/交互、观测时间和持久的自动暂停策略共同决定离开多久；模型输出、轮询和定时任务活动不能刷新 Owner 在场。策略使用明示的产品默认或用户配置，默认值在实现前按现成做法核定并记录理由，不新增单独 Owner 裁决。未知策略、时钟或在场事实不准新定时模型调用。每次运行前执行暂停条件并持久回读；自动暂停后不悄悄恢复。界面用实际事实描述离开，不能把“无交互”说成“没打开”。
- **运行范围**：产品/宿主活着时运行，并在重启时对账；不新增 Windows 后台任务、系统服务或停机运行承诺，不把停机错过的任务集中补成模型调用。入口的安静、几件等你、正在做什么、没设置、用不了及原始原因/真实恢复时间都由宿主事实给出，不从对话猜。

K-UI 按 `secretaryModel.ts` 的 `SecretaryPage / SecretarySettings / Routine / EntryState / ActionLine` 与 `Secretary.tsx` 的 `SecretarySource / SecretaryActions` 适配；确切全局长对话、动作历史及宿主导航关联由薄 G context 承接，UI 类型不另立持久写方。E.3 依赖 A.1/C.1/E.2/F.2/H.1，G.1 依赖 E.3；原约定的 A/C/E/F/H 持久写方不变。`V14` 在 M3 验完整长期对话、权限拒绝、全球/项目隔离、双向原文与四种送达、定时重启去重和缺事实拒绝、暂停/恢复/删除、用户不在事实、入口/两标签/动作行及不弹窗；预览不能顶替已装候选本体。

参照：直接读 Claude `Secretary.tsx/secretaryModel.ts`、现有 Sidebar/opaque sidebarNode、MainApp 两个右栏节点、普通 Messages/Composer、native A 的 global 查询和原 H/C 调用链；历史 gogo-party `room/src/role-policy.ts` 的角色界定和只读默认只作为参考，沿用新 Owner 决定下可写信箱而不写项目文件的范围。仓库 `kernel-parts-harvest` 的 Hermes Profile/Bot 长期身份、Curator，以及 `reuse-blueprint`/`upstream-reference-map` 仅借做法；Hermes Studio 的独立许可边界不当作 Hermes Agent 的授权，也不引入任一 runtime。Owner 样稿链接无法通过工具读取，依据 Owner 认可和确切分支，不宣称画面比对通过。

这次除 G 目录和 Sidebar 入口槽外，不新增宿主目录或通用前端事件/type 写域。范围摘要、v2 回执、T00、MANIFEST 和三处固定回执 blob 在同一 PR 更新，Owner 合并后精确读回才能实施。

聚焦复核落实：旧“不承诺”里泛称决策登记册、长期记忆在第二批，现明确区分：秘书长隔离的全局记忆和全局决策册属于本次 M3，通用项目登记册、通用长期记忆自动化及额度仍不纳入。历史可搜要由 A 提供同一秘书长对话的原内容/账本位置搜索，不能拿现有线程标题搜索代替。投递必须核实际正文及来源材料：作用域标签本身不能证明任意模型文字没有混入别的项目；来源或目标资格不清楚就保留拒绝/unknown，不投递，V14 对实际材料做跨项目负例。

USER 是产品授权层，不是要求 Owner 手工操作每项设置。Controller 按已有测试授权，在隔离候选/测试协调域中使用已授权、已登录测试实例和既有已验证模型/强度、边界内最低可保证权限，自动走真实 USER 配置，验证没设置、已配置和被拒绝并保全/读回/恢复测试记录；不改 Owner 真实全局设置或凭据。真实使用的偏好仍由 Owner 在产品里设置。只有新增凭据、提权或已有授权确实未覆盖时走原条件触点，不为本次测试再新增 Owner 触点，也不把 M3 秘书长测试变成 OT3 的 M1 前置条件。


## 2026-10-08：普通对话原生接线的最小共享范围（提案，待 Owner 合并）

本次给 INTEGRATOR 增补三个 hooks 和一个既有原生 effect helper 的确切范围；G 的写入范围、契约持久写方、H/LPAC/CLI/实例边界及既有 Owner 决定保持不变：

- `apps/desktop/src/features/threads/hooks/useThreads.ts`：在新建/接入前采用宿主核实的原生路由；原有 runtime args 设置只留给 legacy。不能把已加载的 UI 快照当成原生关联重连已读完。
- `apps/desktop/src/features/threads/hooks/useThreadMessaging.ts`：原生发送不再先写 legacy runtime args，也不带宿主不接受的 legacy model/effort/access override；实例、模型、强度和权限采用原配置/准入事实，不能猜。legacy 发送不变。
- `apps/desktop/src/features/threads/hooks/useThreadActions.ts`：原生水合与重连走已有完整 `read_thread`；读取不借 `resume_thread` 改 H 会话，loaded shortcut 不能跳过原生全量对账；保留原错误。

- `apps/desktop/native-host/src/store/product_database/v37_visible_effect.rs`：只在首次 effect 准备前、原 immediate transaction 内核 workspace 最新选择；不越过后来的选择去采用历史同 association。已有 requestId 的重放/恢复继续核原 action/selection row 和 H/A 回执，不受后来选择影响。该 helper 已存在于 shadow，但在映射快照及 main 尚不存在；T00 明确记录来源，不把历史施工提交当授权。

现有 MainApp/tauri.ts 插槽能读 USER choices、显式选择其精确关联、再接同一个 H session；它们不能消除这三个 hooks 内部的 preflight、override 与 loaded shortcut。本次不新增普通模型子进程、编号别名、第二套渲染器或事件/type 文件；helper 修改仅为已有 USER 写请求的当前选择约束。不能自动采用候选列表第一项，也不能从 legacy workspace/thread、composer 偏好或事件顺序推导 native 身份与 Now 输出锚点。

真实 USER 入口已优先分派到原 visible effect，finite 写操作不能按后面的只读分支误判为未实现；thread/start 因缺新会话身份仍为 UNSUPPORTED，因此本提案不是功能完成声明。真正新建按既有 H/USER 准入和原意图在已授权生产者范围内实施；如实际需要改变契约归属、安全边界或再加文件，保留未接上状态，另提最少修订。事件 seed 只建立原帧续读位置，不代替 UI 完整历史。

参照：直接读现有三 hooks、MainApp、`codex/mod.rs` 原 native history、`v37_visible_conversation.rs` 的 choices/select、`v37_visible_effect.rs` 的原 USER effect，以及当前 shared_files 清单。采用已有完整 hydration 和原 H/A 来源；增加无法在外层包装替代的三处消费者，并将已有 effect helper 的当前选择修正纳入同一最小范围；不为方便扩大 G 或通用 types/events 范围。

此修订改变范围摘要，同 PR 更新 v2 回执、MANIFEST 和三处固定回执 blob。Owner 合并一次、Controller 精确读回并用 main verifier 重算摘要后，才能修改这四个确切路径；不新增登录、重启、候选接受或公开发布触点。M2 既有授权范围内的云检继续推进。


## 2026-10-08：原生首次项目登记与 live-state 回读（范围提案，待 Owner 合并）

直接复核确认两个闭环缺口：`v37_visible_conversation.rs` 的 live-state 内层已有原 H 物理观察，外层却留为 UNKNOWN；Tauri 的 liveness 消费者因此拒绝原生重连。首次 `add_workspace` 在保存新编号前要求该编号已有 USER 选择，产生循环；普通视图只读确认候选尚无既存 workspace 行。不能靠修改候选数据文件、猜编号或采用 global CLI 绕开。

本修订只增一个确切写路径，并澄清四个既有共享路径的最少职责：

- `apps/desktop/native-host/src/store/product_database/v37_visible_conversation.rs`：仅在原 H 已证明 LIVE 或完整 STOPPED 且无原因时把读回外层置 APPLIED。UNKNOWN、原错误、关联校验、物理句柄和原 StopFact 要求保持。现行 main 没有该文件；只读依据为施工源 `8d0e6ea5cf34f1f25603577fae9e36f22c6687f8` 的 blob `d54a0636cc4cab671734d262aa6b2724c753d907`，历史实现和阅读参照不是授权。
- `apps/desktop/src-tauri/src/lib.rs`：在第一次 workspace 调用前建立既有原生 USER view context，不启动模型。
- `apps/desktop/src-tauri/src/codex/mod.rs`：仅启动已有受验证的 USER host，读取原新 workspace 的 NEEDS_SETUP；无原 association 时提供只登记路径。其他失败保留原拒绝，连接仍必须先有原 USER 精确选择。
- `apps/desktop/src-tauri/src/shared/workspaces_core/crud_persistence.rs`：仅现有 add_workspace 登记原新编号/元数据并返回 connected=false，不创建会话，不准入 H，不借 legacy 模型。原磁盘写错误保留，不声称原直接文件写入具有原子回滚。本次不扩展 native clone/import。
- `apps/desktop/src-tauri/src/bin/gogoke_daemon.rs`：仅补编译兼容的 false probe；没有 USER host 的 daemon 不借此声称 native 支持，原生命周期保持。

上述改变不增加 Tauri 命令、字段、通用类型/事件、能力、持久写方或 CLI/LPAC/账号权限。登记项目、选择已准入 H 的原身份、连接原会话分开；禁止自动采用第一项或从 workspace/thread/cwd 推导身份。受影响云端编译/检查和已安装普通 Win11 的实际首次登记、原 H live/stop 回读、普通对话与重启历史继续按既有端到端门槛验证；source review 和历史通过不是本体通过。没有新登录、重启、公开发布或验收步骤。

参照：gogo-party `packages/room/src/server.ts` 的 createProjectFromPath 先 openOrRegister 再切换/派活；采用“登记与启动分开”，保留 gogoke 已有 exact USER/H 边界。直接核了现行 native producer、Tauri liveness、workspace persistence、daemon stub、原 UI restore 和 startup。只修新项目登记，不因旁支 clone UI 的立即 connect 扩大界面写域。

本次范围摘要改变；同 PR 更新 v2 回执、MANIFEST 和三处固定 receipt blob。Owner 合并一次并精确读回前，以上增补未授权，不提交保全的产品补丁。原 PR77 四路径施工结果、失败原件和云检证据保留；M2 既有授权内工作继续。

## 中断后部分输出的只读冷恢复（同范围细节修订）

普通 Win11 原固定 CLI 直接证据：同一 agentMessage 已发 item/started 和有序文本 delta，却没有 item/completed；原 turn/completed 为 interrupted、itemsView=notLoaded、items=[]。固定 Codex 协议明示 notLoaded 的空数组不代表完整持久 item 清单。现有投影误把没有最终快照视为无法读回，冷启动又不会重放历史实时事件，导致中断前已经流出的文字无法恢复。

本修订只明确三个既有 INTEGRATOR 共享路径：v37_visible_conversation.rs 的原 A/H 只读投影、src-tauri/codex/mod.rs 的既有合页、useThreadActions.ts 的原完整 hydration。原 vendor Turn/item 字段保持；使用既有读投影元数据明确记录“中断时部分输出”，带原 thread/turn/item 身份、有序 sourceRefs 和原高水位。仅在完整来源覆盖、原 item/started、同 item 的有序 delta 和明确 interrupted 终态均成立且没有最终 item 时派生；有最终快照则优先，不重复显示。不得将派生文本称为最终 vendor item，不改 itemsView 为 full。缺帧、乱序、异身份、无可信终态或来源不明继续 UNKNOWN 并保留原文。

Tauri 不改变首次附着丢弃历史实时事件的规则；冷读合页保存上述元数据及来源，hook 在现有消息表示中清楚标为“中断时的部分输出”。不改 G、通用类型/事件，不新增命令、权限、模型进程、持久写方或 Owner 触点。原 H 物理句柄、StopFact、E/F/H/A 关联和 scope/seat 检查保持；未执行的真实工具/中断恢复不可写通过。

验证采用真实 CLI 黄金样本的结构与确切源顺序：原终态不改、partial 文本与原 delta 精确一致、逐项来源可核、冷读幂等且无 stdin/USER 写入；缺源、异 turn/item、非 interrupted 及有 final item 的反例保持原拒绝/优先规则。原生只在云端验证，稳定后候选完整链与已安装 Win11 同一真实中断后的冷恢复仍必须实测。单次 interrupt ACK、源审阅和旧候选历史通过都不等于整体验收。

参照：gogo-party packages/seat-runtime/src/seat-runtime.ts 的 assistant-delta/item-completed/turn-completed 分开记录；既有 A 原帧账本、native full thread/read、Tauri 合页和 hydration；固定 Codex commit a956835d020762cb2b570053af06f643a11c0ecc 的 codex-rs/app-server-protocol/src/protocol/v2/thread_data.rs 中 TurnItemsView 定义。采用明确区分部分输出与最终快照，保留 gogoke 更严格的原 H/A 来源，而不是复制实时事件去假装冷读原件。

共享路径、conditional、owner、各线写域及操作/权限契约均未增加，范围摘要和 v2 回执不变。按现行 amendment_rule，独立审计通过后由 Controller 合并细节 PR，再精确读回；不重新请求 Owner 合并或登录。实施前不得将这份修订草稿当作已生效计划。

## 2026-10-10：秘书长 E.3 到期接线的唯一共享路径（范围提案，待 Owner 合并）

只新增 `apps/desktop/native-host/src/store/product_database.rs` 为 INTEGRATOR 共享文件，限定在既有单一已验证数据库、Owner issuer 与原 H custody 上组合 E.3 到期判定、同事务原 H 请求 journal 和原 H receipt 结算。已列共享 `apps/desktop/native-host/src/main.rs` 仅在现有桌面宿主权威循环调用这一窄接线；E 的 `apps/desktop/native-host/src/store/seat/` 和 H 的 `apps/desktop/native-host/src/store/session_transport/` 各守原写域。不得给外部开放原数据库或 issuer，不另建 scheduler、后台服务、持久写方、权限档位、StopFact 或向项目文件写入的模型通道。未知送达不重发，原缺席事实或 H 资格不足继续拒绝。

实际 ProductDatabase 的 connection、issuer 和活动 H 会话均为私有；main.rs 属独立 binary crate，E/H 是 store 下的兄弟模块。现有 `store/mod.rs` 注册职责和 main.rs 的公开调用均不能单独完成同库事务组合。直接在既有 product_database.rs 增窄方法即可，不新增模块注册或其他共享路径。所依据的静态准备是私有 `g37-secretary-scheduler-preparation-20261010/scope-amendment.md`；它不构成产品证据或授权。

本次只有该共享路径进入范围摘要；需求、阶段、角色、E/H 写域、契约持久写方和 Owner 触点不变。参照 main `d91c9aa2` 已生效的 PR83 治理，仅将 `PLAN.json.sources` 中 `AGENTS.md` 与 `docs/governance/gogoke-build-and-release.md` 两个来源 blob 同步到当前 main；其余来源与映射保持，不新增治理要求。同 PR 更新 v2 授权回执、MANIFEST 和三处固定回执 blob。Owner 合并并精确读回新摘要前不得施工该路径；本提案不宣称定时任务已能运行、V14 通过或候选验收。

## 2026-10-10：项目 policy head 显式创建前只读判定（范围提案，待 Owner 合并）

真实 V10 隔离测试域缺原 policy head 时，原生 LEAD `create-from-template` 被拒；经已有 Owner `policy-initialize` 建立该域 head 后，原 LEAD 创建获得 APPLIED。这说明该次拒绝是测试前置缺失，不能据此判定模型创建逻辑有缺陷。静态产品源又表明普通 UI 的 USER 席位创建只调用原 create/bind/rename/tune，既有 `seats-page-read` 不返回 head、revision 或 stage；原生 `call-permission-table` 要求已存在 head 和活动 H 调用者，也不提供初次项目创建的缺行事实。读取不得偷偷初始化，更不能由模型创建子席位时补 head。

只新增 INTEGRATOR 共享路径 `apps/desktop/native-host/src/store/product_database/v37_seat.rs`：在既有 Owner 配置只读入口及同一已验证 ProductDatabase 快照中明确返回 head **absent** 或原 `{revision,currentStage}`，保留原错误，不写入、不自动修复、不授予 call grant、gate、route 或任何模型权限。G 已列共享 `apps/desktop/src/services/tauri.ts` 仅在显式新建项目或 USER 席位流程中消费该事实；确证 absent 才提交现有 Owner `policy-initialize`，既有 head 的 revision/stage、撤销和 grants 原样保留。原命令的 request bytes、精确重放及冲突规则保持；并发冲突只重读当前事实，不覆盖。初始 stage 的实际取值仍待按原项目创建语义确认；历史测试夹具使用 `OPEN` 不单独授予产品默认值。本提案不改变原政策表、持久写方、默认 grant、模型工具、契约或 Owner 触点。

只读定位依据为当前施工源 `2d700a5b` 的该文件 blob `633b3d29e918202700f2a1391ccca6eaf376c3a2`；它尚不存在于 `origin/main`，不是已生效实现或授权。现有 `seat/policy.rs` 只有 Owner initialize 写者、缺 head 的 native 读为 Denied；现有 UI transport 无自动 initialize。新增这个唯一共享文件是提供缺行与当前 stage 可信只读事实的最小生产接线。Owner 只审阅并合并本次范围 PR 一次；Controller 精确读回新 v2 回执、main 摘要及新路径后才可施工。当前测试 APPLIED 不证明普通 UI 项目 intake 已接通，也不代替云检、候选和真实安装验收。

## 2026-10-10：政策元数据与未设置流程阶段分开（Owner 已批准的同范围细节）

Owner 已认可最小方案，替换上一节“只沿原具名初始化、不改原政策表”的实现限制。设计的 USER 编排范围与用户定义阶段分开：原 native 子席位派活只需要 head revision，不需要预先定义阶段；不能把测试 OPEN 当产品默认。E 的 head 明确允许 currentStage 为 null，表示尚未设置流程阶段，不占用真实阶段名称。

仅显式认证 USER 项目/直属主控创建或设置编排范围时，可对确证 absent 的 head 初始化空政策元数据；grants、gates、routes 全为空。读取和模型调用不能初始化。后续 USER 设置初始阶段只允许对原 revision 且仍为 null 的 head 做 CAS；阶段缺值时关口和流转拒绝，具名阶段后的原关口规则保持。原验证后的 USER scope、层级、实例/模型/权限/并发上限和 H 物理调用证明仍限定第一张 native DISPATCH，不添加默认宽泛权限。

旧 schema 精确迁移，仅允许原已知 schema，保留旧阶段、revision、grants、撤销、事件和回执。空阶段 head 已存在时，旧 policy-initialize 仍按原冲突语义拒绝；旧成功请求精确重放只返回原事件回执，不能解释为首次设置阶段。任何不明 schema、CAS、提交或回滚保留原错误，不覆盖已有政策内容。

最少路径为 E 已授权的 store/seat/policy.rs、store/seat/mod.rs 和受影响安全测试；既有共享 product_database/v37_seat.rs 与 services/tauri.ts 接原 Owner 命令/读取；G 的 features/seats 仅按需表达“未设置”，仍由 Claude 施工。不扩展 v37_policy.rs、model wire、H、默认权限、持久写方或 Owner 触点。通知 K-POLICY 的 C/D/G 接缝；原 v2 回执和 scope digest 保持。

受影响安全检查覆盖空阶段无默认授权、关口/流转拒绝、模型不能初始化、旧原具名初始化/精确重放/撤销、CAS 和原 schema 保全。真机用已有登录测试实例的新隔离项目，未定义阶段时按明确 scope 建直属 LEAD，再由明确 USER 设置阶段并核原关口，正常 stop/release 与正式保护。云检、源码和静态复核不能代替这些实际结果。

参照：原设计 USER 编排与用户定义阶段、E 原 Owner policy/head revision/关口、原 USER ingress、已有精确 schema 迁移和 main 范围摘要实现。采用原空授权、CAS 与请求指纹；只解除当前 head 非空阶段约束造成的数据模型耦合。独立只读复核已核提案前提；本细节 PR 仍需独立复核后由 Controller 合并、精确读回，不新起授权回执。


## 2026-10-11：H 宿主退出诊断尾的唯一共享路径（范围提案，待 Owner 合并）

实际 Design 37 host 第二轮运行在 host 退出后进入既有 `HOST_EXITED` 错误路径；授权中的 `product_entry.rs` 已承载原错误转出，但 `public_runtime/design37_host.rs` 是施工提交 `fcc19b54` 引入、当前计划未列出的共享文件。现有文件持有原 `Child`，并已有有界 native stderr 尾缓存。

只新增 `apps/desktop/src-tauri/src/public_runtime/design37_host.rs` 为 INTEGRATOR 共享路径，且仅供 H 在现有 `HOST_EXITED` 错误中保留该有界 stderr 尾和原 `Child` 退出结果。沿用既有启动、进程句柄、错误路径和已授权的 `product_entry.rs`；不增加进程、权限、持久 writer、重放或 StopFact，不改变 Stop/Job/释放语义，也不扩展其他文件写域。

本次只增加这一条 shared path；需求、阶段、H 写域、契约持久写方、不变量和 Owner 触点不变。参照 main `3609d91f` 上 PR #85 的单共享路径范围修订：同 PR 更新 v2 回执、MANIFEST 和三处固定回执 blob。Owner 合并后，Controller 精确读回 main 回执、MANIFEST 与范围摘要再施工；本提案不宣称候选、安装或产品验收通过。 T00 轻核对：可信 `origin/main` `3609d91fe1fa33639c186079ec71de669b3c1dd6` 不含该路径；施工提交 `fcc19b542248d9cf1b04db931f19d30d2f135e07` 的静态源 blob 为 `cddc6faf4e68e2bf24f2d1c82ecccfec9ddf5bbc`，仅作来源核对，不表示该文件已在 main 生效。

## 2026-10-12：F.2 自管 CLI 生命周期的唯一共享桥接路径（范围提案，待 Owner 合并）

F.2 已要求显式升级、退回、自检、查更新和零实例卸载；原 USER dispatcher 目前只接安装流程。只新增 `apps/desktop/native-host/src/store/product_database/v37_managed_cli.rs` 为 INTEGRATOR 共享路径，将这些既定 USER 动作接到 F 的可信版本目录、原 copy/instance/SOURCE 和已有 ProductDatabase 权威事务。其私有 H/custody/intent 核对不能从 F 兄弟模块或已列桌面转发文件取得；现有桌面与 Node 共享路径继续由汇合席位使用，不新增其他写路径。

升级或退回必须选择编译内已验证的官方 archive/image/version，写前和完成时核对静止状态、精确 H/C、未结 intent 和 revision CAS；活跃或 UNKNOWN 默认拒绝；仅后述 2026-10-14 待审原生直接事实提案获准并精确读回后，才可对该动作完整历史集合中有原证明的成员作限定重评；2026-10-15 三家完整集合补充提案见后文，审定前仍按原单元守卫拒绝。当前规则仍为 Busy。原 HOME 与凭据字节保留，失败原文和 UNKNOWN 不改写。查更新只记未验证通知，不能激活；doctor 必须有已证明不登录、不改账户、不花模型额度的官方只读行为，不能顶替登录检测。零实例卸载由原 F 门槛拒绝任何现存实例。没有同家双版本可信字节时，真实升级/退回成功仍为 NOT_RUN，不把候选换装或 latest 通知算通过。

不新增数据库/issuer accessor、模型进程、调度器、权限、全局 npm/PATH 发现、自动升级、凭据读取或 Owner 登录步骤。`product_database.rs` 的 E.3 组合授权及其他 helper 不扩大；F 写域、需求、阶段、契约写方、不变量和 Owner 触点保持。本次仅增加一个 shared path，同 PR 更新范围摘要、v2 回执、MANIFEST 和三处固定回执 blob；Owner 合并且 Controller 精确读回前，不施工该路径。

参照已合并的 PR #88 单路径修订和当前 F.2；T00 轻核对以 main `a8ce38c2` 为准，main 尚无该路径，施工树现有 dispatcher blob `f4998aced71a52b9652f7e043c5f55b945824529` 只作静态定位来源。该提案没有产品改动，不宣称生命周期、真机或里程碑通过。

## 2026-10-13：原宿主错误返回仍执行既有 H 停止（Integrator 同范围 detail）

原生 desktop event loop 目前用 run_desktop(...)? 提前返回；一次原 USER 身份拒绝或原 transport 错误会跳过已经接入的 Secretary H stop 和 checked close。内部 ready 请求排队时原 shell 先退出，是该缺陷的一条可达路径；这条静态路径不能冒充已取得原现场 native stderr 或第三次正常退出。

H 发起，Integrator 只在既有 apps/desktop/native-host/src/main.rs 接线：先保留原 event-loop 结果，无论成功或错误都在原 ProductDatabase 权威线程执行已有 stop_secretary_before_close 和 close_checked，再返回原结果。原 event-loop 错误为主，停止与闭库的原错误一并保留；不把拒绝改为成功。继续使用原 H 的物理 StopFact、代际、预算和 Secretary 不 release 语义，不新增进程、线程、writer、权限、调度器、resume cancellation、重放或关窗延迟。F.2、G、E.3 依赖线已通知。

不新增共享路径或条件、不改变持久写方、requirements、invariants、Owner 决定和触点。可信校验器重算摘要须与现有 v2 回执相同；按 authorization 的 detail amendment 规则，独立审计通过后由 Controller 合并并精确读回，不新增回执或 Owner merge。原失败仍 UNKNOWN，不凭进程缺席补 STOPPED；原生错误路径、原 H 停止与有效 production mutation 走云检，新装候选使用共用 READY/EXIT/StopFact/十三项仪器重新跑真实快速关窗，未执行不得记通过。

参照：现有 main 的正常结束接线、H stop_secretary_before_close、K-SESSION 原 StopFact/no-release、User 私管道同步回执与既有 origin 拒绝；采用原机制，仅修错误提前返回的实现遗漏。

## 2026-10-14：prepared child 的两条失败清理路径（范围提案，待 Owner 合并）

`apps/desktop/native-host/src/store/product_database/v37_runtime.rs` 的 open 与 resume 在 C PREPARED 成功记录后、H 绑定或 activation 前，alias bind 失败会直接返回，遗漏调用仍由原 host 持有的 `abort_prepared`。提案只为这两处加入原 custodian 清理，并覆盖 `apps/desktop/native-host/src/store/product_database/v37_runtime_tests.rs` 的真实 suspended child 故障注入与有效 mutation。保留首因、清理错误及全部 UNKNOWN、claim 和 alias 状态；abort 不构成 StopFact，也不提供冷恢复，不改历史数据。Owner 在同一 PR 合并并读回前不施工。


## 2026-10-14：以直接原生事实限定重评历史版本阻断（待 Claude 复核合并）

Owner 已授权准备本提案，并指定提案 PR 交 Claude 复核合并；本段不代表产品实现或任何历史单元已资格化。Claude 经 Owner 授权审定后合并这一份提案，Controller 核实际 merge、回执 Git 字节及 main 的 scope_digest 后，才按生效写域施工。其余 Owner 触点、最终验收与公开发布权限保持。审定前 OpenCode 的 Busy 不变，F.2 正向验证先用经实际守卫核准、未被阻挡的自管 Claude/Codex 副本继续。

### 事实与不可变历史

版本变更默认拒绝活跃、UNKNOWN 和未结 intent。拟议例外只接受 H 的只读原生事实：受信内核直接证实重启边界已截断原执行所属进程世代；或按原 PID 与完整 creation FILETIME 精确确认原执行已不存在。精确对象族必须证明集合完整，不能将单个 PID 消失扩大为未知后代已停；经旧来源绑定的未激活 host/child 集合可作为有限集合。两族是各自资格化的选择，不要求缺失的历史 Job-ID，不把 ACL-only 列名 holder fence 自动提升为全树证明。不得用超时、时间差、阈值、宽限期、外层关窗或 abort bool 代替事实，不要求 Owner 重启来凑证据。

版本准入重评中，旧 C/H UNKNOWN、claim/owner-binding、alias、temporary home、原成功/失败和错误原文原样保留；不回填 STOPPED、StopFact、APPLIED 或新成功/失败结论。新原生观察不提供 admission release、旧会话 continuation、凭据读取、alias/worktree/HOME 清理或强杀权限。注册/认证 HOME 与凭据字节不动。若同一 PID 已可靠对应不同 creation，则只能证明旧精确对象已消失，不得操作当前对象；PID 重用或出生身份不能判清、查询拒绝/不确定、来源不可信或集合不完整一律 NOT_QUALIFIED，并保留原 OS 错误；不能降格为通过。

### 主体、原 namespace 与完整守卫

观察主体绑定原物理 Root/DB FileID、driver、instance、原 domain/session/generation/request/operation 的完整权威关联、原 PID/creation、原 image 和 HOME 身份、原 alias namespace/name/target 与相关 row 指纹。证明来源必须由受审原生 producer 直接取得、或由原有可信生产链验证；D 报告、操作员字符串与调用方自报 bool 不是资格。原来源链或原 namespace 无法核准时保持 Busy，不从新 logon 的同名盘符缺席推断旧 alias 已结算。

同一限定历史单元的 claim、owner-binding、open、alias、target temporary-home、C custody 与桥接层重复资源拒绝必须逐项关联并重评。所有其它单元、临时 HOME、driver、holder、intent、credential、image、签名、SOURCE 与 CAS 守卫仍执行；不能只处理最先遇到的三个 SQL。目标 temporary home 的状态只在该版本动作视图中参与精确资格，原 ACTIVE 行不改写。这个例外不关闭其它已停止或未启动释放的 temporary home，也不承诺一个历史单元解堵即可完成全部 F.2。

### 一次动作与重复检查的精确关联

使用现有 Design37InstallCliRequest 的 requestId，不发明新的用户输入。现有 product_entry.rs 在 versioned native begin、stage 和 probe 中转发同一个 requestId；最终仍是已有六字段 USER frame，requestId 必须相同。旧无绑定的 versioned frame 可以继续走原严格规则，但不得消费冷例外；first-copy 的旧路径不继承新权限。

首次显式 USER begin 在原 verified ProductDatabase/F 事务内建立一个严格动作行，以物理 authority、driver、requestId 唯一标识；保存 operation、原 source/copy/行级 revision 指纹、编译内 target archive/image/version、确切旧单元和原生事实来源。每 driver 最多一个未结动作。相同 requestId 与原始 begin bytes 只能只读回取原结果并重验资格；更换 bytes、target、operation、旧主体或来源即拒绝。不同 requestId 遇未结动作保持 Busy。后续 stage/probe 必须携带同一 requestId、匹配实际 stage 与已记录的 copy/source 前后 revision 链；不把动作自身合法写入误判为外部漂移。

每次 begin/stage/probe、写前、最终 switch 与 receipt 对账都重验资格和其它完整守卫；只读 guard 不耗尽资格。最终 USER 原 raw frame/hash 在原 switch 事务绑定同一动作，效果、原 F switch receipt 与动作消费原子完成。COMMIT 结果未知时只读原动作和原 receipt，不能以新 ID 重做或重放旧 open；已明确失败的准备保留其原错误，另一次动作仍须满足现有失败/重试守卫。rollback 是一个新的 requestId/动作和当前资格核对，不能沿用上次一次性许可。原历史 image 永远保持原值，与本次 source/target image 分开。

### 最小实现写域与生产者

新增 H 观察实现和声明限于现有 process/session/；现有共享 process/mod.rs 只 re-export 受限类型，类型不允许外部构造或 Deserialize 冒充事实。F 的严格动作 journal/schema、版本消费者和测试限于现有 store/instance/，包括 managed_cli.rs、program_source.rs、mod.rs 及必要的 F-owned helper；F/K-INSTANCE 仍是唯一持久写方。原十表形到新增精确动作表形的迁移必须完整、原子且拒绝影子/损坏/未知表，不分类或清除旧 H/C 行。

现有共享 v37_managed_cli.rs 是同一 ProductDatabase/RootLock/VerifiedDatabaseConnection/OwnerIssuer 桥；product_entry.rs 仅转发既有 USER requestId，不增加 UI 字段、登录、权限或第二调度器。版本准入本身没有新增冷 helper 共享文件，也不扩 E.3；下文独立 F HOME 关闭只给 product_database.rs 增一条限定 USER 路由，E.3 本身保持。PR #93 的两条 runtime 共享路径在本提案完整保留，仍仅供同原 host prepared child 的预防性 abort，不用于冷证明或历史结算。发生其它共享路径、writer、UI/命令或权限需求时，施工先返回 Controller，不自行扩大。

### 验证与非结论

正式原生构建和测试遵循 docs/governance/gogoke-build-and-release.md，全部走云端。先验仪器，核实际加载的源码、构建与文件身份；用真实原生精确对象消失和可靠 PID 复用正例，以及活的同一精确对象、伪造/不明出生身份、拒绝访问、原始查询失败、未知后代/旧来源、namespace/目标漂移、跨单元/driver/home、缺 requestId、重复 begin/各 stage/final 不同 ID、行级 CAS、COMMIT unknown 与独立 rollback 反例。真实重启族正例必须有直接重启证据，模拟时间或 mock 不算。有效 production mutation 必须仍可编译，并因行为断言失败，恢复精确字节后再通过。新候选实际 Win11 USER 路径沿用共用 READY、StopFact/绑定表、正常关闭和十三项仪器。

本提案只授权边界，不生成缺少的历史证据。任何具体历史单元仍须在未来原生资格中逐项核准；未执行的原生/云端/Win11/协议 golden/F.2 结果均为 NOT_RUN，跳过不是通过，合并不是验收。
## 2026-10-14 补充：原证明关闭历史临时 HOME 与逐案例收尾（同一 PR94）

Owner 指定补充直接并入 PR94，改完重新交 Claude 复核合并，不另开 PR。只准备和修订测试流程；审定、实际 main 回执与范围读回之前保持 Busy，不执行产品关闭或冷例外。旧 head 的审查和检查不覆盖新字节。

采用独立 F-owned K-INSTANCE/temporary-home-close USER 动作，唯一新 close request/raw、严格 selector、原 F create/H/C 完整来源、当前物理身份与 revision、原 StopProof 或合格直接精确身份消失事实，才能把一条 ACTIVE(1) 临时 SESSION HOME 原子写为 CLOSED(2)并给独立 F receipt；目录和 marker 保留。原 H/C UNKNOWN、业务 stop/release/结果原文不改，原 v1 receipt 兼容；新 absence receipt 严格区分证明种类和来源。不凭超时、空集合、release、外层关窗或清单放行；无原执行身份且未启动的历史 HOME 缺证明仍拒绝，不偷增第三证据族。精确 request/receipt、拒绝、COMMIT 未知回读及 finally 流程见 [TEST_CASE_HOME_TEARDOWN.md](TEST_CASE_HOME_TEARDOWN.md)。

[逐项历史清单](HISTORICAL_TEMP_HOME_PROOFS.json) 记录共用闭库报告中的 297 个 HOME：OpenCode 9、Claude 24、Codex 264。183 项仅为记录的核心停止 tuple 候选，19 项投影无完整 tuple，95 项缺旧 episode 投影；不能把缺投影当成不存在原证明。每项都明确保持 REFUSED_PENDING_DIRECT_PROOF_AND_SOURCE_LINKS，当前原生逐 HOME 来源、marker/SID/FID/revision 和精确身份消失资格仍未运行。C71 与未启动 release 不能借候选列表变成已证明。清单属于待证事实，不是批准或原生事实生产者。

最小写域新增 v37_session.rs 与 v37_session_tests.rs 两条共享路径，仅供原 USER 桥精确选择并调用 F writer 及生产接点验证；现有 product_database.rs 仅增加此 K-INSTANCE 路由，process/mod.rs 仅允许 F 读取 H 构造的受限事实。F store/instance 保持唯一 HOME 持久写方；H session_transport/wire 只扩明确 whitelist，不加原 stop/release 副作用。PR93 同原 host abort 与原 cold-version 全部一动作守卫保留；版本 begin 不自动关闭其它 HOME。

测试共用 installed-temporary-home-lifecycle.mjs 与 product-cdp.mjs，并接 provider/visible 案例：每个原创建尝试在 dispatch 前归属本例，部分失败/UNKNOWN也保留；finally 沿合法原停止路径取证，再仅按本例原 createRequestId 请求关闭，保持原业务错误和独立收尾错误。完成前用共用闭库读回确认原 HOME 集合 CLOSED、其它 HOME/认证对象和十三项保护不变；缺证明或缺生产者归属为 CASE_TEARDOWN_INCOMPLETE，不能以关窗、skip 或预期业务拒绝作通过。未迁移的创建路径不得开启新案例；私有历史脚本不改写为新成功。产品桥及新共用调用器需在本授权生效后施工并按实际加载版本验证，当前未运行 HOME 关闭或正向换版。

不物理删除 HOME/alias/worktree、不改凭据或系统权限、不强杀、不伪造 StopFact、不新增 Owner 登录/重启/手工安装触点。相同 PR 内更新 scope_digest、回执、三处固定 blob 和实际 Git LF MANIFEST；旧回执仍直接 supersede 当前 main 生效值，不把未合并 PR93/旧 PR94 作为已生效授权。受影响新 head 云检和 Claude 复核须重新完成。

## 2026-10-14 后续：Codex M1 legacy SOURCE 单实例迁移（待 Claude 复核与授权合并）

Owner 已明确允许准备 PR94 之后的后续授权 PR 并交独立 Claude 复核；本次许可不委派 merge 或执行，审定前继续拒绝，后续生效仍需另获授权的 main merge 与精确读回。本提案只覆盖公开符号 `LEGACY_CODEX_M1`：原 ProductDatabase 中唯一一条未 tombstone、disabled、Codex、SOURCE 整行缺失的 legacy 实例。实际实例 ID、HOME/FileID 与账号观察留在私有证据，不进入公开仓库。原生必须独立枚举并证明这类候选恰好一条；零条、多条、enabled、换 target 或 caller 自选替身都拒绝。

先对真实原 schema/raw、APPLIED operation/native receipt、HOME/pin 和既有 replay 前置条件做只读三态判定：ELIGIBLE 仅考虑原请求精确 replay，本迁移拒绝；PROVEN_INELIGIBLE 必须由完整原记录正向证明哪个实际前置条件不适用，并另有原 legacy 注册完整性证明；UNKNOWN 一律拒绝。现实现没有独立 SOURCE-bind journal，不能用其缺失、SOURCE 缺行、旧名字/版本、APPLIED 标签或试跑 replay 失败推断 legacy，也不得为试探资格派发注册或写 SOURCE。给定摘要未含完整原注册 raw/receipt，实际 M1 分类仍 UNKNOWN。批准目标的原注册/HOME 锚点必须与新请求和原生解出的 journal 精确相符；当前唯一候选不等于同一原目标，锚点变动不重新选人。

迁移资格要求原 Root/DB/HOME 的精确身份与当前物理 HOME FileID/marker/SID、原注册请求和回执、编译内官方 Codex 同 pin catalog、当前 stage 的 archive/image/executable/version 完整来源链，以及所有会影响该实例或共享 current-stage 选择的 claim、owner-binding、session/generation、H/C custody、alias、temporary HOME、pending operation、managed version action 与行 revision 的完整静止快照。未结执行只能用原 StopFact/StopProof，或 PR94 已授权的、覆盖完整原执行身份集合的 fresh typed native 精确消失/直接内核重启事实资格化；超时、宽限期、年龄、关窗、空 caller 集合、PID 单值或 ACL-only 记录均不算。任一来源、对象族、身份、查询或 revision 不完整即拒绝并保留原错误。

写入必须由一个新的显式认证 Owner-layer migration request 发起，使用独立 requestId/raw 和独立 F/K-INSTANCE durable journal；它不是原注册 replay，也不是后续版本 requestId。journal 至少绑定新请求、唯一 native-selected target、原注册 raw/receipt、Root/DB/HOME 身份、写前 SOURCE 缺行指纹、同 pin stage/包来源链、完整静止事实与所有 CAS revision、phase/result/receipt 及写后 SOURCE 指纹。本次首次持久意图修订提案把不可换 ID 边界明确为已核实提交的 PREPARING：首笔 COMMIT 成功前不能尝试 SOURCE 效果；首笔提交报未知仍保留原错误与 ADMISSION_COMMIT_UNKNOWN，当前调用不执行 SOURCE。之后只在同一原物理 Root/DB、同一批准注册/HOME 锚点下，成功读取完整专用 journal 为空且该批准目标 SOURCE 缺行时，才允许后续显式认证 Owner 请求用原 ID 或新 ID 重新申请受理，并重跑全部原生资格；这只是明确的受理前例外，不改写旧未知结果。任一目标的持久 journal 已存在即排斥新迁移；PREPARING 仅同 ID/raw 读回或拒绝、绝不恢复效果，APPLIED 仅同 ID/raw 与精确 SOURCE 读回原回执。错配、缺表、查询失败、身份或锚点不确定一律拒绝。Owner 2026-10-15 只授权准备此修订；独立复核、另获授权的 main 合并和精确读回前不实现、不执行，保持拒绝。

唯一允许的数据效果是在原 verified ProductDatabase/RootLock/OwnerIssuer 事务中，原子写独立 migration journal/receipt 和一条此前不存在的 `gogoke_v37_instance_program_sources`。SOURCE 仍写 MANAGED、当前官方 stage、原 instance 同 pin digest/version、原 home identity 和原 APPLIED registration request ID；独立 migration journal 才说明它是后来由新 Owner 意图创建，任何读回不得声称历史注册原本就是 MANAGED。不得更新或删除既有 SOURCE，不改 instance/profile enabled/tombstone/login、copy state、版本、HOME、credential、alias、H/C、claim、原 operation/receipt 或旧 UNKNOWN。

现有受控施工源的 `program_source.rs::migrate_quiescent_legacy_instances` 和 `v37_managed_cli.rs::migrate_managed_cli` 只作规划来源，当前批量/直接写 SOURCE 的形状不满足独立 Owner request/journal 和唯一 disabled candidate 契约，不能因本提案合并就用于该案例。施工仍限 F 的 `store/instance/` 与已列共享 `v37_managed_cli.rs`；允许在既有 authenticated USER managed-cli schema/桥内新增一个精确字段的严格迁移 command arm，把独立 requestId/raw 与原目标锚点交给 F journal，旧四字段 migrate 帧不能隐式获得该效果。H 只提供既有受限 typed native facts；新顶层入口/transport、新 writer、未列 shared path、UI/权限或第三证据族均先返回 Controller。

完整契约、验证负例和停止点见 [LEGACY_SOURCE_MIGRATION.md](LEGACY_SOURCE_MIGRATION.md)。本 PR 只改变授权范围，不实现或执行迁移；未跑 native/云端/真机均为 NOT_RUN。即使以后 SOURCE migration APPLIED，原 version begin/stage/probe/switch、SOURCE/CAS、COLD/Busy、credential、其它实例与历史阻断仍全部执行，不承诺 preparedReplacement 或 F.2 通过。


## 三家受管 CLI 完整历史集合补充提案（2026-10-15）

Owner 要求一次查全 Claude、Codex、OpenCode 当前库所有阻挡类型，统一采用有原证明才处理、缺证逐项拒绝。完整 28 类原表、原守卫、原证明、独立拒绝和当前观察清单见 [MANAGED_CLI_HISTORICAL_FAMILIES.md](MANAGED_CLI_HISTORICAL_FAMILIES.md)。本补充仅拟将前述单历史单元限制改为 native 独立枚举的单 driver、单 requestId、同一物理 Root/DB 完整有限集合；原 StopFact/StopProof 或既有完整精确消失/内核重启证明逐成员资格化，所有重复守卫消费同一事实。旧 UNKNOWN、PREPARED、HOME、H/C/F 原结果不改，不自动关闭 HOME。SOURCE 缺口、未结版本/F/凭据效果和旧 namespace 缺证仍逐项拒绝；NoLaunch 仅另列同份 D-NOLAUNCH 待 Owner 明确裁决。除该项获准后的限定只读原启动边界 constructor 外，不增加证明生产者、写方、OS/namespace 观察、transport、UI 或权限。未来写域仅合同列出的九文件，H 两文件保留 H 所有权，只有 D 明确获准后才可增加该 sealed 只读源接口；否则仅原语义 helper/re-export。Owner 已指定该同一提案交 Claude 独立复核合并，Controller 精确读回后才施工；已合并 PR96 的 SOURCE 首次持久意图状态规则完整保留；本提案另明确 D-NOLAUNCH 对原单一 M1 静止判定的有限消费者，仍须同一待裁决项批准。当前已完成一次同库原始采集，完整匿名逐项观察见 CURRENT_HISTORICAL_OBSERVATIONS.json；未运行的 native/内存/kernel/namespace/ACL 资格仍为 NOT_OBSERVED/NOT_RUN，不能当作零或已解堵。


### 同一三家提案的 D-NOLAUNCH 待裁决项

已观察到 processless COMMITTED 历史类型，现有三类证明不能处理。具体提案在 MANAGED_CLI_HISTORICAL_FAMILIES.md 的 D-NOLAUNCH：仅拟在九个精确写域内由 H 构造 sealed 只读原未跨持久启动边界事实，F 用独立第四 kind 消费；必须证明该历史实际 producer/原合同、原 raw/receipt/关联/负集合及每条对应 OS prepare 路径均受同一持久 H OPEN/RESUME 支配。当前 e995 源码、无 C、空 PID、release 或时间均不足；全局登录/credential/ACL/history 准备效果独立拒绝。无原证明仍拒绝、不释放 claim、不关 HOME、不改 UNKNOWN/PREPARED，不新增 OS/namespace 观察或其它写方。因这是新的封闭证明边界，Owner 必须明确批准该项后，Claude 才可在独立复核和最终提交全部检查后合并同一提案；审定前整份保持 draft/Busy，不预授权施工。


### D-NOLAUNCH 的原单一 M1 SOURCE 静止消费者

同一待裁决项还精确列入 legacy_source_migration.rs：仅原 PR95 单一 M1 的 processless claim 及精确关联 ACTIVE 临时 HOME 的 H 执行资源静止检查可消费同一 H sealed 原未跨启动边界事实。不得把 COMMITTED/HOME 改称 RELEASED/CLOSED，不改原行；原注册、物理 Root/DB/HOME、官方来源、全局 C/H/F/凭据效果与 CAS 均独立，PR96 首次持久意图状态表不变。缺该历史实际 producer/原证明或任何其它独立条件，继续拒绝；不是新增 SOURCE 目标或写方，不是实际迁移授权。

### T07 旧 probe 占用与新版输出的限定修订草案

Owner 已批准独立审查后的限定 B 资源边界：在原 USER、RootLock、同一 verified Root/DB 内，完整读取原 C 十二列及 NULL 语义，保留 STOPPED/hash 为原存储数据；对无任何保留 action 关联的原 C，仅以其同 Root 的精确官方物理 stage/catalog/archive、原 F/SOURCE/注册直接关系和已有 sealed KERNEL_REBOOT 对原 PID/完整 creation 的真实 preboot 确认，重评该旧进程占用。原 operation/hash 不能反推旧 request、StopProof preimage、确认、stdout、命令类别或版本成功；无保留关联不证明历史非 action。可关联 action 或残留效应指向缺失 action 禁止降级；其它/恢复后未决 PROBING Busy，当前 action 正常原子 PROBING→PROBED 和完整 CONSUMED/独立 rollback 仍走原规则。原完整集合及字段绑定新 journal，在 begin 重放、stage、probe、pre/post-effect、final、回执重放逐步重新枚举和 native 验证；缺原来源、同 boot、空集、查询错误或漂移 Busy，保留原错误。其余 HOME/SOURCE/凭据/FHG/ACL/child/namespace、D-NOLAUNCH、UNKNOWN/PREPARED 等拒绝独立有效。旧 stdout 仍 `NOT_RETAINED`，不补旧命令/证明/结果。新版 STRICT/COLD action 的实际 confirmed StopProof、唯一 C、stdout/exit、原公开 receipt bytes 仍在 PROBING→PROBED 持久化并在 PROBED/CONSUMED 重验；不加 schema、OS producer、公开字段或写者。B01–B18 和真实跨启动正例须实际验证；未闭合前继续 Busy，完整规格与反例见同份 MANAGED_CLI_HISTORICAL_FAMILIES.md。

这是同一三家合同的限定修订准备；Claude 独立复核、授权 main 合并与精确 scope/receipt 读回前不实施旧 probe 例外或新版动作，不新增 Owner 触点。
