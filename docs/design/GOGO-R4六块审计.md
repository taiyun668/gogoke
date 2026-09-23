# 审计：S1-R4 六块设计

- 被审：`taiyun668/gogo-party`，分支 `codex/r4-g1-sealing-tests`，`docs/design/gogoke-s1-r4-plan-v1/`（22 个文件）
- 审计：Claude（Opus 5），2026-09-19
- 方法：六块正文全读（ARCHITECTURE / OPEN_ADAPTER / CONTEXT / DECISION / DREAM_EVALUATION / PLAN / RUNBOOK），JSON 用脚本核（OBJECT_MODEL 16 对象、CAPABILITY_TASK_MAP 84 行、EXECUTION_PLAN 32 任务、CHECKS、VALIDATION、DECISION_FAMILIES 18 场景）
- 对照：gogoke 本体、Owner 裁决、V2／V3 承诺、我前两轮审计的遗留项

---

## 结论

**六块的工程质量是这个项目迄今最高的一份**，尤其是"不许用弱实现冒充"那一类条款，密度和精确度都超过 V2／V3。我前两轮提的问题基本闭环了 —— C→T 的映射我用脚本核过，**84 行全部有 first_checks，零缺失**。

**但它不再是"把 Codex 解耦出来"，而是换发动机。** 这件事在六块里只用一句话带过，是本次审计最重的一条。

另外，`VALIDATION.json` 自己记着 **`independent_architecture_review: false`** —— 六块从没被独立审过架构。这份就是。

---

## 一、最重的一条：旧 Rust 被一句话降级

`ARCHITECTURE.md` A1 里有这么一句：

> 旧 gogoke_daemon **仅 legacy-only**，不允许与 public 服务争用同根/同 profile。

**就这一句。** 而这一句处置掉的是：

| 模块 | 行数 |
|---|---|
| `bin/`（gogoke_daemon + daemonctl） | 4,902 |
| `shared/`（十几个 *_core） | 11,199 |
| `backend/` | 1,437 |
| `codex/` | 1,519 |
| `remote_backend/` | 515 |
| **小计（宿主/服务路径上的）** | **约 19,572** |

对比 V2 的 WP03 原话：「长期模型执行**优先复用现有 daemon 承载**」。R4 改成了：一个 T3-derived 的 **Node 服务**拥有产品数据根和 SQLite，Rust 只剩一个**新写的薄 native-host**（`apps/desktop/native-host/`，`planned_new`）负责 OS 句柄、Job、pipe 和根锁观察。

这不是坏决定 —— T3 那套 provider/event/receipt 确实比从零重建近。**但它是换发动机，不是拆线。** 三个后果没有被写进任何一块：

1. **那 19,572 行里的公共能力去哪了。** `shared/` 的 11,199 行里有 `git_ui_core`、`workspaces_core`、`worktree_core`、`prompts_core`、`files_core`、`local_usage_core`、`process_core`、`settings_core` —— V3 的 C04/C05/C23/C29/C32/C33 都建在上面。它们是随新服务重写、还是继续由 legacy 路径服务、还是搬进 Node？**六块没说。**
2. **中间期两套数据谁是权威。** A2 明写「真实迁移留 WP12」，而 WP12 **不在本阶段的六个包里**。也就是说：本阶段要建新根、新 SQLite、新迁移账本，**而旧数据的真实迁移排在阶段之外**。A2 的根锁和单 writer 只管新服务自己；"legacy-only" 的旧 daemon 仍可能持有旧根。**新库先建、旧数据后迁，这中间的权威归属需要一条明确的规则。**
3. **产品要装一个 Node 运行时。** A2 固定 Node 24.13.1 + pnpm 11.10.0。V2 §3.3 提的是"受限 worker，只做协议编解码，不选实例、不持久化队列、不决定授权"。R4 走到了**整个主服务是 TS/Node，Rust 反过来变成被调用的薄层**。方向可以，但这是 V2 那个岔路口的另一条路，**没有回到 Owner 那里重新确认过**。

---

## 二、WP04 是一堵承重墙

`EXECUTION_PLAN.json` 32 个任务，**12 个压在 WP04**；`CAPABILITY_TASK_MAP.json` 84 条能力，**23 条主责是 WP04**。

WP04 名下：

```
S1-04-D  耐久派发与接受未知        S1-04-E  事件恢复与背压
S1-04-A  授权/私域/批准和最小委托   S1-04-T  六产品投递与UI纵向回归
R4-C-ASSEMBLE  Manifest装配与权限化检索
R4-C-LINEAGE   Session lineage和暴露证据
R4-D-ENGINE    受权选择与事务预约
R4-D-BACKENDS  Jev/规则/fake/replay及Gemini配方
R4-E-SCORING   下游结果/校准/数据隔离
R4-E-DREAM     低优先级梦境回放和候选发布
R4-V-COGNITION 认知闭环/崩溃/退化实际集成
R4-V-FINAL     升级/退出与最新候选全轴交付
```

**六块里的第四块（Context Fabric）后半、第五块（Decision Runtime）整块、第六块（Evaluation & Dream）整块，加上投递、隐私执行和最终交付，全在 WP04。**

我在 V2 审计里提过"WP04 扛 12 个测试是排期风险"，V3 回的是「"工作包=一个人"不成立，Sol 实现 Luna 写测试」。那回答解决的是人手，**没有解决"一个包变成四块地基的承重墙"这件事**：

- WP04 出问题，Context、Decision、Evaluation、Dream 一起停
- 它的前置是 WP03 + WP09a，而 WP03 自己在 R4 里又是 host/进程/custody 的重灾区
- `R4-V-FINAL`（阶段最终交付）也在 WP04 —— **实现者和交付者是同一个包**

**建议**：至少把 `R4-E-SCORING`／`R4-E-DREAM`／`R4-V-FINAL` 从 WP04 剥出来。E 块与投递、Context 没有硬依赖（它消费 OutcomeRecord，不参与派发路径），完全可以独立成包；最终交付更不该由实现包自己签。

---

## 三、逐块

### 块 1　Product Authority —— 六块里唯一没有对象的一块

`PLAN.md` 第一块列的是「Project / Goal / Role / Seat / Grant / Privacy / Task / Acceptance」八样。

而 `OBJECT_MODEL.json` 的 16 个对象是：

```
RuntimeDriver  RuntimeInstance  ModelRef  RoleSpec  Seat  ExecutionRecipe
NativeBinding  ContextObject  ContextManifest  ExposureReceipt  SessionLineage
DecisionRecord  OutcomeRecord  CalibrationProfile  DreamRun  DreamProposal
```

八样里**只有 RoleSpec 和 Seat 有对象定义**。Project、Goal、Task、Grant、Acceptance、PrivacyDomain **一个都没有** —— 它们只以 `domainId`、`grantRef`、`taskId` 这样的外键字段出现在别的对象里。

后面五块（运行端、上下文、决策、评价、做梦）每一块都有自己的对象、状态机和不变量；**第一块被列为六块之首，实际是五块的外键集合。**

这里最要紧的是 **PrivacyDomain**。Owner 2026-09-17 定的那条（施工席位不得看到 Owner↔主控 对话）是整个设计里最硬的一条规矩，C 块的 §5.3 也写得最细 —— 但**隐私域本身没有对象定义**，只有散落在十来个对象上的 `domainId` 字段。一个只以外键存在的概念，没法回答"域是怎么创建的、谁能改、撤销后旧引用怎么办"。

**建议**：`Project`、`Task`、`Grant`、`PrivacyDomain` 至少要有对象定义和状态机，和另外五块一个待遇。Acceptance 要么给定义，要么从第一块的清单里拿掉 —— 因为 Owner 2026-09-16 已经砍掉了例行验收动作，这个词留在那里会让施工的人误以为要做一个验收对象。

### 块 2　Host & Persistence —— 写得最好的一块

站得住的（我逐条核过措辞，这些是真条款不是口号）：

- **`ACCEPTANCE_UNKNOWN`**：崩溃或失联于外发前后且无可靠 native 回执时保未知，只能由可信 native 幂等或无副作用查询化解，否则隔离旧绑定后显式新 operation，**不盲重发**
- **`DB commit 报错也可能未知，先读回，不重复外部动作`** —— 这条大多数人想不到
- **幂等键 `(domain, operationId)` 命中后仍核当前访问权**；同键不同内容/目标/世代判冲突，**不因正文相同误去重**
- **两阶段进程**：持久 launch intent → `native-host.prepare(ticket)` → 受管句柄/Job → 主服务持久 custody → `activate(ticket)`。Windows 用 `CREATE_SUSPENDED`，`AssignProcessToJobObject` 成功**并保存 custody 后**才 `ResumeThread`，失败不得裸跑。**票据一次消费，不能经模型参数授予。**
- **`KILL_ON_JOB_CLOSE 只是机制，不是退出证据`**
- **`Unix group 不保证防 setsid 逃离，不宣称等同 Windows`** —— 正好是我在 V2 审计里指出的 close.ts POSIX 缺口，收了
- **`不要给所有短命 probe 强加挂起`** —— 知道什么时候不该上机制
- `EOF、exit 0、Promise 完成不是任务接受证明`

两处要注意：

- **SQLite 的 `synchronous=FULL` 自陈「是配置合同，不声称硬件断电实测通过」**。诚实，但意味着 A2 的耐久承诺没有实测背书，而 A3 的整个投递语义建在它上面。至少要在 CHECKS 里明确这是 `verified_fixture` 不是 `verified_native`。
- **停止预算 grace 10s / termination 5s / observe 5s / host 总 30s**，自陈是"工程预算不是按 8s 强退"。数字本身没问题，**但没有说这些值从哪来**。R4 自己反复禁止"未知不当 0"，这里的 10/5/5/30 同样是没有依据的常数。

### 块 3　Open Runtime —— 四分法是对的，但有个 third_party 的后果没人提

站得住：

- **RuntimeDriver / RuntimeInstance / ModelRef / Seat 四分**，`Gemini 属于 ModelRef`，**不能把 Gemini 写进 driver 注册表，也不能把一个 Google 账号等同一个席位**
- **capability 三层**（declared / observed / qualified），资格键含 driver + adapter/native 摘要 + platform + profile/authRevision + model/运行模式 + isolation/toolProfile + generation
- **`注册表存在不是账号 ready`**、`模型未报告具体版本则记 unknown，不以别名假称固定`
- **合法未知 driver 保存并呈 unavailable，不能 fallback Codex** —— 这条挡住了最常见的假适配
- Pi 作为"随机陌生运行端"验开放性，而不是第六家凑数
- **`探针也可能启动 hook/登录窗口`** —— 这是我自己在老 GOGO 记过的坑（探测必须用隔离 HOME），它独立想到了

一处新发现：

**O1 说「新增 driver 通常只改 adapter 目录、组装注册/锁依赖、局部配置 UI 与自有测试」。** 但 T3 的 built-in drivers 在 donor 树里，而产品自有代码现在也写在 `third_party/t3code/apps/server/src/gogoke/`。**那么"只改 adapter 目录"这句话里的 adapter 目录，在 third_party 里还是在 gogoke 里？** 如果在 donor 树里，每加一家 CLI 就要改 `third_party/` —— 上游同步、许可边界、"哪些是我们的"三件事一起变糟。这是 third_party 位置问题的第一个具体后果，**六块里没有一处讨论过它**。

### 块 4　Context Fabric —— 设计最细，但 S1 验不了它的核心

站得住（这块的措辞密度最高）：

- **ExposureReceipt 六态**：`HOST_PREPARED / HOST_DELIVERED / NATIVE_ACKED / INHERITED / POSSIBLE / UNKNOWN`
- **`不把宿主发送的 Manifest 说成模型的完整脑内上下文`**；原生自动读文件/工具输出/插件/历史超出观测范围时**保 unknown**
- **`Jev 不能把 unknown 洗成 clean`**
- `mandatoryConstraints 强制加入且不可被模型筛掉`
- **`跨域 exclude 明细本身可能泄密，只给有权审计者看；普通调用者只获通用拒绝原因`** —— 连拒绝原因都当信道处理
- `摘要是 DERIVED_UNVERIFIED 直到逐项核对`；`hash 核对证明身份，不证明摘要语义正确`
- `跨域重建不是让模型忘记旧内容；封旧 sourceEpoch`
- `Scope 不是许可；GLOBAL 也可为 Owner-private`

**缺口**：C2 写「S1 用 SQLite FTS/元数据索引，**不要求新 embedding 服务**」。

而 DF10（context_relevance 材料相关性）和 DF12（compression_coverage 摘要必要信息覆盖）要的是**语义**判断 —— FTS 做不了。所以相关性实际落在 **Jev** 上，而 Jev 在 S1 是 fake transport。

于是就有了这个张力：**`PLAN.md` §3 把"上下文相关性"列进"最低执行闭环必须实际演示"的四项之一，而这一项在 S1 的技术条件下只能演示接口，演示不了判断。** 两处说法要对齐 —— 要么把"上下文相关性"从最低闭环里换成可验的（比如 mandatoryConstraints 强制与越权拒绝），要么明确它在 S1 只验装配链路、判断质量留 GN。

### 块 5　Decision Runtime —— 护栏够，但 18 个场景在 S1 只能验骨架

站得住：

- **Jev 是 typed 判断后端，不是 RuntimeDriver，不拥有运行事实**
- **Choice 必须含 NONE/WAIT 出口**；`Noul 没有独立 confidence 字段`
- **`Jev 超时不能取消确定性 stop`**；`停止/撤权不等待 Jev`
- **`数学/计数/日期/阈值计算在代码`**；`请求 key 名不被当指令`
- **`TypeScript as T 不是校验`** + 返回值运行时逐项校验（model、问题集合、primitive、合法候选、finite 概率/范围/和、缺失/额外结果、body/内容类型上限）
- commit 时比较 taskRevision / policyRevision / candidateHash / capabilityRevision / bindingGeneration **五种 revision**，事务内重查余额/容量/写入所有权
- **`不能精确预算靠 Jev 猜`**；`数值未知不当 0`
- **`代码保留公平和等待，不能三个任务各自 argmax 后抢同一资源`**
- **ReadGrant 与 EgressGrant 分离**；`不可把多个私域 batch 成一次请求`；**`脱敏过程本身也不得未经许可先发给另一个模型`**
- 外层 deadline 2s（交互）/10s（后台），**`默认 maxRetries=0`**，自陈"是初始工程上限，不是供应商 SLA"

**缺口**：D2 说 S1 用「可解释有界贪心：先 hard constraints、priority class、等待时间，再 **qualified semantic rank** 和估计成本」。semantic rank 来自 Jev；S1 的 Jev 是 fake。

所以 **S1 验的是贪心骨架、事务重查和拒绝路径，不是决策质量** —— 这一点 D3 自己也承认了（"真实 Jev 资格为空、external budget=0"）。**没问题，但 18 个场景不要在阶段回执里被记成"已实现"**，应当统一记 `implemented_not_verified` 或 `verified_fixture`，绝不能是 `verified`。

另外 D4 的**参考模型 `jev-1.13.0`** —— 全文没有一处说明 Jev 是谁家的模型、走哪个端点、账号从哪来。`live_jev_enabled: false` 现在挡着，但 GN 授权之前必须先回答这个，否则"真实 Jev"是一张空头支票。

### 块 6　Evaluation & Dream —— 防自欺写得最好，但 S1 交付的是一台没有燃料的机器

站得住（这块我最认可）：

- **`评分 rubric 在看最终留出结果前固定，不让 Dream 优化 reward 定义来"提高分数"`** —— 直接堵 reward hacking
- **`不能让 Jev 自己给自己当客观标签`**；`SELF_REPORT 永不成为唯一通过标签`
- 数据按 project/time/session-lineage/近重复 cluster 分组，划 development / calibration / **sealed final holdout**；**候选提出器只能看 dev 错误**；holdout 有使用预算，耗尽换新独立数据
- **`无反馈 = PENDING 或 CENSORED，不计成功也不计失败`**
- **`未选候选没有反事实结果，不能从单臂日志宣称最优`** —— 这是做过离线评估才写得出的
- `基础设施故障单列，不能全部扣到 Jev`
- `Brier 用于带真实标签的概率，ECE 只作分桶辅助`；`不要把 nativeConfidence 直接当某次正确率，也不要将多问题概率相乘称联合保证`
- Dream 是 **scheduler 的 `kind=maintenance`，不是常驻 agent**；并发 1，idle 120s，每 run ≤20 步/300s；**`控制面收到 foreground 后 2s 内停止新优化派发；已发模型请求取消/状态核对异步收尾，不谎称 2s 内所有进程已停`**
- **`Dream 使用只读快照；不能争用产品 writer、在旧 snapshot 上覆盖新事实`**；不执行任意生成代码，不改生产代码、原始证据、Grant、审查要求、测试预期或 scorer 规则
- 发布链 DRAFT→DEV_VALIDATED→CALIBRATED→HOLDOUT_VALIDATED→SHADOW→CANARY→ACTIVE，激活要 **compare-and-swap activeRevision 原子提交**，在途 decision 固定旧 policy

**要说清楚的**：E4 默认真实调用预算 0，E6 的 S1 收敛范围止于"fake shadow/回滚演示"。而 E1 的**校准键包含 `modelVersion`** —— 合成数据产出的 CalibrationProfile，按它自己的定义对真实模型无效，D3 也明写"生产不得把合成校准 profile 带入线上"。

结论不是"有问题"，而是：**S1 交付的 E 块，是一台结构完整、但没有任何有效校准数据的机器。它的价值要到 GN 之后才兑现。** 这一点应该写进阶段回执，避免"E 块已完成"被读成"评价体系可用"。

---

## 四、对照 Owner 的裁决

| 裁决 | R4 的处置 | 判 |
|---|---|---|
| 远程访问暂时关掉，封入口不删码 | WP10a `preserved_disabled`；C41 覆盖 tailscale/remote_backend/Server 设置/移动向导；T56 验全入口封死且外部 Tailscale 状态不改 | ✅ |
| 语音暂时关掉 | 同上，T57；**`迁移不重复拷贝大模型，不把缓存全删来"清理依赖"`** | ✅ 比我要求的细 |
| 施工席位不得看到 Owner↔主控 | C 块 §5.3 上下行都覆盖；T49 用随机高熵标记 + 正反对照；**`模型答不出不是通过的充分证据`** | ✅ 但隐私域无对象定义，见块 1 |
| 主控派活是 CLI→CLI，不是 sub-agent | D1 受委托选择权；`原生 subagent 不自动获得公共席位权限` | ✅ |
| 一席一实例不适用 | O2 `一个账号可供多个合格实例/席位共享已核容量`，`不能按邮箱合并身份` | ✅ |
| 首版只做代码生产 | 六块全是运行时，无通用成果页 | ✅ |
| 不要例行验收按钮/闸门 | C18／§7 `报告不自动成为验收`、`不存在新增例行验收按钮` | ⚠️ 但 PLAN 第一块列了 "Acceptance"，词面会误导，见块 1 |
| 尽量不砍功能、UI 尽量不变 | R5 的 KEEP/ADAPT/REPLACE 只裁定了一处（sealed SettingsView 的 Connect & add）；**`其它测试不能为新架构方便随意删除`** | ⚠️ 原则对，但 25 个 feature 的逐项保全清单不在本阶段 |
| 五家 | 变成六家（加 Pi 作开放性验证），全部 `native_qualified: false`，先过 fake | ✅ |

---

## 五、对照我的整合设计：产品面整块缺席

六块全是机器。**没有一块是界面。**

- 四个状态字（待处理/在运行/改动就绪/未读）—— 对象模型里没有对应字段；未读靠 T66，而 T66 主责 WP05，**不在本阶段**
- 工作树、旁聊递话、注意力、左栏 —— 全在 WP05／WP06，不在本阶段
- `S1-04-T` 叫"六产品投递与 **UI 纵向回归**"，是回归不是新界面

这不是缺陷，是分工 —— 但后果要说明白：**这六块做完，你在界面上看到的变化接近于零。** 唯一会动的是 sealed 模式下设置页的那个 Connect & add。真正的产品面从 WP05 才开始，而 WP05 不在本次授权的六个包里。

---

## 六、仪器质量：这块必须表扬

`VALIDATION.json`：**1,965 项结构检查、21 个负控制全部被拒、重复运行字节一致**，并且诚实记着 `runtime_verified: false` 和 `independent_architecture_review: false`。

`RUNBOOK` R3 更硬：

> checker 实现**先用一项真实失败反例证明没有空跑**。0 测试或 test 目标缺失为 **FAIL_INSTRUMENT**；mock 只满足其层次。

`corrections` 里还记了自查出的计划不一致（如 `R4-DOC-01` continuation 组缺失）并在建分支前改掉。

**这是"先验仪器，再验对象"的正确做法**，比 V1/V2 都强。

唯一要提的：`CHECKS.json` 的 `all_due_status` 是 `NOT_RUN`，`CAPABILITY_TASK_MAP` 84 行全是 `PLANNED_NOT_VERIFIED`。**所有数字都是计划分类，没有一条是已通过** —— 文档自己说了，但回执阶段要盯住这一点别悄悄变绿。

---

## 七、给 Codex 的七条

1. **补一节"旧 Rust 的去向"**：`shared/` 11,199 行里的公共能力（git/worktree/prompts/files/usage/process/settings）逐个判"随新服务重写／legacy 继续服务／搬进 Node"，以及旧 daemon 的退役条件。现在只有 A1 那半句。
2. **写清新根与旧根的中间期权威**：本阶段建新库，而 A2 说真实迁移留 WP12（阶段外）。两套数据并存期间谁是 writer、旧 daemon 的 legacy 路径能不能写、冲突怎么判。
3. **把 E 块和 `R4-V-FINAL` 从 WP04 剥出来。** 12/32 任务、23/84 能力压一个包，且实现者兼最终交付者。
4. **给块 1 的 Project／Task／Grant／PrivacyDomain 对象定义和状态机**，与另外五块同等待遇。PrivacyDomain 尤其不能只作外键存在 —— 它承载的是 Owner 最硬的那条裁决。
5. **"Acceptance" 要么定义要么删掉。** Owner 2026-09-16 砍过例行验收动作，这个词留在第一块清单里会误导施工。
6. **对齐"上下文相关性"的两处说法**：PLAN §3 把它列为最低闭环必须演示，而 S1 的 FTS + fake Jev 演示不了语义判断。
7. **回答 Jev 是谁**：`jev-1.13.0` 是哪家模型、什么端点、账号从哪来。GN 授权前必须有答案，否则"真实 Jev"是空头支票。

另外三项状态登记（不是本阶段要解决的）：产品代码写在 `third_party/` 内、`THIRD_PARTY_NOTICES.md` 未记 T3、seat-runtime 的 AGPL 与 MIT donor 的分发边界。**其中第一项已经产生具体后果 —— 见块 3 的 O1。**

---

## 附：核对方式

| 结论 | 怎么核的 |
|---|---|
| 84 条能力全有 first_checks | 脚本读 CAPABILITY_TASK_MAP，`first_checks` 为空者 0 |
| 44 在范围内全有任务、40 范围外全无任务 | 同上按 owning_wp 与六包集合求差，两边异常各 0 |
| WP04 扛 12/32 任务、23/84 能力 | EXECUTION_PLAN 与 CAPABILITY_TASK_MAP 计数 |
| 16 对象里无 Project/Task/Grant/Acceptance/PrivacyDomain | 列 OBJECT_MODEL 的 objects 键 |
| 旧 Rust 约 19,572 行 | `wc -l` 于 bin/shared/backend/codex/remote_backend |
| 1,965 检查 / 21 负控全拒 / 字节一致 | VALIDATION.json |
| `independent_architecture_review: false` | 同上 |
| 18 场景与八个族 | DECISION_FAMILIES.json |
| 旧 daemon 仅一句处置 | 全目录 grep `legacy|gogoke_daemon|src-tauri`，正文命中仅 ARCHITECTURE.md:7 |
