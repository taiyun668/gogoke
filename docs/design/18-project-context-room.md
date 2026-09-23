# Project Context Workspace：卡片式项目上下文现场

- 状态：`v0.3-p31-d1-product-contract-draft`
- 日期：2026-08-22
- 产品决定来源：Owner 提供的双 Agent 卡片式时间线视觉参考；P31 D1 收口 ContextEntry/CollaborationProposal/ReactionSignal
- 性质：P31 产品契约草案；P20/P21 投影输入；不改变 Task、Receipt 或 Gate 权威；Owner `ACCEPTED` 前非实现授权

## 1. 产品目标

Project Context Workspace 把一个项目中不同 Agent 的成果、提问、交接、审计、决策和上下文变更投影成一条易读的卡片时间线。视觉上接近群聊或协作动态流：每个 Agent 有清晰身份，卡片显示内容、时间、提及、回复和反应；语义上它不是普通聊天记录，也不是新的消息总线。

它不是全局 Command Center 的中央工作面。用户先从 `Projects` 选择一个 Project，进入该 Project 后，项目首页本身就是 Context Workspace：左侧选择 Workflow/Task/Party/Agent，中间显示该项目内相应作用域的 Agent 上下文流，右侧检查结构化事实、执行状态、Gate、证据和允许动作。

它必须同时解决两个需求：

1. 人能像阅读团队协作频道一样理解“谁向谁交付了什么、为什么、下一步是什么”。
2. 系统仍以结构化 Project/Task/Assignment/Result/Evidence/Receipt/Gate 对象为权威，不从自然语言或表情反推状态。

## 2. 双层模型

```text
┌────────────── Project Context Workspace ─────────────┐
│ Human-readable Context Feed                          │
│ Agent cards / mentions / replies / reactions / diff │
└───────────────────────┬──────────────────────────────┘
                        │ typed projection + Intent
┌───────────────────────▼──────────────────────────────┐
│ Authoritative Context Ledger                         │
│ ContextEntry / CollaborationProposal / Instruction   │
│ Fact / Decision / Result / Evidence / ContextSnapshot│
│ Assignment / Receipt / Gate / Handoff / revision     │
└──────────────────────────────────────────────────────┘
```

Context Feed 可以折叠、筛选和重新排序显示，但不能改变 Context Ledger。Context Ledger 只能由 Control API、Controller、Transition Engine 与相应 Gate 推进。

## 3. 与 Projects 融合

不设置独立 `Context` 一级导航，也不把项目对话混入全局 Command Center。入口关系固定为：

```text
Command Center
└─ 跨 Project 运营总览：状态、Attention、Gate、风险、资源

Projects
└─ Select Project
   └─ Project Context Workspace（默认项目首页）
      ├─ Project scope    完整 Party/Agent Context Room 与项目控制
      ├─ Workflow scope   某次 WorkflowRun 的任务接力
      ├─ Task scope       某 Task 的提问、Result、审计、Gate 因果链
      └─ Agent scope      某 Agent 的 Assignment、Session、Snapshot 与 Handoff
```

全局 Command Center 只能导航到 Project 或 Attention，不提供项目上下文 compose、reply、批准或派活。进入 Project 后，`Facts / Instructions / Snapshots / Projections / Handoffs` 作为右侧 Context Inspector 的标签和中央 Feed 的筛选器。用户在项目范围内始终留在同一个上下文现场，不需要在“项目状态”和“上下文管理”之间切换。

## 4. Project Context Workspace 布局

```text
┌ Project identity/revision/WorkflowRun/Party + automation control ──┐
├──────────────────┬───────────────────────────────┬─────────────────┤
│ Scope Navigator  │ Context Canvas                │ Inspector       │
│                  │                               │                 │
│ Projects         │ Agent card / @Agent / reply   │ Canonical state │
│ Workflow / Tasks │ Result / audit / handoff      │ Attempt/Session │
│ Party / Agents   │ Decision / context drift      │ Fact/Evidence   │
│ Attention / Gate │ reactions + allowed actions   │ Snapshot/Prompt │
│                  │                               │ Gate/LoadProof  │
├──────────────────┴───────────────────────────────┴─────────────────┤
│ Structured command composer: Ask / Task / Audit / Decide / Run    │
└────────────────────────────────────────────────────────────────────┘
```

项目顶部同时保留 Attention、Human Gate、Unknown risk 和 automation limit；它们是当前 Project Context Workspace 的控制带。移动或窄屏时 Scope Navigator 和 Inspector 变为抽屉；不能因收起抽屉而丢失 Project scope、卡片权威状态或当前动作边界。

## 5. 卡片结构

每张卡片最少显示：

- Agent 头像、稳定名称、Role、Harness；
- 精确模型/ExecutionTarget、Profile 类型、Host 与 readiness 新鲜度；
- `Author | Recipient | Observer` 关系；
- 绝对时间与相对年龄；
- 卡片类型和权威状态；
- 人类可读摘要；
- 结构化收件人 `@AgentRef`；
- 绑定的 Task/Assignment/Attempt/Session；
- source revision、ContextSnapshot 和 Prompt digest；
- Result/Evidence/Receipt/Gate/Handoff 引用；
- evidence strength 与 freshness；
- Controller 给出的 allowed actions。

示意：

```text
○ Claude · Auditor                         12 minutes ago
  QUESTION · PROPOSAL                     @Codex Implementer
┌────────────────────────────────────────────────────────────┐
│ Can you fix this issue?                                    │
│ Bound: task/42 · result/res_... · source@abc123             │
│ Context: ctx_... · visibility: IMPLEMENTER_ALLOWED          │
├────────────────────────────────────────────────────────────┤
│ [Reply] [Create correction task] [Request evidence]         │
└────────────────────────────────────────────────────────────┘
           │ structured reply-to / causation
○ Codex · Implementer                      12 minutes ago
  RESPONSE · NOT_ACCEPTED
┌────────────────────────────────────────────────────────────┐
│ No. I decide I don't care.                                 │
│ No ResultCapsule · no Receipt · cannot satisfy Gate         │
├────────────────────────────────────────────────────────────┤
│ [Convert to Attention] [Reassign] [Open Attempt evidence]   │
└────────────────────────────────────────────────────────────┘
```

自然语言可以幽默、简洁或个性化；旁边的结构化状态必须诚实显示它是否构成结果、证据或决策。

## 6. 卡片类型

| 类型 | 来源对象 | 能否直接推进状态 |
|---|---|---|
| `NOTE` | 人或 Agent 的普通协作内容 | 否 |
| `QUESTION` | Question/Attention proposal | 否；等待结构化回答或 Decision |
| `TASK_PROPOSAL` | Task/Assignment proposal | 否；提交 Intent 后由 Controller 决定 |
| `RESULT_PROPOSAL` | ResultCapsule | 否；等待 Receipt/Gate/materialization |
| `EVIDENCE` | Evidence/Artifact projection | 否；必须通过 scope/digest/freshness 验证 |
| `DECISION` | 已提交 Decision | 只按对应 Gate/Policy 规则生效 |
| `HANDOFF` | Handoff Capsule | 否；连续性握手非 PASS 不得继续 |
| `CONTEXT_CHANGE` | ContextSnapshot/Prompt/Policy diff | 否；必须重新编译、投影和 LoadProof |
| `SYSTEM_STATUS` | Command/Observation/Receipt/Gate projection | 只展示权威对象已经发生的状态 |

## 7. 提及、回复和派活

### 7.1 `@Agent` 必须结构化

显示文本可以是 `@Codex`，底层必须绑定：

```text
project_id + party_version + agent_id + role_version
```

可选再绑定 `task_id / assignment_id / session_id / visibility_policy`。系统不得从正文、昵称、最后发言者或 transcript 猜收件人。

### 7.2 提及不是 Assignment

普通 `@Agent` 只产生收件人明确的 Context Card。只有用户或具备权限的 Agent 选择以下动作，才提交结构化 Intent：

- `Ask agent`
- `Create task`
- `Assign work`
- `Request audit`
- `Request correction`
- `Run to next Gate`

Controller 仍需验证 scope、revision、Policy、能力、预算、可见性和 fencing；验证不通过时卡片保留，但不创建 Assignment。

### 7.3 Reply 绑定因果而非文本引用

回复必须保存 `reply_to_card_id` 以及其权威对象引用；若对应对象已过期、跨 Project、被 supersede 或不可见，提交时 fail-closed。

## 7.4 ContextEntry 与 CollaborationProposal

Feed 卡片必须绑定一个 `ContextEntry`。卡片类型只是投影标签；权威生命周期见领域模型 `DRAFT -> SUBMITTED -> ACCEPTED | REJECTED | SUPERSEDED | EXPIRED | ARCHIVED`。

从卡片发起的 Ask / Create task / Assign / Request audit / Include in snapshot / Create Attention 必须生成 `CollaborationProposal`，带 actor、scope、expected revision、idempotency 和副作用预览。Controller 验证失败时卡片保留，不创建 Assignment/Fact/Gate。

GC owner 是 P36。被活跃 Task、Gate、Fact、Snapshot 或 Handoff 引用的 Entry 进入 HOLD。Pin for humans 只改变显示，不创建 HOLD，也不进入 Snapshot。

MVP 提供 Project 内结构化筛选（类型、Workflow/Task/Agent scope、revision、可见性）。全文检索 UI 为 post-MVP。筛选结果必须显示 index watermark/lag；Reveal Gate 前不得展示其他 Auditor 的命中数、摘要或存在性。

## 8. Reactions 的语义

`ReactionSignal` **不进入 MVP 权威对象**，P32 不得为其赋予 schema authority。表情反应用于协作信号和界面气氛，全部是 `DISPLAY_ONLY`：

- 👍 不等于 Gate PASS；
- 👎 不等于 Gate FAIL；
- 👀 不等于 Auditor 已取得可见性证明；
- ✅ 不等于 Result accepted；
- 数量不构成 quorum；
- Agent 自己的 reaction 不构成授权。

若产品以后支持结构化投票，必须使用独立 `Vote/Decision` 对象、actor authority、精确 Gate snapshot 和截止 generation；UI 可以同时投影成 reaction 样式，但不能复用普通 reaction 数据。

## 9. 从卡片进入项目上下文

卡片正文默认不会自动写入后续 Agent 的 ContextSnapshot。允许动作如下：

| 动作 | 结果 |
|---|---|
| `Pin for humans` | 只改变 Room 的显示置顶，不进入模型上下文 |
| `Propose fact` | 创建 Fact Proposal，等待来源/证据/Gate |
| `Accept into project context` | 对已验证 Proposal 提交 Promotion Intent |
| `Include in next snapshot` | 创建带来源和可见性的 Context inclusion proposal |
| `Create task` | 创建 Task Proposal，不直接派活 |
| `Request audit` | 创建 Auditor Assignment Intent |
| `Create Attention` | 生成持久 AttentionItem |
| `Exclude/supersede` | 保留历史，未来 Snapshot 不再选择；需要理由和权限 |

每个动作都先显示 preview：目标 Project、Agent、Snapshot layer、source revision、visibility、预期副作用和 idempotency identity。

## 10. Context Inspector

选中卡片后，Inspector 固定回答：

1. 这段内容来自谁、哪个 Harness/Session/Attempt？
2. 它绑定哪个 Project、Task、revision 和 ContextSnapshot？
3. 它现在是普通内容、Proposal、已验证 Evidence、Receipt、Decision，还是 Canonical Fact？
4. 哪些 Agent 可以看到？是否受 Reveal Gate 约束？
5. 它是否已经进入任何 ContextSnapshot？进入了哪些 Agent/Assignment？
6. 投影到了哪些 Harness 目标？是否有 LoadProof？
7. 是否发生 source/Policy/Role/Prompt drift？
8. 当前允许的下一安全动作是什么？
9. 本次实际使用哪个 ExecutionTarget；requested/selected/observed 是否一致，是否发生显式 fallback？

Inspector 不提供直接编辑生成 Harness 文件的入口。持久修改必须创建 Role/Policy/Fact/Context Proposal 并重新编译。

## 11. Feed 与完整 transcript 的边界

Context Room 不是完整 transcript 聚合器：

- 默认只显示被 Controller 接受的结构化卡片和显式发布的协作内容；
- 原生 Harness 的思维过程、工具细节和全部对话仍留在 Native Session Plane；
- 必要片段可脱敏提取为 Evidence/Note Proposal，并保留 native locator 和 digest；
- PTY 最后几行、stdout 中的“完成”、窗口标题和表情不能生成权威卡片状态；
- transcript unavailable 不会删除已经耐久提交的 Result/Evidence/Receipt。

## 12. Context 与 Project 隔离

- Context Room 必须始终显示 Project scope bar。
- 切换 Project 时清空 feed cursor、搜索、草稿、选中卡片和 Inspector。
- 跨 Project 提及、reply-to、Fact、Evidence、Snapshot 和 native Session ref 默认拒绝。
- 未来跨项目共享只能通过显式 Publication/Import Proposal，记录来源、许可、版本和目标可见性。
- Auditor 在 Reveal Gate 前看不到其他 Auditor 的卡片、反应、Result 或未允许材料；连数量和存在性是否可见也由 Policy 决定。

## 13. Context Health 在 Room 中的表达

Room 顶部显示 Context Health，但不简化为 token 进度条：

```text
HEALTHY | WATCH | ROTATE_RECOMMENDED | ROTATE_REQUIRED | BLOCKED | UNKNOWN
```

当发生 revision、Policy、Role、Prompt 或事实漂移时，Feed 生成 `CONTEXT_CHANGE` 卡片，显示受影响的 Agent/Assignment、旧新 digest、是否能安全 steer、是否需要重新 Snapshot 或换 Session。

Session 轮换生成 Handoff 卡片链：prepare -> old Session settle -> new Snapshot -> projection/load proof -> continuity handshake。非 PASS 时卡片停在 Attention，不能用“已切换”绿色文案遮盖。

## 14. MVP 范围

MVP 实现：

- 单 Project Context Room；
- Implementer 与一个 Auditor 的卡片；
- `NOTE/QUESTION/RESULT_PROPOSAL/EVIDENCE/DECISION/HANDOFF/SYSTEM_STATUS`；
- 结构化 mention/reply；
- display-only reactions（`ReactionSignal` 非权威，无领域 retention）；
- Task/Result/Evidence/Receipt/Gate/ContextSnapshot 绑定；
- Inspector 与 Context inclusion preview；
- Project 切换隔离负测；
- 从 Result 到 Auditor 再到 Human Gate 的卡片链。

MVP 不实现：

- 自由多人群聊替代 Workflow；
- `ReactionSignal` 权威对象或 reaction quorum；
- 完整 transcript 同步；
- 全文检索 UI；
- 跨 Project Context Room；
- 十 Auditor 实时辩论；
- Chief of Staff 自主修改项目上下文。

## 15. 验收负测

1. 自然语言 `@Codex` 没有结构化 `agent_id`：显示为普通文本，不派活。
2. Agent 回复“完成”但无 ResultCapsule/Receipt：卡片保持 `NOT_ACCEPTED`，Task 不变绿。
3. 👍/✅ reaction：Gate 和 Fact 状态完全不变。
4. 跨 Project reply/mention/context inclusion：拒绝并生成审计事件。
5. 迟到卡片引用旧 Attempt generation：保留诊断，不进入当前 Snapshot/Gate。
6. Auditor 在 Reveal Gate 前读取他人卡片或 reaction：拒绝；不得从计数泄漏结论。
7. 卡片已 pin 但未通过 Fact Promotion：不进入后续 ContextSnapshot。
8. Projection 文件已写但无 LoadProof：不得显示“Agent 已获得上下文”。
9. UI 重连、事件重复或乱序：卡片可折叠重复，但不创建第二 Task/Result/Decision。
10. Context Room 不可用：Controller、Runner 和已在途 Attempt 继续按持久状态运行，UI 恢复后补读。

## 16. 实现归属

- P12：Context Compiler 与 Snapshot/Manifest，不负责 Feed UI。
- P13：Projection/LoadProof 与 Inspector 数据来源。
- P14：Context Health、Handoff、continuity handshake。
- P18/P19：Context Room 的 versioned API、read projection、cursor 与 Intent。
- P21：Context Room、Card、Inspector、reaction display 和交互。
- P22：结构化 mention、reaction 无权威、跨 Project/Reveal Gate、重复/乱序和无 Receipt 负测。

在 P18/P19 完成前只能使用明确标记 `MOCK / NON-AUTHORITATIVE` 的静态卡片；不得让前端 mock 定义领域状态。

Context Room 中 Agent 卡片的“可运行”状态必须来自 Controller 的 derived dispatch authorization，并同时展示技术 Gate 与全局 publication/hold 两层状态。卡片、mention、reaction、Harness 原生完成文案或某个 Gate 的 `TECHNICALLY_READY` 都不能单独开启 Assignment；因此 Project Context 现场与 Command Center 共用一套权限真相，而不是各自推断。
