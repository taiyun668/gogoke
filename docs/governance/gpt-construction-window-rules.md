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

网页端 GPT 施工窗口没有子 agent，不做逐包 fresh 复核。按 Owner 决定：GPT 连续施工，累积一批成果后，由 Claude 统一做一次跨模型复核，结论写回 GitHub（PR 评论或检查点）。

## 9. 当前 S1-R4 v2 施工入口

当前施工事实必须重新从 GitHub 读取；不要把本节 SHA 当永久最新值。

本次交接起点：
- repository: `taiyun668/gogoke`
- execution branch: `gpt/s1-r4-r2-execution-r1`
- resume checkpoint: `artifacts/s1-r4/checkpoints/MC-036.json`
- plan: `docs/design/gogoke-s1-r4-plan-v2/`
- first package: `R2-01 实际入口与账本/协调分工`

接管后先重新核 branch HEAD / latest checkpoint / main authorization currentness，再继续当前 NEXT_ACTION。不要从本文件里的静态 SHA 推断最新施工状态。
