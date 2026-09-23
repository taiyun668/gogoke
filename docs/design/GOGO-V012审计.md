# Codex《gogoke 设计 V0.12》审计

- 被审对象：`gogoke_design_v0_12.md`，2026-09-16，自述状态「供设计复核，不自报审计通过」
- 审计日期：2026-09-17
- 判据三样：**gogoke 的实际代码**（基线 `ab006d5`，本仓库 `apps/desktop`）、**它自己的内部一致性与证据**、**本仓库已有的现成做法与实测**
- **不作为判据**：Owner 2026-09-16 之后的裁决。那批用于整合，不用于评判一份在它之前写成的稿子

---

## 结论

**不能原样作为施工依据，但不是推倒。**

- **A 类（必须改）**：它对 gogoke 现状的盘点漏了一整块——现成的席位与派活模型。据此写出的落点表和「派出卡由宿主新建」会造成重做。
- **B 类（必须降级）**：两条最贵的承诺（Managed-Tools v1、步骤隔离）在本仓库里有更便宜的现成做法，而它自己把这两条列为未验证的原型门。
- **C 类（直接收）**：它挑出的五个时序缺口是真的，且本仓库其他文档都没覆盖。这部分价值最高。

---

## 一、A 类：对 gogoke 现状的盘点有一整块缺口

它 §1 写「优先复用现有产品」，§19 给了落点表。但落点表里**没有任何一行提到 gogoke 已有的协作模型**：

| gogoke 已有的**渲染层**（已核类型与源码） | V0.12 的处理 |
|---|---|
| `ConversationItem.kind:"tool"` 带 `collabSender / collabReceivers[] / collabStatuses[]`；`toolType:"collabToolCall"`；解析在 `utils/threadItems.collab.ts`；补全在 `enrichConversationItemsWithThreads` | **未提及**。§3.3 把「派出卡」当成要由宿主新建的东西 |
| `ThreadSummary.isSubagent / subagentNickname / subagentRole` —— 席位就是带角色的线程 | **未提及**。§5.1 把席位注册表当成新建 |
| `Sidebar.tsx` 的 `pendingUserInputKeys` —— 左栏已经在标哪条要你答 | **未提及** |
| `useResponseRequiredNotificationsController` 的 `isSubagentThread` —— 已在区分席位线程与主线程 | **未提及**。§1「席位不直接跟你说话」被当成全新约束 |
| `ThreadListOrganizeMode = by_project / …` —— 左栏已按项目分组 | **未提及** |
| `tool.changes[] { path, kind, diff }` | §19 只写「Git／Diff／Files → ResultWorkbench」，没说这条已有 |

**这一块为什么重要**：`sender_thread_id` 是下划线命名，来自 app-server 线上协议。**上表每一项都是 Codex 的协议形状**——`src-tauri/src/` 里只有 `codex/` 模块，claude／anthropic／grok／xai／gemini 零引用（2026-09-17 核）。**gogoke 现在是纯 Codex 的，它有的是渲染层，不是产品能力。**这恰恰印证了 V0.12 §1「Harness 对等」的必要性（今天确实只有一家能用），但它没有把这件事写出来，于是：

- §3.3「派出卡只由 `execution.registered` 及后续宿主事件创建」——方向对，但没说它要**替换**现有的 collab 渲染路径，而不是在空地上新建；
- §19 的落点表少了 `utils/threadItems.collab.ts`、`features/threads/hooks/useThreadLinking.ts`、`MessageRows.tsx` 的 collab 分支这几处必改点。

**判定：落点表必须重做一遍**，否则施工时会出现两套派活显示。

### 附带的一处内部矛盾

§1 写「保留 gogoke 的布局、编辑、Git、Diff、Files、终端和设置体验」，§19 却写「现有 messages／composer → `features/conversations/BoundComposer`」——把输入区整个换掉。保留与替换在同一份稿子里对同一个对象给了两种处理。**需要它自己定一个。**

---

## 二、B 类：两条最贵的承诺，本仓库有更便宜的现成做法

### B-1 Managed-Tools v1（§11.1）

它要求：关闭三家 CLI 未托管的内建读写、终端、浏览器、插件、hooks 与额外 MCP，全部副作用改走自建工具网关，配 LPAC ToolWorker。

**本仓库里有一份在生产里跑着的实现**，`docs/design/22-seat-runtime-inheritance-ledger.md` 记录（本体在 `<local>\Grok Worker Provider`；4954 行，实测 51 次调用 / 7240 万 token / 4 账号 active；本次已核源码）：

| 做法 | 出处 |
|---|---|
| 启动契约当数据，spawn 前逐条断言：`terminalDenied` `subagentsDenied` `compatHooksDisabled` `folderTrustEnabled` `trustFlagAbsent` `noPlan` `noMemory` `streamingJson`，并老实写 `windowsSandboxEnforcement: false` | `lib/provider.js:656`，断言 `:441` |
| 两级家目录：持久按账号装认证与模型缓存，临时按每次调用装 HOME／USERPROFILE／LOCALAPPDATA | `:698` |
| 硬拒默认家目录及其父子路径 | `:424` + `:327` |
| 路径围栏：拒符号链接与 junction，限死允许根 | `:308` / `:320` |
| 项目权威预检：拒绝派进带 `.claude/settings.local.json` 的目录 | `:517` / `:527` |
| 剩余工具边界 **35 行**：`bash`／`run_terminal_cmd` 直接拒，`edit`／`write` 比对允许写入根，每条追加审计 | `lib/hook-boundary.js` |

第二条正好解掉 V0.12 §5.2 自己点出的死结（Codex OAuth 与 `CODEX_HOME` 耦合，严格隔离就丢登录态）：**认证留持久层，隔离做临时层。**

另有三家旁证：Codex 自带 Windows 沙箱（ACL + WFP + 独立身份 + 提权后端）、Grok Build 自带 permission rules / exec risk / preflight / auto mode、DeepSeek Harness 的 `sandbox-windows-acl`（本项目审计判定只覆盖 ACL 能表达的一片，只能标 partial）。

**四家里没有一家把内建工具全关掉再重写一遍。**

**判定：降级。** 改为「收口清单 + spawn 前断言 + 一个钩子 + 路径围栏」，不建统一工具网关，不建 LPAC ToolWorker 子系统；能力如实标注，不宣称 OS 级强制。V0.12 §11.1 那条「一家收不了口则三家统一不发布」保留。

### B-2 步骤隔离（§6.2、§8.4）

它要求：一个 `RuntimeUnit`／`NativeBinding`／`SourceEpoch` 只服务一个 `NativeStep`，回合终态后退役旧生产者，下一步新建绑定。目的是消除旧工具帧误归新执行（R11-01）。

**问题是真的。但它选的解法是本项目已经付钱掉过头的那条路。**

`22-seat-runtime-inheritance-ledger.md` §4.1 原文：`provider.js:433` 的 `spawnSync` 是 GOGO 与 grok-worker 分岔的那一行；隔离、契约、密钥卫生、围栏、审计全部可继承，**只有进程生命周期这一处必须换掉**——换成常驻。§4.2 给的理由：一次性席位**中途插话纠偏不可能**、上下文每次都是新人。

准确地说 V0.12 是中间态（回合内进程活着，回合之间靠官方 resume 接上下文），不等于回到 `spawnSync`。但它买回上下文连续性的那个机制，在本仓库的实测表里只有 SOURCE 等级：`docs/research/adapter-spike/01-capability-evidence.md` 记 Codex `thread/resume|fork|archive` 为 **SUPPORTED / SOURCE，「仍需动态隔离测试」**；Grok load/resume 为 **SOURCE + HANDSHAKE，「仍需动态重连测试」**；未 PASS 清单第三条正是「进程崩溃、Controller 重启、协议断线后的 resume/reconcile」。

**同一个问题，四家都不靠重启解决：**

| 做法 | 出处 |
|---|---|
| Windows suspended spawn，子进程加入 Job 前起不来；持有 process handle 防 PID 复用 | Codex `codex-rs/utils/pty` |
| Weak registry 防 PID 复用误杀；close/spawn 竞态即杀晚到子进程；`kill_all` 幂等 | xAI `ProcessScope` |
| 记 `pid + processStartTicks`，区分数据属于活进程还是已死进程 | grok-worker `captureRunOwner` / `inspectRunOwner` |
| terminal receipt fence：把「进程结束」与「回执提交」分成两件事 | Claudexor |

**判定：不做步骤隔离。** 席位保持常驻；来源归属用不可复用的进程身份（`pid + processStartTicks`）加 receipt fence。若要坚持步骤隔离，先补 resume 的动态实测，否则等于把架构押在一个 SOURCE 级能力上。

---

## 三、证据面的两处缺口

1. **它没有引用本仓库的实测**。附录列的是 W1–W11（厂商文档）与 D0/D1/A0/A1（它自己的前几版）。`docs/research/adapter-spike/01-capability-evidence.md` 是本仓库唯一一份对三家做过 OBSERVED 级观察的材料，它没引。于是它在厂商文档说「支持」的地方直接当作可用，而实测表在同一处标着「仍需动态测试」。
2. **它没有引用 `22-seat-runtime-inheritance-ledger.md` 与 ADR-0006/0007**。前者是本仓库唯一一份「每条都有生产实现出处」的文档，正好覆盖它 §11 的问题；后者是本项目对同一问题已接受的立场（按 profile 出 CapabilityEvidence，声明 enforcement strength，而不是全局网关）。

**判定：不是造假，是取证不全。** 修订时必须把这三份纳入判据。

---

## 四、它自己承认未验证的三处

§24.2 列了三道原型门：工具面与 MCP、官方认证及并发、LPAC 与副作用停止。§6.2 另说步骤隔离的成本与恢复质量待原型验证；§8.1 说「协议支持不等于指定 Grok binary 已通过」。

**这份稿子最贵的三项承诺，按它自己的说法都没有验证过。** 它写明了，没有隐瞒——这是它的优点。但意味着：**设计复核可以过，实施放行不能过**，与它 §0 的自述一致。

---

## 五、C 类：它对，而且本仓库其他文档都没有覆盖——直接收

| 它的规则 | 为什么收 |
|---|---|
| **五种事实分开**（§10.1）：原生步骤结束／工人返回／检查通过／采用某版本／目标验收 | 「采用」与「验收」是两件事，本仓库其余文档都把它们混作一个 |
| **CLOSING 与在途结算**（§10.7） | 终结意图已登记但还有在途工作，这个状态原来没有人处理 |
| **答复承诺点**（§9.3）：`RESPONSE_RESERVED` 不等于准许外发；`response_fact` 与 `execution_control` 两个字段不许互相覆盖 | 直接决定「要你定」那张卡怎么做——单状态卡会同时宣称「已批准」和「从未提交」 |
| **ProjectInstructionSnapshot**（§14.5） | 项目规则谁装载、哪个版本、缺了怎么办，本仓库整个没写过 |
| **私聊分室**（§4.4）：Owner 私聊与主控委托审查用不同逻辑会话、不同历史、不同可见域；盲审另建干净会话 | 回答了「独立审计怎么才算真」这个一直悬着的问题 |
| **来源不可改绑的原则**（§8.4）：工具来源由启动前固定的记录决定，不由「谁现在活跃」反查 | 原则完全成立。只是实现不必靠换进程——见 B-2 |
| **INV-01～INV-12 不变量表**（§17.2） | 可直接作为验收断言 |
| **单写宿主 + outbox + 幂等 + reconciliation**（§13） | 与本项目 ADR-0004/0005 的既有立场一致 |
| **四层对象解耦**：席位／实例／绑定／进程 | 与 ADR-0002、ADR-0006 一致 |

---

## 六、给 Codex 的修订要求

1. **重做 §19 落点表**，先盘 `utils/threadItems.collab.ts`、`useThreadLinking.ts`、`MessageRows.tsx` 的 collab 分支、`RequestUserInput*`、`ApprovalRequest`、`pendingUserInputKeys`、`isSubagentThread`、`tool.changes[]`，写明哪些是改、哪些是替换、哪些不动。
2. **§1「保留 composer」与 §19「换成 BoundComposer」二选一**。
3. **§11.1 Managed-Tools 降级**为收口清单 + spawn 前断言 + 钩子 + 路径围栏；把 `22-seat-runtime-inheritance-ledger.md` 列为判据。
4. **§6.2／§8.4 取消步骤隔离**，改用进程身份 + receipt fence；若坚持，先出 resume 的动态实测。
5. **附录补三份判据**：`adapter-spike/01-capability-evidence.md`、`22-seat-runtime-inheritance-ledger.md`、ADR-0006/0007。
6. 明确写出这句现状陈述（整份稿子最重要的一句，现在没有）：**gogoke 今天是纯 Codex 的，它现有的席位、派活、问题、审批都是 app-server 的协议形状；可复用的是渲染层，产品能力为零，适配层从零建。** 把一家的能力当成产品本体，是本稿最需要堵的口子。

---

## 七、审计结论

| 类 | 处理 |
|---|---|
| §10.1 / §10.7 / §9.3 / §14.5 / §4.4 / §8.4 原则 / §17.2 / §13 / 四层解耦 | **通过，直接收** |
| §11.1 Managed-Tools、§6.2 步骤隔离 | **不通过，降级或取消**（B-1、B-2） |
| §19 落点表、§3.3 派出卡、§1 与 §19 的 composer 矛盾 | **不通过，重做**（A 类） |
| §24.2 三道原型门 | **保留为门**，未过不得放行实施 |
| 其余（§2–§5、§7、§12、§15–§18、§20–§23） | **未逐条审**；本轮只审了与 gogoke 现状、现成做法、证据面直接相关的部分 |

**设计复核不通过，但缺口是定点的，不需要推倒重写。** 按第六节六条修订后可再审。
