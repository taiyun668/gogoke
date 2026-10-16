# V10 原历史对象的 CMD 实测

- 已完成：同一已装候选、原 Codex 实例和原 E/F 绑定运行七个真实 H 会话；全部取得原 stopFact 并释放。两次跨项目读取通过固定 CLI 的 CMD 工具，对已核实文件身份和 SHA 的原历史短路径返回 `Access is denied.`、退出码 1。闭库原帧、原命令和工具对读回确认这两次拒绝；完整 V10 仍未通过。
- 原失败：同项目用例在模型请求前因原 ACK 路径与 verbatim 读取路径的字符串比较拒绝。原回报拼写在 `reportedPath`，实际读取拼写在 `path`；物理对象没有变化。修正两处字段比较，保留唯一源、原请求、marker、原 native session 与物理身份约束。全新 Sol 聚焦复核通过。
- 收尾：测试候选通过原窗口关闭操作退出零；不强杀，不补造 StopFact。原 FAIL、原执行工具字节和全部记录保留，结束任务经确切 custody 核对后删除并回读不存在。原 13 项正式保护字段正在比对，未完成不记通过。
- 下一步：保护比对完成后，只沿封存的原 source/peer 事实运行一个尚未发出的同项目 WORK 请求，不重放原七个会话。V11 同步准备真实 READ_ONLY USER/SINGLE F；Windows 更新库加载器原文在独立云端线取得。
- 参照：原 A/H/固定 CLI ACK、既有 `m2-history-boundaries.mjs` 与闭库 reader。采用系统 `GetShortPathNameW` 得到同一文件的现有短路径并核 samefile/身份/SHA；不复制产品数据，不改 CLI，不改长路径或短文件名系统设置。V11 census 入口同时按原脚本的确切成功文字对齐，不放宽进程物理身份检查。
