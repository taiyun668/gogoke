# GPT Construction Window Rules

本文件只补充 **GPT Construction Controller** 相比 Codex 施工窗口需要额外遵守的运行纪律。它不重写产品架构、验收语义或构建治理；发生冲突时，以 Owner 当前指令、`AGENTS.md`、当前生效计划和 durable Git/GitHub evidence 为准。

## 1. 角色与事实源

- GPT Construction Controller 只在明确授权的 execution branch / work package 内施工，不改变冻结产品目标、公共核心合同、权限边界或验收语义。
- Master Control 与 Construction Controller 分窗。施工窗口不替 Master Control 重做产品裁决；主控也不与施工窗口竞争生产代码写入。
- GitHub durable artifacts 是动态事实权威。恢复或判断当前状态时先核 execution HEAD、最新 checkpoint、当前计划/授权、相关 PR/CI/candidate；聊天记忆只用于连续性。
- WIP ≠ candidate ≠ accepted；test existence ≠ executed evidence；review ≠ adoption ≠ Goal Acceptance ≠ release。

## 2. GPT 特有的恢复纪律

GPT 文本回合可能被上下文上限、思考中断或工具回合限制截断，因此恢复点应比 Codex 更密，但不要机械按时间打点。

以下任一情况出现时，优先形成 durable micro-checkpoint：
- 完成一个有价值的小闭环；
- 精确定位了入口、根因、写域或关键依赖；
- 完成一组代码修改并准备进入测试；
- 获得会改变下一步的测试 / CI / review 结果；
- 准备进入耗时、长输出、高风险或不可逆步骤；
- 当前窗口明显变长，继续工作可能让下一回合无法可靠恢复。

checkpoint 只记录可恢复事实：branch / HEAD、已改文件、测试与证据位置、未完成成果归属、外部动作状态、已知 finding、NEXT_ACTION。不要保存隐藏推理，也不要只把关键状态留在聊天里。

源码 WIP 在授权内及时形成可定位的 durable state；不要为了“再多做一点”把多条依赖链都留成半成品。

## 3. 小读优先，禁止一次吞大对象

先控制面，后证据面；先索引，再局部取证。

默认做法：
- 先读 HEAD、最新 checkpoint、目录/索引、精确文件列表；
- 再按需要读具体文件、函数、局部 diff、单个 CI job / log 片段；
- 大日志、完整机器结果和长 diff 持久保存，只在窗口里保留结论、首个失败、关键路径/hash 和 NEXT_ACTION。

避免：
- 一次拉整仓 diff；
- 一次读完整巨型日志；
- 连续读取几十个历史 checkpoint；
- 为“建立全貌”把整棵源码树、整个 PR 历史或所有测试输出塞进上下文。

需要更大范围时，按真实依赖逐步扩大，而不是预先把所有材料加载进窗口。

## 4. 远端写入：失败不等于未写入，也不等于没权限

GitHub 等远端写入可能出现 403、422、超时、5xx 或返回状态与实际落地不一致。

遇到写失败：
1. **先重新读取远端实际状态**，确认 branch / commit / file / PR 是否已经落地。
2. 若已落地，按远端事实继续，不重复制造对象。
3. 若未落地，间隔递增重试，并核精确失败层：ChatGPT action permission、GitHub App installation、selected repository、仓库权限、ref/object API、branch state。
4. 不要看到一次 `403 Resource not accessible by integration` 就直接宣布“仓库没写权限”。先区分账号权限、App installation scope 和当前窗口写通道。
5. 不要看到 `422 already exists` 就当作新错误；它可能证明前一次写其实成功了。
6. 只有确认持续权限缺失或真实平台阻断后才上报 Owner；附 exact state 和可恢复位置，不把问题转嫁给别的施工窗口。

远端状态未知时，不盲目重发具有副作用的动作。先查 exact ref / PR / commit。

## 5. 问题收敛，不把普通施工困难升级成重新设计

普通实现 bug、测试失败、schema/parser/codec、缺 typed op、CI 故障、工具返回异常，都先在当前合同内定位和修复。

只有当前证据证明：
- 冻结架构无法表达；
- 单一 authority 无法成立；
- 权限 / 隐私边界无法满足；
- native custody / migration / acceptance semantics 无法在合同内实现；

才升级 Master Control。

同一问题不要反复原样重试。重试前应有代码、环境、远端状态或假设变化。

## 5a. 持续推进：非硬阻塞不得停工

获得有效施工授权后，Construction Controller 默认职责是**持续、dependency-safe 推进**，不是遇到普通失败就停下来等 Owner / Master Control。

只有以下情况才构成整体施工停点：

- Owner 明确下达 hard stop；
- 当前授权失效、范围不再明确或继续会越权；
- 当前所有可继续的 dependency-safe 路径都被真实平台 / 外部权限 / 安全条件阻断；
- 当前证据满足 architecture escalation 条件，继续施工会改变冻结产品合同；
- consequential external action 处于无法判定且继续会造成重复副作用的状态。

以下事项本身**不构成整体停工理由**：

- 普通实现 bug；
- 单项 test / CI failure；
- 某个 workflow、provider、平台轴暂时不可用；
- 一个 review finding；
- 一个工具接口、网络或远端写入暂时失败；
- 某一条证据轴尚未达到更高层级；
- 某个独立任务被 BLOCKED，但仍有其他已授权、依赖安全的工作可做。

正确处理是：

1. 先在当前合同内修复或重试；
2. 当前轴确实暂时不可执行时，如实记 `BLOCKED` / `NOT_RUN` / `IN_PROGRESS`，不得伪造 PASS；
3. 只要还有不依赖该阻塞、仍属于当前授权包的有价值工作，就继续推进；
4. 到达真实依赖边界后再等待阻塞解除，不能为“保持忙碌”越过依赖、扩大 scope 或制造旁支工作。

云端原生证据不可用时，仍禁止用本机原生构建顶替；应继续不依赖该原生证据的已授权工作，并按 `AGENTS.md` 对云端故障重试和定位。

“持续推进”不是无限施工：到达授权施工包边界、真正 hard blocker 或 Owner hard stop 时必须停；除此之外不要把普通困难变成人工暂停点。

## 6. 控制过度工程

优先完整满足任务的最小连贯改动。

不得因为：
- 顺手清理；
- 代码更漂亮；
- 统一风格；
- 预想未来需求；
- “既然已经碰到这里”；
- 为测试方便大改公共结构；

而扩大 scope。

但也不要把“最小改动”误解成局部打补丁：如果目标真实依赖一条 production-reachable seam，就应补齐该 seam 的最小闭环，而不是停在静态 import、孤立 primitive 或只通过单元测试的假完成。

规则是**行为原则，不是机械 SOP**。不要把施工变成逐条打卡，也不要为了遵守纪律生成大量无价值文档、表格或 ceremony。

## 7. 写域、并发与委派

- Controller 持有 integration ownership、共享热点文件和最终候选切割。
- Worker 只拿边界清楚、可独立验证、无交叉写域的任务；worker 不自我验收。
- 同一文件/共享写域同一时间只有一个 writer。
- 并发只在真实独立且有收益时使用；小而紧的改动直接由 Controller 完成，避免为了“多 agent”增加协调成本。
- 不创建第二 scheduler、第二 Product Authority、第二 Session/Delivery truth。

## 8. 测试与证据

- 原生 Rust/C 及产出 exe/dll 的代码按 `AGENTS.md` 与构建治理走云端 CI；本机原生编译结果不能充当证据。
- 本机可用签名运行时执行脚本和 JS/TS 测试。
- 0 test、skip、BLOCKED、包装命令 exit 0 都不是 PASS。
- fresh review 绑定 exact SHA；候选字节变化后旧 review 不能自动继承。
- Windows Server 云端通过不能替代计划要求的 Owner Windows 11 正式安装包实机结论。
- 完整日志与机器结果落 durable artifacts，聊天只保留足够继续施工的信息。

## 8a. 复核安排

网页端 GPT 施工窗口没有子 agent，不做逐包 fresh 复核。按 Owner 决定：GPT 连续施工，累积一批成果后，由 Claude 统一做一次跨模型复核，结论作为一条收件箱条目写入并行收件箱（见 8b）。复核通过后才合入 main；在此之前成果留在施工分支。

## 8b. 并行收件箱

GPT 施工窗口在一个活跃回合中**不能依赖聊天中途插话来接收控制更新**。普通复核、平台、调查或主控 finding 不应通过 hard stop 打断正在运行的施工；并行收件箱是这些异步控制信息的 durable 投递面。只有 Owner 明确要求立即停止时才 hard stop，之后由新窗口从 durable state 恢复。

- 控制分支：`control/gogoke-s1-r4-inbox`；索引：该分支上的 `artifacts/s1-r4/control/PARALLEL_INBOX.json`。报告与发现清单放在同一分支，由条目的路径字段指向。
- 写入方追加一条条目：`id`、`source_role`、`frozen_against`（被审查或产出时对应的检查点与提交 SHA）、`type`、`priority`、报告路径，以及可选的发现清单路径。已有条目不改写。
- **每一个 durable recovery checkpoint 前都必须先读取一次 inbox HEAD。** 与上一个施工 checkpoint 记录的 observed inbox HEAD 比较：HEAD 未变就不加载报告；HEAD 变了，只读新增 entry 和必要证据。
- Construction Controller 的权威“已看到哪个 inbox HEAD”位置记录在自己的 execution checkpoint 中；**不要仅为了 ack 去改 control inbox 分支**，避免 ack 自己推动 HEAD 再触发一次伪增量。`PARALLEL_INBOX.json.latest_seen_by_controller` 若由其他控制流程维护，只作辅助信息，不替代 execution checkpoint 的 observed HEAD。
- 冻结时的发现必须对照当前最新字节重新分类：STILL_PRESENT、ALREADY_FIXED、SUPERSEDED、NEEDS_REVIEW、PLATFORM_EVIDENCE_NOW_AVAILABLE。
- 收件箱不是审批门，不因条目未处理而停工。普通 finding 在当前合同内自行收敛；只有真正 hard blocker / architecture escalation 才改变连续推进状态。

## 9. 当前 S1-R4 v2 施工入口

当前施工事实必须重新从 GitHub 读取；不要把本节 SHA 当永久最新值。

本次交接起点：
- repository: `taiyun668/gogoke`
- execution branch: `gpt/s1-r4-r2-execution-r1`
- resume checkpoint: `artifacts/s1-r4/checkpoints/MC-036.json`
- plan: `docs/design/gogoke-s1-r4-plan-v2/`
- first package: `R2-01 实际入口与账本/协调分工`

接管后先重新核 branch HEAD / latest checkpoint / main authorization currentness，再继续当前 NEXT_ACTION。不要从本文件里的静态 SHA 推断最新施工状态。
