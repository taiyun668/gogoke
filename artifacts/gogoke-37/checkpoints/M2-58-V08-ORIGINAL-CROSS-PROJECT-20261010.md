# V08 原跨项目拒绝读回

- 已安装 0.1.58 的新用例实际发送一次模型输入。原 A 帧向另一个项目的 gate 发起 `gate-submit`，原 H typed reply 逐字返回 `DENIED`，固定 CLI 的原工具终态 failed、原轮次 completed。没有重发输入；两 H 均取得真实停止事实并释放，两个产品窗口正常退出零。
- 原仪器失败保留。闭库 reader 修正四个前提：外层用例与嵌套 V08 用例不是同一个编号；观察用 output-stream 不是追加的变更；原 F digest 已带 sha256 前缀；gate-submit 的回执保存在原 A/H，而不写 call-permission-table 使用的 K-POLICY receipt 表。独立源码复核确认最后一项，不补产品写入或伪造数据库回执。
- 新的 immutable 读回确认原请求、唯一 typed 拒绝、政策不变、原停止及释放，数据库字节未变。原前后快照的五组正式保护、五项记忆保护和三项账本保护相等；原快照引用哈希逐项核对。原失败、补读结果和任务导出全部保留，任务删后逐项回读不存在。
- 结论仅是这一次跨项目调用被拒绝，不唯一归因内部提前返回的源码行，也不代表完整 V08 或 M2 通过。其余审批流仍明确 NOT_RUN。
- 已并行开工：Grok 在同一原实例、原已闭合席位上跑两轮与正常冷恢复；Claude transport 补迟到的原 initialize ACK 结算，共享接线由 Root 负责；秘书长首次保存的原身份读取环正在按既有同库 USER 事实修正。G 目录仍归 Claude。
- 参照：固定候选的 v37_model_tools/v37_policy、seat/policy、原 A tool 帧和 H RPC journal；沿用现成原源读回，不把另一操作使用的表当作本操作的证据。

中间仅受影响云检与构建—安装—复现。稳定全链、完整矩阵与 SAC 开启状态的实测未完成，不自行验收。
