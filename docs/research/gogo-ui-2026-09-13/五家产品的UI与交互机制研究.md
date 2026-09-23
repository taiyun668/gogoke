# 五家产品的 UI 与交互机制研究

**产品范围说明：GOGO 面向多种成果生产，软件施工与通用生产两套模式并行。软件模式保留 Git、diff 和测试等既有要求，通用模式采用对应成果的审查依据；共用基础交互，允许同项目协作。完整方向见[改版研究总图](revision-map.html)。**

补充资料：[扩展工作主路径研究](work-paths.html) · [项目上下文与隔离](context-map.html) · [可点的 UI 组件图谱](components-atlas.html) · [设置与功能地图](settings-map.html) · [尺寸、组件清单与源码说明](UI设计与组件拆解.md)。

**团队协作机制优先看 AionUi，面向普通用户的主交互优先看 Cindy，效果反馈和改动审查看 Orca 与 Vibe Kanban，后台运行与状态表达看 Herdr。** 不宜选择一家整套照搬：它们让用户管理的对象不同，“完成”的含义也不同。

对 GOGO 而言，最重要的区分是：**模型负责提出问题和工作选择，界面负责忠实呈现与传回答案；运行记录、审计证据和最终验收各有自己的含义。** 降低界面负担，应减少重复概念、重复展示和重复确认，而不是掩盖真实失败或削弱验收。

## 1. 结论与证据边界

| 产品 | 用户主要在管理什么 | 最有价值的参考 | 不宜照搬的部分 |
|---|---|---|---|
| AionUi Team Mode | 一支团队及其成员会话 | Leader 入口、成员身份、团队共享资料、协作过程投影 | 默认多人并排聊天；让每个成员的权限提示都成为主要界面负担 |
| Cindy | 一项持续进行的工作 | 输入区承接模型提问、回答后的紧凑记录、统一任务身份、按需 Review | 庞大的连接、模型和配置系统；宿主额外生成的业务确认 |
| Orca | 仓库中的多个隔离工作现场 | 点页面给反馈、批量 Diff 批注、面板布局保存、运行现场直达 | 把分支、终端、运行实例作为非程序员的默认主语言；默认放宽权限的策略 |
| Herdr | 真实终端和里面的 Agent 进程 | 关闭界面不结束进程、阻塞定位、运行状态与未读状态分开 | 终端密度；把屏幕识别结果当作业务验收依据 |
| Vibe Kanban | 工作事项与执行工作区 | 计划事项和执行现场分开、对话与结果并置、批注反馈 | 为普通交流先建立一套项目管理流程；把历史云服务文档当成现行可用能力 |

以上是基于公开页面、官方演示、设计资料与固定源码快照的比较；不是安装五个产品、登录账号后完成真实模型任务的实机验收。源码证明的是具体路径及约束的存在，官方演示证明的是公开展示方式，两者都不能替代某个发行版在某台机器上的完整运行结果。

源码快照如下。文中源码链接固定到这些提交，避免移动中的主分支造成歧义。

| 产品 | 固定提交 | 提交日期 | 证据重点 |
|---|---|---|---|
| AionUi | `6744099b279b` | 2026-09-09 | Team 页面、三种视图、问题与权限组件、团队数据类型 |
| Cindy | `88f712116574` | 2026-09-13 | 任务命名、问题输入区、回答记录、模型选择器、Review 运行约束 |
| Orca | `ca2356c19412` | 2026-09-13 | 工作区文档、Native Chat、问题组件、Dashboard 实际入口、Map 代码 |
| Herdr | `2746ac7ebf75` | 2026-09-13 | 终端层级、状态来源、客户端与服务器生命周期 |
| Vibe Kanban | `4deb7eca8f38` | 2026-04-24 | 四面板、问答横条、改动审查、执行适配器；另核官方停运公告 |

**一项重要更新：** bloop 于 2026 年 4 月 10 日宣布关闭公司，Vibe Kanban 保留开源与社区维护；公告区分了继续工作的本地工作区与将移除的远程服务。它仍值得研究，但历史云端看板、评论、组织功能不能直接当作今天仍有官方托管服务的证据。[1](#s1)

## 2. AionUi：团队组织最接近，默认视觉重心未必适合 GOGO

### 用户如何开始和推进

AionUi 把普通会话与团队分别组织。Team Mode 中，Leader 理解目标，通过 Team MCP 向成员分派工作；成员在共享目录内执行，通过异步信箱交换结果，并更新共享任务记录。这是“给团队一个目标”的组织方式，不只是同时打开几个聊天框。[2](#s2)

前端以成员实例为身份。相同助手可以在同一团队出现多次，每个实例都有自己的会话。选中成员、成员颜色和消息归属保持一致，默认选中 Leader；被选成员移除后回到 Leader。这解决的是“当前在跟谁说话”，而不是替用户评判谁的结论正确。[3](#s3)

典型路径是：**进入团队 → 对 Leader 描述目标 → Leader 分工 → 按需查看成员或共享资料 → 回到 Leader 汇总。** 成员可以动态增减。添加入口也提供“告诉 Leader”的辅助路径，先准备输入内容而不擅自发送；但这类引导是 AionUi 自己的产品功能，不是 CLI 原生问题。[3](#s3)

### 三种视图表达的东西不同

| 视图 | 当前实现 | 使用价值 | 对 GOGO 的启发 |
|---|---|---|---|
| Parallel | 多成员会话横向并排，默认模式 | 同时观察团队；身份和交接可见 | 适合主动监看，不宜作为普通用户的唯一默认 |
| Single | 当前选中成员的完整会话 | 集中阅读和输入；不被其他成员挤窄 | 更适合作为主控对话的常态 |
| Board | 按成员划分消息与任务泳道，只读投影 | 回顾谁在做什么、消息如何流转 | 是过程视图，不是拖动卡片就改变业务状态的看板 |

当前实现已经有三种视图，早期团队体验文档只讨论 Parallel / Single，不能只按旧文档判断。视图偏好按团队保存，切换视图不会另造一个团队或丢失当前成员身份。[4](#s4)

Board 能分别筛选消息和任务，隐藏系统消息或已结束任务；任务中的依赖标签可以跳到阻塞项，或者显示其摘要。这里的卡片是消息和任务，列是成员，不能与“规划中、进行中、已完成”的状态列混用。[5](#s5)

### CLI 问题与权限如何出现

源码明确拆开了两条通道：结构化问题使用 `MessageQuestion`，权限请求使用 `MessageAcpPermission`。问题组件按上游问题显示单选、多选、选项说明与自由输入；整批问题准备好后一次提交，通过专门的回答接口返回，而不是发一条普通聊天消息。取消也有单独的拒绝语义。[6](#s6)

权限组件则显示上游工具请求及其选项，并把选中的原始标识传回。如果上游内容缺少可读命令，它会展示请求详情。这保留了信息，但可能把技术 JSON 暴露给普通用户；“没有丢信息”和“已经好用”是两件事。[7](#s7)

**可借鉴：** Leader 单聊作为集中入口；成员身份贯穿页签、消息与资料；对话、过程与成果是同一现场的不同视图；问题与权限分通道。

**不宜照搬：** 默认三四列聊天、每成员都显眼等待批准、同一颜色同时承担成员身份和任务状态；以及用任务 `completed` 的绿色直接表达 GOGO 的 Owner 验收。已读 UI 类型和任务组件没有证明它具有 GOGO 那套绑定仓库证据的最终完成权。[5](#s5)

## 3. Cindy：普通用户的任务连续性与问题呈现最值得细看

### 先给用户一个稳定的工作对象

Cindy 明确区分：侧栏的一项是任务，任务内人与 AI 交流的过程是对话，其中一次往来是消息；底层的 turn 不额外成为用户需要学习的一层。这样换模型、开资料或查看结果时，用户仍知道自己在做同一件工作。[8](#s8)

它的公开产品原则强调：不要求用户先理解 Key、CLI、MCP 和 Agent Loop，连接层应保留上游能力，用熟悉的工作语言表达状态。这与 GOGO 当前的目标人群非常接近；但原则本身是设计立场，仍需看具体组件如何落实。[9](#s9)

### 模型提问临时接管输入位置

Cindy 的 `AskUserQuestionPrompt` 出现在底部输入区，替换普通输入，而不是在整条对话里不断堆新的待确认框。问题多时逐题呈现；显示当前问题、选项说明和进度。单选点击后推进，多选需要显式下一步；支持返回、跳过、自定义回答和收起。[10](#s10)

这里最值得借鉴的并不是圆角或按钮颜色，而是**位置稳定**：用户一直在同一个位置表达意图。普通时候打字；CLI 真正需要答案时，在这里答题；暂时收起后仍能恢复原先选择。草稿身份同时绑定任务、请求及问题内容，避免切换任务或收到新问题后沿用旧答案。[10](#s10)

回答结束后，待回答表单不继续占据消息流。聊天里留下紧凑的“问题 / 回答”记录，保留后来理解上下文所需的信息；未答或已过期的请求不会伪装成已回答记录。[11](#s11)

```text
普通输入 → CLI 发来问题 → 同一输入区呈现选项 → 回答传回原请求
                                      ↓
                              对话留下简短问答记录
```

这是结构归纳，不是产品截图。问题来源、回答格式和过期处理仍由实际协议决定；不能仅仿制表单外观。

### Review 比最初的产品表格更接近 GOGO

Cindy 的公开 Review 方向包含：独立无开发记忆、只读、结果回到来源任务、来源改变后旧结论失效。源码也有对应实施：创建 Review 时去掉续接会话和记忆；Codex Review 不获得工作区写入根；保留的 Review 任务拒绝外部继续输入；发布结果前重新验证来源，成果另有内容指纹。[12](#s12) [13](#s13) [14](#s14)

因此，不能再简单把 Cindy 归为“只有统一助手和记忆”。它在独立 Review 与证据生命周期上已有很接近的机制。仍不能据此认定它与 GOGO 完全等价：本次没有证明其 Review 结果与 GOGO 的验收记录、对账闩锁，以及验收后停止派工的约束一致。

### 也有不应照搬的部分

Cindy 的统一模型选择器把搜索、来源、模型、引擎和强度收在同一面板，承认“已选择”和“当前生效”可能不同；操作成功才收起，失败保留上下文。交互的一致性值得学，完整配置密度未必适合 GOGO 的主输入区。[15](#s15)

它也会产生宿主自己的确认。例如公开的媒体下载权限界面是 Cindy 对下载来源作判断后生成的，不是 CLI 原生问答。其源码截图不能被拿来证明“所有选择都来自 CLI”。GOGO 若坚持 CLI 是工作问题的唯一来源，就只能参考它的布局、草稿和反馈处理，不能照搬所有出题逻辑。[16](#s16)

**最适合借鉴的组合：** 稳定的任务身份、底部问题输入、答完后的简短记录、按需打开成果与独立 Review。对 GOGO 的普通用户交互，这比照搬多列团队控制台更有价值。

## 4. Orca：把反馈直接落在工作对象上

### 核心路径是隔离工作现场

Orca 的标准路径从仓库和工作区出发：添加仓库、建 worktree、选 Agent、执行、比较改动、反馈、提交。不同工作区有各自的文件、终端和布局；页签可以承载终端、浏览器、文件或 Diff，并拖到边缘拆分面板。[17](#s17) [18](#s18)

这让多 Agent 并行的边界容易看见，但也要求用户理解仓库、分支、终端和合并。它更适合愿意管理开发现场的人，不能直接成为 GOGO 面向非程序员的默认首页。

### 两个交互尤其值得迁移

**在成果上直接提出修改。** Design Mode 让用户点击真实页面元素，把相关画面和定位信息带给 Agent，再说想改什么。用户不必知道组件名或代码路径；技术上下文由工具附带，人在结果层表达意图。[19](#s19)

**把反馈汇成一轮。** Diff 中可以逐行写评论，最后通过 Send to agent 汇成一批，再选择接收的 Agent。修改后评论仍能用来复核。它解决的是“反馈定位和一次性传递”，不要求用户复制长段日志或一条意见一个确认框。[20](#s20)

### 终端与 Native Chat 不是同一种实现

Orca 当前文档把 Chat UI 标为实验性视图：底层仍是同一个终端会话，结构化聊天是它的另一种呈现；可以回到原始 TUI。它支持模型问题卡、会话选项和上下文恢复，但文档也明确承认终端还原度仍在调整。[21](#s21)

当前问题组件使用上游问题、选项与说明，区分单选和多选；选择只高亮，显式 Send / Next 才提交。源码注释指出，一点就自动提交曾让表单消失得太快，看起来像“没发生”。这与 Cindy 的单选推进策略不同：值得根据具体问题形状测试，不宜认定只有一种按钮行为才高级。[22](#s22)

### 与画布想法相关，但需要区分“代码有”和“用户能打开”

Orca 仓库保留了 Agent Map：项目圈、工作区圈、Agent 节点、派生关系线、状态光晕，以及缩放、聚合、选中聚焦等实现。它的关系来自项目、工作区和编排谱系，不是按名字相似度猜测的知识关系。[23](#s23)

但在本次快照中，可追到的 Dashboard 根入口渲染 `AgentKanbanBoard`；没有找到从生产入口到 `AgentDashboardMapView` 的调用路径。因此这里只把 Map 列为**已存在的源码参考**，不声称当前发布界面中可达或已完成实机验证。[24](#s24)

即便采用类似图形，也要先决定节点代表目标、成员还是会话。圆圈大小在该实现中与分组内容等布局因素相关，不能直接解读为工作重要程度。

### 不借用它的权限默认值

Orca 的文档及默认参数包含跳过 CLI 审批、甚至绕开 Codex 沙箱的启动选项。worktree 提供 Git 隔离，并不等于文件系统或权限沙箱。GOGO 可以参考它的布局、反馈和工作区机制，但不应顺手带入这些权限默认值。[25](#s25)

## 5. Herdr：把真实运行现场保留下来

Herdr 的层级是 Workspace → Tab → Pane，Pane 就是真实终端，Agent 是里面被识别出的进程。界面可以鼠标选择、拖动分隔线、右键操作，也支持键盘前缀；它不会重新创造一套聊天消息样式来替代 CLI。[26](#s26)

这解释了它为何能保留原生 CLI 的很多交互：问题、选项和输入仍在终端里，宿主把键盘与鼠标输入送回进程。代价是用户也会看到终端的复杂度，不适合直接当作 GOGO 的普通用户主窗。

### 最值得参考的是状态语义

| 状态 | Herdr 的含义 | 对 GOGO 的启发 |
|---|---|---|
| Working | Agent 正在运行 | 运行活跃不能从“任务已开工”推导 |
| Blocked | 等待输入、许可或决定 | 必须能定位到实际等待的现场 |
| Done | 已结束，但该客户端尚未查看 | 是未读/注意力状态，不是业务验收 |
| Idle | 已结束或等待，且已经看过 | 不应继续用鲜艳运行色表示 |
| Unknown | 无法可靠识别 | 不编一个确定状态安慰用户 |

不同客户端各自记录是否看过完成，因此一个窗口的 Done 不一定与另一个窗口相同。这个区分对 GOGO 的“完成未看”有参考价值，也提醒我们不要把视图已读标记当成任务权威。[26](#s26)

### 运行权威在服务器，不在窗口

分离客户端与后台服务器后，关窗口或断开 SSH 不必结束 Agent；服务器和机器重启时则是恢复布局、尝试恢复支持的会话，**不是原进程一直活着**。这两种恢复必须在产品里诚实区分。[27](#s27)

它对一些 Agent 用完整生命周期 hook，对另一些（包括文档中的 Claude、Codex、Grok）用前台进程加终端底部画面规则识别状态。同一 Agent 不同时把两类来源当作并列权威。这很适合解释它的运行状态，但不应成为 GOGO 仓库验收的依据；GOGO 已有原生事件和仓库事实时，应继续使用更直接的来源。[28](#s28)

![Herdr 官方仓库中的终端布局截图](sources/herdr/assets/screenshot.png)

图示仅用于理解真实终端的视觉密度；截图内的 CLI 版本不代表本次源码快照的默认版本。[29](#s29)

## 6. Vibe Kanban：从工作事项进入执行，再把审查放到旁边

Vibe Kanban 把“要做什么”的 issue 与“在哪里执行”的 workspace 分开。一个工作事项可以关联执行现场，工作区内再有多个 Agent 会话。它的文档明确指出：同一工作区的会话共享文件，但不自动共享对话历史；“同一个文件夹”并不等于每个 Agent 知道所有上下文。[30](#s30)

### 四面板不是四份重复信息

| 位置 | 内容 | 回答的问题 |
|---|---|---|
| 最左侧 | 工作区列表 | 我现在在哪件工作里？ |
| 左主区 | Agent 对话与底部输入 | 我要怎么表达意图、追问或反馈？ |
| 右主区 | 改动、日志或预览 | 实际做出了什么？ |
| 最右侧 | Git、终端、备注 | 需要进一步操作时，现场是什么？ |

面板可调整、可关闭；具体 Diff 控件只在改动视图出现。好的部分是把交谈和成果放在一起，技术操作位于边缘，而不是让所有类型的信息争夺一条聊天流。[31](#s31)

![Vibe Kanban 官方仓库中的工作区截图](sources/vibe-kanban/packages/public/vibe-kanban-screenshot-workspace.png)

官方存图用于说明布局，不作为所有现行云功能仍可用的证据。[32](#s32)

### 输入、问题与反馈

运行时有 Queue 和 Stop，用户可以先写下一条补充，而不必等 Agent 完全停下。模型、会话与文件引用放在输入工具区；界面保留发送、排队、发送中等不同反馈。[33](#s33)

`AskUserQuestionBanner` 按上游问题逐题显示。单选推进，多选确认后推进，自由回答由输入区交给问题组件；整批答案通过回答通道提交。执行适配器分别处理 Claude 的 AskUserQuestion 与 Codex 的 requestUserInput，不能把一个普通系统提示冒充模型提问。[34](#s34)

Diff 评论与后续消息共同交给 Agent，用户能在同一个工作现场核对修改。这比“在聊天中解释哪一行，再粘贴一大段上下文”更直接。[35](#s35)

它仍是开发工作台：用户通常负责建立事项、选择工作区、审查代码并合并。GOGO 面向不写代码的人时，应借用“成果在旁边、反馈带定位”的结构，不应要求用户先学会完整 Git 操作。

## 7. 不能混为一谈的三类界面

### 工作问题、权限请求、运行状态

| 类型 | 谁提供语义 | 应如何呈现 | 不应如何呈现 |
|---|---|---|---|
| 模型工作问题 | CLI / Agent 发来的具体请求 | 原问题、原选项与说明，按协议允许回答 | 根据暂停、施工或报告状态自行补一张业务确认题 |
| 权限请求 | 上游 CLI 或已明确的宿主权限机制 | 明确操作对象、范围与真实可用决定 | 和普通偏好问题混用含糊的“确认/继续” |
| 运行状态 | 进程、协议、送达和存储事实 | 就地进度、失败说明、需要时可展开的细节 | 把每条状态变化都变成一道必须回答的题 |

“只有 CLI 发来的选择”最有价值的含义是：**产品不替模型制造工作问题，不改写答案集合，不悄悄代答。** 提交、返回、收起、关闭等只是回答容器的操作，不应再构成第二层业务审批。AionUi、Cindy、Orca 在容器形式上各有差异，不能把某一种外观误当成唯一正确的协议。[6](#s6) [10](#s10) [22](#s22)

### 三种“完成”

Agent 一轮结束、工作材料已经生成、Owner 确认目标完成，是三个不同事实。Herdr 的 Done 明确带未读含义；Orca 的 Done 是会话运行状态；AionUi 任务板的 completed 也不能自动变成 GOGO 的最终验收。[5](#s5) [26](#s26) [36](#s36)

GOGO 设计 35 的核心边界仍有独立价值：施工与审计分权，对账不等于完成，结果有证据归属，Owner 保留最终完成权。但这些边界可以留在后台和按需查看的记录中，不必变成主对话中反复出现的阶段标签或确认卡。

## 8. 与 GOGO 的具体对应

| 当前关切 | 最直接的参考 | 应保留的机制 | 不应带入的假设 |
|---|---|---|---|
| 左栏到底放什么 | Cindy 的稳定任务身份；AionUi 的 Team 入口 | 项目/工作入口与当前对话明确，减少重复层级 | 不把进程、内部任务卡、阶段都变成用户要管理的平级导航 |
| 默认只和主控聊 | AionUi Single + Leader | 成员与成果按需打开，主对话不被挤成窄列 | 有多 Agent 就必须同时展示每一列聊天 |
| CLI 发来选项如何显示 | Cindy 底部提问；AionUi 专用问答通道 | 请求身份、原文、单多选、草稿、过期与回传 | 把暂停、对账、运行结束转换成系统自己出的题 |
| 旁聊与转交 | AionUi 成员会话身份；Orca 定位反馈 | 收件人明确，原始上下文可查看，发送有回执 | 每次转交都再问是否确认，或把整段转述重复铺满主对话 |
| 不懂代码的人怎么指出问题 | Orca Design Mode | 点实际效果，技术定位自动附带，直接描述改法 | 要求用户先找文件名、组件名或写命令 |
| 对话里信息太多 | Cindy 问答记录；AionUi 内容过滤 | 默认读结果，过程与详细依据渐进展开 | 把日志、工具调用、状态、审批、报告全部用同一种大卡显示 |
| 分栏与画布如何共存 | AionUi 多视图；Orca Map 源码 | 同一批对象的不同投影，布局不改变任务状态 | 混用成员泳道、目标状态列、运行进程节点的语义 |
| 审计如何出现 | Cindy 按需独立 Review | 对象身份固定，结论过期明确，结果回到原工作 | 另开审计界面后要求用户重新交代整个任务，或把测试绿灯当最终完成 |
| 关闭页面、换窗口与恢复 | Herdr；Orca 每工作区布局 | 运行与界面解耦，恢复原现场，明确重连状态 | 关页面等于取消，或把重建会话说成原进程从未中断 |

这些是研究得到的迁移候选，不是已经批准的新施工方案，更不意味着恢复暂停中的代码改造。

## 9. 更适合 GOGO 的职责分配

从这五家可归纳出一种值得验证的结构：**默认围绕主控对话与成果；团队、过程和运行现场按需打开；真正的 CLI 问题在稳定的位置完成回答。** 其优势来自分工，而不是堆叠更多面板。

```text
项目 / 工作入口       主控对话                         成果、资料或旁聊
─────────────────   ────────────────────────────    ─────────────────────
负责定位             用户目标、模型回复、关键结果       随当前查看对象展开
不代替主控排活       CLI 真正需要时呈现问题             可拉宽、收起、固定查看
                     底部始终是表达与回答的位置

看板：主动查看工作的全局关系与进展
过程：主动追查团队实际做了什么
运行现场：诊断时查看真实 CLI / 连接 / 送达事实
```

这里只表达职责，不固定每个项目都必须“设计 → 计划 → 施工”。设计文件、计划文件、审计记录是否出现、何时审查，应由具体任务与 Agent 的真实工作决定。

三个可立即用于后续设计评估的标准：

1. **用户少学一个概念。** 一项工作不因为内部多了回合、子任务或审计进程，就增加一个必须管理的平级入口。
2. **每次交互有一个明确对象。** 选谁、问哪件事、反馈哪个成果、回答哪个 CLI 请求，都必须能确定；布局变化不暗中改变对象。
3. **减少重复，不抹掉事实。** 重复确认、重复转述与常驻技术元信息可以去掉；真实失败、问题原文、证据版本和最终验收归属不能去掉。

## 10. 尚未证明的部分

- 没有完成五个产品的安装登录和真实模型闭环，因此不对跨平台运行质量、真实送达成功率或所有 Provider 的兼容性作验收判断。
- AionUi 的 Team 协作 UI 和问答路径已检查；没有据此证明其具有与 GOGO 相同的施工/审计权限分离及最终 Owner 验收合同。
- Cindy 的独立 Review 已有实际代码约束；没有把整套实现重新做安全审计，也没有证明它的最终完成权与 GOGO 一致。
- Orca 的 Native Chat 是文档标注的实验能力；Map 虽有源码，当前生产入口可达性没有得到证明。不能把它包装成已验证的成熟画布产品。
- Herdr 的屏幕识别能服务运行监看，不是代码正确性或目标完成的证据。
- Vibe Kanban 的官方存图与部分文档包含历史云能力；当前本地功能与社区维护状态必须单独看待。

## 11. 来源与开发定位

以下源码链接均指向固定快照。公开设计文档在各自项目中的“权威”称谓只描述它们自己的治理，不构成 GOGO 的新规则。图像著作权归各原项目，图像只用于研究其界面。

<a id="s1"></a>1. bloop，Louis Knight-Webb：[Goodbye bloop](https://www.vibekanban.com/blog/shutdown)，2026-04-10。

<a id="s2"></a>2. AionUi：[Team Mode 产品说明](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/readme.md#team-mode--coordinated-multi-agent-collaboration)；[官方产品演示](https://www.aionui.com/zh-CN/)。

<a id="s3"></a>3. AionUi：[团队运行体验设计](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/docs/prds/teams/team-runtime-experience.md)；[成员页签实现](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/team/components/TeamTabs.tsx)。

<a id="s4"></a>4. AionUi：[视图状态](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/team/hooks/useTeamViewMode.ts)；[生产 Team 页面](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/team/TeamPage.tsx#L783)。

<a id="s5"></a>5. AionUi：[团队过程视图](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/team/activity/TeamActivityView.tsx)；[任务卡与依赖跳转](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/team/activity/TaskCard.tsx)。

<a id="s6"></a>6. AionUi：[MessageQuestion](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/conversation/Messages/MessageQuestion.tsx)；[专用 answerAsk 通道](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/common/adapter/ipcBridge.ts#L405)。

<a id="s7"></a>7. AionUi：[MessageAcpPermission](https://github.com/iOfficeAI/AionUi/blob/6744099b279b991c17e31c243f0920477bd31cb6/packages/desktop/src/renderer/pages/conversation/Messages/acp/MessageAcpPermission.tsx)。

<a id="s8"></a>8. Cindy：[任务、对话、消息的命名](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/docs/product-rules/task-and-conversation-naming.md)。

<a id="s9"></a>9. Cindy：[核心产品原则](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/docs/product-rules/core-product-principles.md)。

<a id="s10"></a>10. Cindy：[底部问题交互](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/renderer/components/new-chat/AskUserQuestionPrompt.tsx)；[问题身份回归测试](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/renderer/__tests__/askUserQuestionIdentity.test.tsx)。

<a id="s11"></a>11. Cindy：[回答后的问答记录](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/renderer/components/chat/AskUserQuestionBubble.tsx)；[消息流分类](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/renderer/components/chat/MessageStream.tsx#L1895)。

<a id="s12"></a>12. Cindy：[Review 产品边界](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/docs/product-rules/review-product-direction.md)。

<a id="s13"></a>13. Cindy：[Review 会话策略](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/main/reviewer/reviewSessionPolicy.ts)；[禁止后续外部输入](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/main/reviewer/reviewSessionInputPolicy.ts)；[Codex Review 写入根约束](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/packages/maker-core/src/agents/codex/index.ts#L5152)。

<a id="s14"></a>14. Cindy：[Review 发布前验证](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/main/maker-ipc/reviewStartHandler.ts#L530)；[成果指纹](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/main/reviewer/reviewArtifactFingerprint.ts)。

<a id="s15"></a>15. Cindy：[统一模型选择器](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/docs/product-rules/model-selector-unified.md)；[选择与运行时生效边界](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/docs/product-rules/session-runtime-control.md)。

<a id="s16"></a>16. Cindy：[宿主媒体下载确认截图](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/.github/pr-assets/media-download-permission-light.png)；[对应宿主权限测试](https://github.com/makecindy/cindy/blob/88f71211657420432ed956d91b2ef5a2564cd469/apps/desktop/src/main/cindy-media/__tests__/mediaDownloadApproval.test.ts)。

<a id="s17"></a>17. Orca：[首个三 Agent 工作区](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/docs/site/content/docs/first-session.mdx)。

<a id="s18"></a>18. Orca：[页签、面板与分栏](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/docs/site/content/docs/model/tabs-panes-splits.mdx)。

<a id="s19"></a>19. Orca：[Design Mode](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/docs/site/content/docs/browser/design-mode.mdx)。

<a id="s20"></a>20. Orca：[Annotate AI Diff](https://www.onorca.dev/docs/review/annotate-ai-diff)。

<a id="s21"></a>21. Orca：[Native Chat 的定位与限制](https://www.onorca.dev/docs/agents/native-chat)。

<a id="s22"></a>22. Orca：[NativeChatQuestionCard](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/components/native-chat/NativeChatQuestionCard.tsx#L88)。

<a id="s23"></a>23. Orca：[Agent Map 场景](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/components/dashboard-popout/AgentMapScene.tsx)；[分组与谱系语义](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/components/dashboard-popout/agent-map-layout.ts)；[状态光晕测试](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/components/dashboard-popout/AgentMapStatusGlow.test.tsx)。

<a id="s24"></a>24. Orca：[Dashboard 根入口](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/components/dashboard-popout/DashboardPopoutRoot.tsx)；[实际分栏视图](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/renderer/src/components/dashboard-popout/AgentKanbanBoard.tsx)。

<a id="s25"></a>25. Orca：[默认权限参数](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/src/shared/tui-agent-permissions.ts)；[文档中的启动默认值](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/docs/site/content/docs/model/agents-sessions.mdx)。

<a id="s26"></a>26. Herdr：[Concepts：层级、状态与客户端已读](https://herdr.dev/docs/concepts/)。

<a id="s27"></a>27. Herdr：[运行与恢复声明](https://github.com/herdrdev/herdr/blob/2746ac7ebf75d55289aa522d99ac66fc40f062c4/README.md)；[客户端、服务器说明](https://github.com/herdrdev/herdr/blob/2746ac7ebf75d55289aa522d99ac66fc40f062c4/docs/next/website/src/content/docs/session-state.mdx)。

<a id="s28"></a>28. Herdr：[Agents：状态权威](https://herdr.dev/docs/agents/)；[状态识别实现](https://github.com/herdrdev/herdr/blob/2746ac7ebf75d55289aa522d99ac66fc40f062c4/src/pane/agent_detection.rs)。

<a id="s29"></a>29. Herdr：[官方截图](https://github.com/herdrdev/herdr/blob/2746ac7ebf75d55289aa522d99ac66fc40f062c4/assets/screenshot.png)。

<a id="s30"></a>30. Vibe Kanban：[Session 与共享文件的边界](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/docs/workspaces/sessions.mdx)。

<a id="s31"></a>31. Vibe Kanban：[四面板界面指南](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/docs/workspaces/interface.mdx)；[可调整面板实现](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/packages/web-core/src/pages/workspaces/WorkspacesLayout.tsx)。

<a id="s32"></a>32. Vibe Kanban：[官方工作区截图](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/packages/public/vibe-kanban-screenshot-workspace.png)。

<a id="s33"></a>33. Vibe Kanban：[聊天、排队与审批界面](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/docs/workspaces/chat-interface.mdx)。

<a id="s34"></a>34. Vibe Kanban：[问题交互横条](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/packages/ui/src/components/AskUserQuestionBanner.tsx)；[Claude 请求处理](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/crates/executors/src/executors/claude/client.rs)；[Codex 问题转换](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/crates/executors/src/executors/codex/normalize_logs.rs)。

<a id="s35"></a>35. Vibe Kanban：[改动与反馈](https://github.com/BloopAI/vibe-kanban/blob/4deb7eca8f381f7cbc1f9d15515a9ab8f8009053/docs/workspaces/changes.mdx)。

<a id="s36"></a>36. Orca：[Agent 状态、Dashboard 与会话生命周期](https://github.com/stablyai/orca/blob/ca2356c194122090982426128fbc4e33e407b677/docs/site/content/docs/model/agents-sessions.mdx)。

GOGO 定位基准：项目内 `docs/design/35-construction-os.md`，结合最新明确的“模型 CLI 提供工作问题和选项”边界；设计 35 中描述早期运行时的行号与行为，不作为当前工作区的重新验收结论。
