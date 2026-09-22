# Gogoke S1-R4：开放运行端、上下文与自主决策底座

状态：DESIGN_FIXED_WITH_RUNTIME_GATES。这是施工前设计，不是产品实现或独立架构验收。GPT 编制和发布；Owner 将固定启动词交给现有 Codex 主控后执行。GPT 不控制该 Codex 会话。

## 1. 固定输入与效力

仓库 taiyun668/gogo-party；本方案父提交 3b78e65361d0b75814c91d5a566d78a5e7fa013b；已推送施工参考 88ef8e7dfbf5ba5aef58743dc45fa660f946276e；生产审计追溯 bc665a852833952b76d9508401193bedd2198436。旧 S1 原计划 34b7f891e4e17715b08491570740516d6cf7f49f 的任务、检查截止保留。P00 已有证据关闭/施工前认可不重新审判，不等于新实现通过。

采用 T3 Code donor d6f291303ddc0c9a14f570266a4d9eff6d431593，tree 90d115e4249a741d352016718c1a0526d4e5b2b7。Pi 协议核对基线 d1230ea2000d876b479a69b8b061f9d670f262f5。Jev 接口参考 jev-1.13.0 和 JS SDK v0.6.0@66880ccded6cb642dc1809620c2b108c33730214；Gemini 是 ModelRef，不是 RuntimeDriver，使用已准入的 Antigravity 实例。型号、账号容量与真实支持必须实际发现，本文不指定一个未经验证可用的 Gemini 型号。

本方案替代旧 R3 和旧 AionCore 包的施工指令，不覆写原文件或旧分支。旧 19 任务已经逐项改线进 EXECUTION_PLAN.json；不是要求 Codex 再制定映射。新增 13 任务，总计 32 张。原 68 主测试、157 检查截止、33 S1 到期检查和84能力行为保全；新增检查是独立编号，不改旧截止。旧文件的未执行状态是历史记录，不可当成本轮状态。

## 2. 六块边界，只有一个产品运行真相

1. Product Authority：Project/Goal/Role/Seat/Grant/Privacy/Task/Acceptance。
2. Host & Persistence：一个 T3-derived 服务、同库事务、事件、收据、受管进程、私有IPC。
3. Open Runtime & Model Catalog：N个运行端、多个实例、模型/档位/能力/资格；Pi和随机未知驱动验开放。
4. Context Fabric：GLOBAL/PROJECT/SESSION 作用域、来源、版本、Manifest、exposure/lineage和可撤销引用。
5. Decision Runtime：受权候选、规则/fake/replay/Jev接口、事务重查、配方与资源预约。
6. Evaluation & Dream：下游结果、可靠性校准、空闲回放、受控优化候选与分级采用。

这六块是一个主服务的职责分区，不是六个独立服务，不是多个调度器。T3的Provider/Event/Receipt机制被实质复用；不复制一整套同义Session/Execution/Delivery FSM。薄Rust层只拥有OS句柄和观察，不另有产品数据库。原生CLI仍拥有原生历史/工具循环，Gogoke持有产品绑定与观察。

## 3. 阶段目标与明确非目标

交付现有六产品 P10a/P01/P02/P03/P09a/P04，并在其内部补齐上下文、决策和梦境最小闭环，不另外承诺完成S2/S3。

必须实际演示：受控目标→项目上下文→职责/席位选择→运行配方→不可变Manifest→fake受管运行端→持久结果→下游评分→隔离数据回放→梦境候选；同时验证权限拒绝、断线未知、重启恢复、前台抢占和回滚。

首批资格目标是Codex/Claude Code/Grok/OpenCode/Antigravity/Pi，不是架构封闭枚举。现有5家协议/SDK路径和Pi各有fake注入验证，真实原生账号、能力与产品全矩阵仍在原后续门。测试通过fake路径不意味着原生6家全功能通过。

本阶段实现18个场景的定义/归属/反馈接口，不要求18种自动策略全部在生产启用。最低执行闭环覆盖执行配方、Session选择、上下文选择、项目记忆候选；跨项目提升、元策略优化和所有真实外部模型默认仅候选/关闭。真实Jev及Antigravity/Gemini账号调用、外发、费用、用户数据、远程、语音、安装更新、签名发布、合并均不在当前许可。

## 4. 连续施工顺序

I：G0来源/依赖/实际接包与受控构建；G1保留式封存、公共对象、根锁/SQLite与开放SPI。
II：G2统一宿主/SDK进程通路、Pi与随机运行端、能力/身份、真实受控Windows。
III：G3 Context/Decision/原安全投递；G4下游评价/梦境与纵向回归。
IV：G5最新候选独立全轴复核、修复回归、回滚/升级和阶段交付。

阶段内部可以并行无冲突任务。工作依赖见EXECUTION_PLAN，产品采用还须满足原WP依赖及该产品到期检查；不能把所有旧检查拖到最后G5。普通编译/实现/测试错误留Codex；缺平台记阻断并继续无依赖工作；改变权限/权威/内核/产品保证才回GPT。不因每张任务卡、每个PR或恢复点切平台。

## 5. 开工与完成

本次只发布设计；CODEX_START中由Owner发送的明确授权才启动限定施工。旧PR正文的开门字段不能覆盖后来的停工和新计划。旧分支、PR#6/#7不移动、不合并。实际dirty/untracked/本地未推送状态未知，接包必须固定，不reset/clean。

S1-R4完成要求原33到期检查及全部R4检查有同候选证据、无必需FAIL/BLOCKED/SKIP、独立review合格、未执行真实模型/原生/发布轴诚实列明。BLOCKED可以交停点回执，不能叫阶段完成。没有测试目标/0测试/只运行参考模型都不能开门。所有下游资格与Owner最终产品验收仍保留。
