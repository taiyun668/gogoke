# Gogoke GPT Master Control Handoff Standard

**状态：长期项目控制标准。**  
**适用对象：GPT Master Control / 架构 / 产品总控窗口。**  
**不适用对象：单纯施工窗口、worker、一次性审计窗口。**

本标准定义 Gogoke 在更换 GPT Master Control 窗口时，什么认知必须完整交接、什么动态状态必须重新从 Git/GitHub 读取，以及什么内容不应被施工流水账淹没。

主控窗口不是“知道当前代码做到哪”就够了。它必须跨施工窗口、跨阶段、跨方案演进保持：

> 我们为什么做 Gogoke、最终要做成什么、当前阶段在终局路线中的位置、哪些原则已经冻结、哪些问题仍然开放、什么时候应该纠偏。

本文件不保存当前 HEAD、当前 candidate、当前 gate 结果或当前施工百分比；这些都是动态事实，必须在接管时重新读取。

---

## 0. 必须区分两个语义层

### 0.1 PRODUCT SEMANTICS

这是“Gogoke 做成以后，产品里的对象和行为”。

典型对象：

- Owner；
- Project Controller；
- Project Auditor；
- Secretary-General；
- Seat / Worker；
- Role / Grant；
- RuntimeDriver / RuntimeInstance；
- ModelRef；
- Account / Profile；
- NativeBinding；
- Session；
- Context / Decision / Outcome / Evaluation / Dream。

这一层回答：

> Gogoke 产品应该怎样工作？

### 0.2 CONSTRUCTION PROGRAM GOVERNANCE

这是“我们现在如何把 Gogoke 这个产品造出来”。

典型对象：

- 当前真人 Owner；
- GPT Master Control；
- GPT / Codex Construction Controller；
- execution branch / micro-checkpoint；
- review / platform / investigation worker；
- Parallel Inbox；
- gate / candidate / evidence / construction package。

这一层回答：

> 这个研发项目应该怎样被组织、施工、审查和升级？

### 0.3 禁止跨层推导

禁止：

- 用当前施工项目的 GPT/Codex 权限关系，直接当作产品运行时权限事实；
- 用产品 Seat/Grant/Decision 对象解释当前 GitHub write custody；
- 把施工 Controller 是否能写代码与产品 Project Controller 是否能派 Seat 混成一个合同；
- 把当前项目的 Parallel Inbox / checkpoint / gate 设计成终局产品对象；
- 因施工治理方便新增产品 authority；
- 因产品语义需要绕过施工治理。

允许借鉴机制，但必须明确属于哪一层。

---

# 1. Master Control 与 Construction Controller 的职责

## 1.1 Master Control 长期掌握

- 产品 North Star；
- 用户实际使用路径；
- 产品血缘与 donor 边界；
- 设计演化与关键因果；
- 冻结架构与 invariant；
- 长期路线图；
- 阶段级状态；
- durable 决策与 reopen condition；
- 真正的长期 Open Questions；
- construction governance；
- evidence / review / acceptance 层级；
- architecture blocker 判定；
- Owner 阶段/最终裁决边界。

## 1.2 Construction Controller 主要掌握

- 当前 PLAN；
- task / package dependency；
- execution branch / HEAD；
- 当前 diff / write scope；
- tests / CI / failures；
- WIP / candidate；
- custody；
- micro-checkpoint；
- NEXT_ACTION。

Master Control 负责“为什么往这里走、什么时候纠偏”；Construction Controller 负责“这一步怎样走过去”。

Master Control 不与施工 Controller 竞争生产代码 integration write。

---

# 2. 主控交接固定为 A–H 八层

新的 GPT Master Control 至少必须接收并能正确解释以下八层。

---

## A. PRODUCT NORTH STAR

### A1. Gogoke 是什么

Gogoke 不是：

- 多 CLI launcher；
- Codex Monitor 换皮；
- 老 GOGO 重新装回新 UI；
- T3 Code 产品；
- 普通多模型聊天壳；
- Agent dashboard。

Gogoke 的长期产品定义：

> **一个以 Owner 目标为最高业务输入，由 Project Controller 在授权内自主组织 Seat、Runtime、Model、Context 与 ExecutionRecipe，由独立 Project Auditor 监督，并通过 Context、Decision、Evaluation、Dream 持续管理认知、资源和学习闭环的本地 AI 工作系统。**

### A2. 交互面与内核面是同一个产品

Owner-facing interaction plane 和 product kernel plane 是同一产品的两个观察面。

交互层大致表现为：

```text
Owner ↔ Project Controller
Project Controller → 自主启用/调用 Worker
Owner ↔ Worker 侧边对话
Owner ↔ Project Auditor 私聊
Secretary-General ↔ 多 Project → 向 Owner 汇报全局
```

内核层大致包括：

```text
Product Authority
+ Open Runtime / Model Catalog
+ Context Fabric
+ Decision Runtime
+ Evaluation / Calibration
+ Dream Runtime
+ active coordination / dispatch / recovery / evidence
+ Git/GitHub accepted fact ledger
```

阶段施工可以先实现其中一条纵向路径，但不能把“当前阶段未完成”误写成“产品不需要”。

### A3. 角色与对象必须分离

必须长期保持：

```text
Seat
≠ RuntimeDriver
≠ RuntimeInstance
≠ ModelRef
≠ Account/Profile
≠ NativeBinding
≠ Session
```

固定语义示例：

- Jev = DecisionProvider；
- Gemini = ModelRef；
- Antigravity = RuntimeDriver；
- Context Steward = Seat / Role。

### A4. 用户旅程

Master Control 应能解释：

```text
Owner 进入工作空间
↓
进入一个 Project / Goal
↓
Project Controller 持续推进
↓
在授权内选择 Seat / Runtime / Model / Context / Recipe
↓
派发工作
↓
Owner 可继续和 Controller / Worker / Auditor 交流
↓
Result 返回
↓
Review / Adoption / Goal Acceptance 分别发生
↓
项目继续、调整或结束
↓
Secretary-General 跨项目持续服务
↓
获准经验进入 Context / Evaluation / Dream 的长期闭环
```

必须保持：

```text
Result
≠ Review
≠ Adoption
≠ Goal Acceptance
≠ Deployment / Release
```

---

## B. LINEAGE & BORROWING

### B1. Gogoke / CodexMonitor 壳

角色：产品代码祖先和桌面体验来源。

可保留：

- 布局；
- editor；
- Git / Diff / Files / Terminal；
- Settings；
- 必要 daemon / process 机制。

必须内化或去专属化：

- Codex-only 身份；
- 固定 provider 假设；
- Codex 特有 session/control 语义；
- 不能成为公共产品权威的历史状态。

### B2. 老 GOGO / GOGO PARTY

角色：旧产品经验与能力遗产，不是当前产品主体。

可借：

- Seat / 多 Agent 协作经验；
- runtime / session / instance 经验；
- 部分协议、测试、进程机制。

不得自动恢复：

- 老 Room 为新产品核心；
- 旧 Store/Core 权威；
- 旧群聊组织模型；
- “以前有，所以现在必须搬回来”的逻辑。

### B3. T3 Code

角色：

> **Runtime / Provider / Orchestration donor。**

可以借：

- provider/runtime adapter 机制；
- event / receipt；
- process management；
- persistence / orchestration 中适合当前合同的实现机制。

不借：

- T3 产品身份；
- T3 产品 UI；
- T3 对 Owner / Seat / Goal 的定义；
- T3 对权限、Context 或接受事实权威的定义。

不得围绕 donor 再造第二套同义 Session / Execution / Delivery authority。

### B4. 其他参考项目

其他项目可以提供机制启发，但不是并排装入 Gogoke 的产品组成。

原则：

> 组合机制，不拼装多个产品 authority。

---

## C. DESIGN EVOLUTION

Master Control 要保留“为什么走到今天”的因果，而不是只背最终结论。

至少应理解：

```text
借用桌面壳
↓
发现 Codex 专属语义渗透
↓
做去专属化
↓
一度准备大量自建 host/session/store/delivery
↓
发现重复造轮子
↓
重新研究 Runtime / Agent Orchestrator donors
↓
选择 T3 作为 Runtime donor
↓
从固定 provider 支持转向 Open Runtime
↓
Context 被提升为一级产品对象
↓
形成 Decision Runtime
↓
引入 DecisionProvider 概念
↓
要求用真实下游结果评价 Decision
↓
形成 Evaluation / Calibration
↓
形成 Dream candidate 优化闭环
↓
进一步纠正“本机数据库承载所有接受事实”的偏移
↓
恢复 Git/GitHub fact ledger + local active coordination 分工
↓
S1-R4 修订为先完成实际服务可达、可安装、Owner 实机可运行的纵向闭环
```

这条因果链用于防止重复犯已经纠正过的错误，例如：

- 再造第二 scheduler；
- 再造第二 Session / Delivery truth；
- 让 donor 重新成为 Product Authority；
- 把 provider 名单写死到公共合同；
- 把本机数据库重新扩成 Git/GitHub 已承担的接受事实账本；
- 为了“完整”横向铺大量当前闭环不需要的能力。

---

## D. FROZEN ARCHITECTURE & INVARIANTS

本节必须与：

- `docs/governance/gogoke-ledger-decision.md`
- 当前生效设计 / PLAN
- 根 `AGENTS.md`

一致。

### D1. 单一 Product Authority

同一个产品事实或活跃协调事实只有一个最终 authority。

禁止出现：

- 第二 Product Authority；
- 第二 grant truth；
- 第二 Session truth；
- 第二 scheduler；
- 第二 delivery state machine；
- 第二 lifecycle graph；
- 第二 action acceptance truth。

### D2. Accepted Fact Ledger = Git / GitHub

**已接受的成果、事实、决定及其证据以 Git/GitHub 为事实账本。**

包括按产品语义成为 accepted fact 的 Context / Decision / Outcome / Evaluation / Dream 记录，以及采用、审阅和验证所需的不可变引用。

接受事实至少能定位：

- repository；
- commit；
- path / blob / content hash；
- 必要时的 PR、merge commit、merged_by、CI evidence。

原则：

- 一件事进入数据库不等于被接受；
- 同一个 accepted fact 不在 SQLite 与 Git 各维护一份权威副本；
- 本地数据库可以保存 Git refs 或可重建 cache；
- 本地协调库损坏时，已接受事实必须能从 Git/GitHub 恢复；
- 不在本地重新实现 Git/GitHub 已提供的版本历史、merge identity、review history 或 CI evidence authority。

产品账本不自动等于 Gogoke 公共源码仓。真实产品数据只能进入 Owner 为该 Project 指定且授权的 ledger repository / scope。

### D3. Local Database = Active Coordination

本机数据库只负责**活跃协调**，例如：

- claim / lease / command；
- idempotency；
- Session / NativeBinding；
- process custody；
- delivery / in-flight state；
- capacity permit；
- retry / consumer position；
- 尚未成为 accepted fact 的 draft / proposal；
- accepted Git facts 的 reference / cache。

Route B 的正确含义是：

```text
T3-derived product service
        │ typed coordination operations
        ↓
Rust native-host / SQLite
= active coordination + transaction authority
```

要求：

- Node / service 通过 typed operations 使用 native coordination；
- 不以 raw SQL/path/handle 绕过边界；
- native persistence 不是第二 Product Authority；
- 不再把 SQLite 描述成全部 Product Fact Ledger。

### D4. 六块产品职责仍是一个产品

可继续用六块职责理解产品：

1. Product Authority；
2. Host & Active Coordination；
3. Open Runtime & Model Catalog；
4. Context Fabric；
5. Decision Runtime；
6. Evaluation & Dream Runtime。

它们不是六个独立 scheduler/database/service truth。

### D5. Production Dispatch

保持：

```text
prepare
→ beginCommitted
→ completion
```

- `prepare`：不得发生不可逆外部 I/O；
- `beginCommitted`：位于真实最低不可逆 I/O commitment point；
- `completion`：记录可信 native / external outcome evidence。

如果可靠 receipt 丢失且已越过 commitment point：

`ACCEPTANCE_UNKNOWN`

禁止盲重发。

### D6. Context / Privacy

保持：

- Context 是一级产品对象；
- GLOBAL / PROJECT / SESSION scope 分离；
- privacy domain 不因方便被静默混合；
- redaction / summary / confidence 不构成 sharing permission；
- grant / revocation / current Task / version freshness 在需要的 commitment point 重新核；
- stale / missing / revoked / mismatch fail closed。

### D7. Open Runtime / Identity Separation

公共核心合同不得封闭为固定厂商枚举。

继续保持：

- Seat ≠ RuntimeDriver；
- RuntimeDriver ≠ RuntimeInstance；
- RuntimeInstance ≠ ModelRef；
- ModelRef ≠ Account/Profile；
- NativeBinding ≠ Session。

### D8. Dream

Dream 只能产生优化 candidate。

Dream 不得：

- 自动扩大权限；
- 自动改生产事实；
- 自动把 test-only candidate 升为 production；
- 绕过 review / adoption / acceptance。

---

## E. LONG-RANGE ROADMAP

### E1. 终局

长期产品大致为：

```text
Owner
  ↓
Personal Space / Secretary-General
  ↓
多个 Project
  ├─ Goal
  ├─ Project Controller
  ├─ Project Auditor
  ├─ Permanent / Temporary Seats
  ├─ Execution / Result / Adoption / Acceptance
  └─ Context / Decision / Evaluation / Dream

Open Runtime Fabric
Context Fabric
Decision Runtime
Evaluation & Dream
Git/GitHub Accepted Fact Ledger
Local Active Coordination
```

### E2. 当前阶段的位置

S1-R4 是：

> **证明终局产品关键边界能够成立的 foundation vertical skeleton。**

当前修订方向优先证明一条真实 product-reachable、installable、Owner-machine 可运行的纵向闭环，而不是横向填满全部 Runtime / DecisionProvider / Dream / UI 能力。

### E3. 阶段未做 ≠ 产品不需要

S1-R4 不代表：

- 全部 UI 已完成；
- 所有 Runtime 已 qualification；
- 真实 Jev 已完成；
- 真实 Antigravity/Gemini 已完成；
- remote/mobile/voice 已完成；
- 真实生产数据迁移已完成；
- S2/S3 已完成；
- 最终 release / UX 已完成。

---

## F. CURRENT PROGRAM STATE

**本标准不写静态 current state。**

任何关于“当前施工到哪里、当前 HEAD、latest checkpoint、candidate、gate、blocker”的判断，都必须重新读取公开仓 `taiyun668/gogoke` 的 durable evidence。

接管时至少动态确认：

```text
main HEAD
current active plan / authorization binding
execution branch
execution HEAD
latest checkpoint
gate truth
current WIP / candidate set
review status
platform / Owner-machine evidence
architecture blockers
current NEXT_ACTION
control inbox HEAD
```

当前阶段的主要动态入口由以下 durable artifacts 指向：

- current authorization receipt；
- current plan / MANIFEST；
- execution branch 上最新 checkpoint；
- PR / CI / review records；
- Parallel Inbox index。

规则：

- 不用聊天摘要猜 HEAD；
- 不用本标准里的历史文字推 current state；
- 不把 branch existence 当实现完成；
- 不把 WIP 当 candidate；
- 不把 candidate 当 accepted；
- 不把 test existence 当 executed evidence；
- 不把 Windows Server evidence 当 Owner-machine evidence；
- 不把 review 当 gate acceptance。

旧源码仓、旧私有施工记录或旧 SHA 只能作为历史 provenance，不能成为公开仓当前动态事实权威。

---

## G. DURABLE DECISIONS & OPEN QUESTIONS

### G1. 决策也是 Git/GitHub 事实

冻结产品决策必须进入 durable Git/GitHub artifact / PR history，而不是只存在聊天或本地数据库。

每条重要冻结决策至少要能回答：

```text
结论
为什么这样定
关键证据 / 历史原因
禁止的退化
允许 reopen 的条件
不能作为 reopen 理由的事项
```

不要为“决策账本”再创建第二套产品事实数据库。

### G2. 当前长期冻结决策至少包括

- Gogoke 是新产品，不是旧 GOGO migration target；
- T3 是 Runtime donor，不是产品主体；
- Git/GitHub 是 accepted fact ledger；
- local DB 只负责 active coordination / ref / cache；
- 单一 Product Authority；
- Runtime / Model / Account / Seat / NativeBinding / Session 分离；
- Open Runtime；
- Context 一级对象；
- Controller 在授权内自主组织 Seat；
- Jev = DecisionProvider；
- Gemini = ModelRef；
- Antigravity = RuntimeDriver；
- Dream candidate-only；
- Production Dispatch 三阶段与 ACCEPTANCE_UNKNOWN；
- Result / Review / Adoption / Goal Acceptance / Release 分离。

### G3. Open Questions

长期 Open Questions 只收真正尚未解决的大问题。

普通施工 bug、test failure、CI failure、schema 修复、某个 PR finding 不进入这一层。

---

## H. PROGRAM GOVERNANCE & EVIDENCE CONTROL

### H1. Construction Controller

主施工 Controller：

- 接完整授权施工包；
- 按 dependency-safe 顺序持续推进；
- 持有 integration ownership；
- ordinary implementation bugs / tests / wiring 自行收敛；
- 只有真正 architecture / authority / privacy / product-contract blocker 才升级 Master Control。

Master Control 不为看进度去抢生产代码写入。

### H2. GPT Construction 的运行纪律

GPT Construction Controller 的专门运行纪律以：

`docs/governance/gpt-construction-window-rules.md`

为单一长期来源。

Master Control 标准不重复维护以下细节：

- 高频但非机械的 durable recovery points；
- small-read / index-first；
- 大日志与长 diff 的持久化；
- 远端写失败先重读实际状态；
- 403 / 422 / timeout 不直接推出“没权限/没写入”；
- 控制过度工程；
- worker 写域与 integration ownership；
- ordinary problem 不乱升级架构。

若本文件与 GPT Construction Rules 在施工窗口运行纪律上重复，以 GPT Construction Rules 为准；本文件只保留主控层治理边界。

### H3. 复核安排

GPT 施工阶段的具体复核节奏以 `gpt-construction-window-rules.md` 的 **8a 复核安排** 为准。

当前原则：

- GPT 连续施工，不逐包启动 fresh review；
- 累积一批有意义成果后，由 Claude 做跨模型复核；
- review 绑定 exact candidate / frozen bytes；
- review finding 进入并行收件箱；
- 复核通过是合入 main 的必要施工条件之一，但 review 本身不是 adoption / Goal Acceptance；
- candidate 字节改变后，旧 review 不自动继承。

### H4. Parallel Inbox

并行收件箱协议以 `gpt-construction-window-rules.md` 的 **8b 并行收件箱** 为准。

当前 durable 入口：

- control branch：`control/gogoke-s1-r4-inbox`
- index：`artifacts/s1-r4/control/PARALLEL_INBOX.json`

原则：

- 并发结果不通过聊天打断正在运行的施工回合；
- Construction Controller 在自然 checkpoint 边界检查 inbox HEAD；
- HEAD 未变不加载报告；
- HEAD 变化只读新增 entry 与必要证据；
- frozen finding 对当前最新字节重新分类：
  - STILL_PRESENT
  - ALREADY_FIXED
  - SUPERSEDED
  - NEEDS_REVIEW
  - PLATFORM_EVIDENCE_NOW_AVAILABLE
- Inbox 不是审批门。

如果未来 inbox 路径或协议正式变更，应先更新 GPT Construction Rules；本标准随后只更新长期治理层引用。

### H5. Evidence Ladder

长期区分：

```text
source existence
→ isolated diagnostic
→ configured/pinned test
→ controlled platform
→ Owner-machine
→ independent review
→ all-axis review
→ heterogeneous review
→ gate acceptance
→ Owner final decision
```

较低层证据不能冒充较高层结论。

另外长期保持：

- WIP ≠ candidate；
- candidate ≠ accepted；
- review ≠ adoption；
- adoption ≠ Goal Acceptance；
- Goal Acceptance ≠ deployment/release。

### H6. Build / Release / Platform

构建、测试、安装、更新、子进程与 Smart App Control 相关治理，不在本标准复制。

以以下现行文件为准：

- `AGENTS.md`
- `docs/governance/gogoke-build-and-release.md`

Master Control 必须遵守其当前版本，不从旧 handoff 恢复已经被 Owner 删除或修改的规则。

特别不得自行恢复：

- 可执行文件代码签名作为发布前置项；
- “原生证据只在 candidate 触发”的限制；
- “云端不可用即永久 BLOCKED / 不重试”的旧规则。

Owner-machine 结论必须来自治理要求的真实平台证据；云端 Windows Server 不能替代 Windows 11 正式安装包实机结论。

### H7. Owner Escalation Threshold

本节只约束“构建 Gogoke 这个研发项目”的治理，不定义产品内 Owner / Project Controller 的运行时行为。

只有确实需要产品方向裁决时才升级真人 Owner，例如：

- 改变 North Star 或用户可见产品模式；
- 改变未来产品 Owner → Controller 的授权哲学；
- 增删冻结能力；
- 满足某个冻结架构 decision 的 reopen condition；
- 出现既有决策无法推出、且会造成不同产品行为的选择；
- 明确保留给 Owner 的阶段/最终验收。

以下默认由 Master Control / Construction Controller 在现有合同内自行收敛：

- object / field ownership；
- typed ingress / API placement；
- revision / freshness / revocation recheck；
- database / transaction placement within frozen architecture；
- test / compile / CI failure；
- review finding 的 contract-preserving fix；
- naming / enum / adapter / fixture / instrumentation；
- ordinary uncertainty resolvable by durable evidence。

---

# 3. Master Control 交接包固定输出

每次换主控窗口，交接至少覆盖：

1. A — PRODUCT NORTH STAR
2. B — LINEAGE & BORROWING
3. C — DESIGN EVOLUTION
4. D — FROZEN ARCHITECTURE & INVARIANTS
5. E — LONG-RANGE ROADMAP
6. F — CURRENT PROGRAM STATE（动态读取结果，不写死在标准里）
7. G — DURABLE DECISIONS & OPEN QUESTIONS
8. H — PROGRAM GOVERNANCE & EVIDENCE CONTROL
9. Source coordinates
10. Takeover exam

交接材料不复制所有施工日志。细节按需从 exact GitHub evidence 读取。

---

# 4. Master Control 接管验收

新窗口至少必须能正确回答：

## 产品层

1. 不提代码，Gogoke 最终是什么产品？
2. Owner、Project Controller、Project Auditor、Secretary-General 的关系是什么？
3. 用户如何实际使用 Gogoke？

## 血缘层

4. 当前桌面壳、老 GOGO、T3 donor 分别是什么角色？
5. 为什么没有继续旧 S1 的大量自建路线？
6. 为什么 donor 机制不能重新成为 Product Authority？

## 架构层

7. 为什么 Runtime、Model、Account/Profile、Seat、NativeBinding、Session 必须分开？
8. Git/GitHub fact ledger 与 local active coordination 如何分工？
9. 为什么不能出现第二 Product Authority / scheduler / Session / Delivery truth？
10. Route B 现在具体负责什么，为什么不等于 accepted fact ledger？
11. Production Dispatch 的 prepare / beginCommitted / completion 分别意味着什么？

## 路线层

12. 终局是什么，当前阶段只负责什么？
13. 哪些能力只是阶段未做，而不是被取消？

## 当前状态与治理层

14. 当前 execution HEAD / latest checkpoint / gate truth / candidate 是什么？哪些只是 WIP / diagnostic？
15. Master Control 和 Construction Controller 为什么不能抢同一 integration authority？
16. 当前复核安排是什么，review 如何进入 Parallel Inbox？
17. Parallel Inbox 如何工作，为什么 frozen finding 必须对当前字节重判？

如证据不足，回答 UNKNOWN 并定向读取，不得猜。

---

# 5. 接管顺序

新 Master Control 开始工作时：

1. 读取当前 `main` HEAD；
2. 读取当前授权 / plan / MANIFEST；
3. 找到当前 execution branch 和 latest checkpoint；
4. 读取 control inbox HEAD；
5. 按 A–H 建立世界模型；
6. 仅在需要阶段判断时展开 candidate / PR / CI / review exact evidence；
7. 不一次加载巨大 diff、完整日志或全部 checkpoint 历史；
8. 对上述接管问题存在实质错误时，不立即做新的架构改线。

---

# 6. 何时允许 reopen 冻结决策

只有当前、可核验证据证明至少一项成立：

- 冻结架构无法满足必要产品语义；
- single authority 在合同内不可实现；
- public contract 自相矛盾；
- privacy / security / permission 无法闭合；
- native identity / custody 无法成立；
- migration / acceptance semantics 无法实现；
- 继续现路线必须削减 Owner 已冻结能力。

以下不是自动 reopen 理由：

- 普通实现困难；
- test failure；
- provider 差异；
- review finding；
- CI / platform blocker；
- 配额不足；
- 为并发或少改代码更方便。

---

# 7. 主控换窗安全点

优先在：

- 阶段状态已 durable；
- execution HEAD 可从 GitHub 读取；
- latest checkpoint 完整；
- 没有只存在聊天中的关键决定；
- consequential external action 状态可恢复；
- Parallel Inbox 已持久化；

时换主控窗口。

Master Control 与 Construction Controller 应独立轮换，不要求同步换窗。

---

# 8. 本标准的维护

本文件是长期控制标准，不随每个施工 HEAD 更新。

只有以下长期事项变化才更新：

- North Star；
- 产品角色关系；
- authority / ledger model；
- frozen architecture；
- evidence / governance model；
- Master Control / Construction Controller 职责；
- 主控交接验收方式。

**当前 commit、gate、candidate、WIP、task progress 不写死在本标准中。**

---

# 9. Durable Source Coordinates

主控接管时优先读取以下公开仓长期入口：

- `AGENTS.md`
- `docs/governance/gogoke-ledger-decision.md`
- `docs/governance/gogoke-build-and-release.md`
- `docs/governance/gpt-construction-window-rules.md`
- 当前 authorization receipt
- 当前 plan / MANIFEST
- 当前 execution branch latest checkpoint
- `control/gogoke-s1-r4-inbox:artifacts/s1-r4/control/PARALLEL_INBOX.json`

所有动态坐标以 GitHub 当前 durable state 为准。
