# M2 Host 忙转闲直接回读

- 已完成：原装 0.1.64 的 OpenCode 生产 H 初始化和 Grok 来源真实算术对话；V08 双席审批核心 13 项有原请求、工具回包、账本及闭库证据。这些均为技术切片结果，完整 M2/M3 未验收。
- 本次失败：`Original Host delivery to the same idle H turn did not complete; preserve without resend`。原问题已回答，原忙回合已终结并报告 idle，但没有 Host send 回执。旧输入未重发，失败不改成通过。
- 直接原因：同一原 custody 中，thread/start 的 idle ACK 之后，首次 open 的合法空线程落盘链依次产生 notLoaded、thread/unarchive 和 thread/resume。现 idle 读取器因历史 notLoaded 永久拒绝；即使只修此处，后续合法同线程 ACK 仍被当成重复 opening 拒绝。后来的真实 idle/turn completed 因此前返回而不被采用。原未归一帧没有未处理的同线程请求；原 QCard 已 ANSWERED。
- 收尾结果：原 PENDING 消息通过新 USER 请求取消；三席原 H 均取得真实 StopFact 与释放回执，秘书长真实停止后正常关闭。原 shell/native 的句柄退出码均为 0；五组正式保护、五项记忆保护和三项账本保护相等，数据库只读测量未改字节，原记录保留。两次收尾脚本在 claim/产品请求之前被测量断言挡住，原失败保留；已按原字段缺省及十进制字符串形状修正，未重发产品请求。
- 下一步：仅请求增列既有 idle 读取器的一个共享写路径；按原帧顺序折叠当前状态，保留原 custody、唯一 opening、同线程 ACK、原回合终结、C/H 单请求和 UNKNOWN 边界。授权合并并精确读回前不修改此产品文件。V06 新模板/原 USER、模型下属和接管用例继续；F 的版本切换核心已集成，真实升级仍未运行。

## 参照

本仓库 `v37_host_recipient.rs` 的当前状态顺序读取、`codex_output.rs` 的合法状态类型，以及历史 idle 处理的晚 ACK/终结集合，是本次采用的现成做法。gogo-party 的席位运行时提供原 turn terminal 与问题收尾，但没有当前原生 custody 折叠；NaveHQ、LoomOS 未找到可直接替代此判据的实现。不删除真实空线程持久化链，不增加等待、重试、第二 scheduler 或测试投递来补偿。
